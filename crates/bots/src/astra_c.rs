//! Astra C: consensus-calibrated value bands with inventory and settlement controls.
//! Samples every five ticks and refreshes quantiles every twenty, while quoting
//! every decision. Before taking liquidity, removes conflicting opposite quotes.
//! All estimates use only this account's observations and the published rules.
//! Band sampling and lot-wise pair valuation build on the public Coral implementation;
//! consensus calibration, inventory skew, value guards and settlement convergence are new.

use crate::common::*;
use crate::strategic::{credit_topup, requote, Want};
use bot_api::*;
use std::collections::VecDeque;

#[derive(Default)]
struct Band {
    samples: VecDeque<(Tick, f64)>,
    next_sample: Tick,
    next_refresh: Tick,
    lo: f64,
    hi: f64,
}

impl Band {
    fn update(&mut self, tick: Tick, ratio: f64, window: Tick, quantile: f64, margin: f64) {
        if tick >= self.next_sample {
            self.samples.push_back((tick, ratio));
            self.next_sample = tick + 5;
        }
        while self
            .samples
            .front()
            .is_some_and(|&(t, _)| t + window < tick)
        {
            self.samples.pop_front();
        }
        if tick >= self.next_refresh {
            self.next_refresh = tick + 20;
            (self.lo, self.hi) = if self.samples.len() >= (window / 10).max(2) as usize {
                let mut xs: Vec<_> = self.samples.iter().map(|x| x.1).collect();
                xs.sort_unstable_by(f64::total_cmp);
                let at = |q: f64| xs[((xs.len() - 1) as f64 * q) as usize];
                let lo = at(quantile);
                (lo, at(1.0 - quantile).max(lo + 2.0 * margin))
            } else {
                (1.0 - margin, 1.0 + margin)
            };
        }
    }
}

pub struct AstraC {
    rules: Rules,
    know: Knowledge,
    bands: Vec<Band>,
    window: Tick,
    quantile: f64,
    margin: f64,
    edge: f64,
    leverage: f64,
    order_frac: f64,
    combo: f64,
    cap: f64,
    tolerance: f64,
    consensus: f64,
    skew: f64,
    settlement: f64,
    guard: f64,
}

impl AstraC {
    pub fn new(s: &Start) -> Self {
        Self {
            rules: s.rules.clone(),
            know: Knowledge::default(),
            bands: (0..s.rules.stocks.len()).map(|_| Band::default()).collect(),
            window: s.param("window", 300.0).max(20.0) as Tick,
            quantile: s.param("quantile", 0.35).clamp(0.0, 0.49),
            margin: s.param("margin", 0.01),
            edge: s.param("edge", 0.02),
            leverage: s.param("leverage", 1.8).clamp(1.0, s.rules.max_leverage),
            order_frac: s.param("order_frac", 0.06),
            combo: s.param("combo", 0.2).clamp(0.0, 1.0),
            cap: s.param("cap", 0.3),
            tolerance: s.param("tolerance", 0.0),
            consensus: s.param("consensus", 0.8).clamp(0.0, 1.0),
            skew: s.param("skew", 0.03),
            settlement: s.param("settlement", 300.0),
            guard: s.param("guard", 0.03).max(0.0),
        }
    }

    /// Average marginal pair value over the whole proposed lot, rather than valuing
    /// every share as if it alone completed the next pair.
    fn pair_value(
        &self,
        obs: &Observation,
        terminal: f64,
        stock: usize,
        qty: Qty,
        buy: bool,
    ) -> f64 {
        if qty <= 0 || self.combo == 0.0 {
            return 0.0;
        }
        let mut after = obs.account.shares.clone();
        after[stock] += if buy { qty } else { -qty };
        let mut v = 0.0;
        for c in &self.rules.combos {
            if (c.a != stock && c.b != stock) || c.end_tick < obs.tick {
                continue;
            }
            let from = obs.tick.max(c.start_tick.saturating_sub(1));
            let periods = payments_after(&self.rules, from, c.end_tick) as f64
                + if c.active(self.rules.ticks_total) {
                    terminal
                } else {
                    0.0
                };
            let before = c.sets(&obs.account.shares);
            let delta = if buy {
                c.sets(&after) - before
            } else {
                before - c.sets(&after)
            };
            v += delta as f64 * c.bonus as f64 * periods / qty as f64;
        }
        v * self.combo
    }
}

impl Bot for AstraC {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
        self.know.update(obs);
        let r = &self.rules;
        let pending: Vec<_> = self.know.pending().collect();
        let low: Vec<_> = self.know.pending_with(Estimate::Low).collect();
        let high: Vec<_> = self.know.pending_with(Estimate::High).collect();
        let t_lo = self.know.terminal(r, Estimate::Low);
        let private_mid = self.know.terminal(r, Estimate::Mid);
        // The cross-section supplies a noisy public estimate of the common terminal
        // multiple. Robust aggregation avoids chasing any single crowded stock.
        let mut implied: Vec<f64> = obs
            .books
            .iter()
            .enumerate()
            .filter_map(|(s, b)| {
                if b.dividend <= 0 {
                    return None;
                }
                let cashflows = hold_value(r, obs.tick, s, b.dividend, &pending, 0.0);
                // Use the observed dividend scale: projecting noisy private changes
                // into both numerator and denominator amplifies estimation errors.
                Some((b.mid().unwrap_or(b.mark) as f64 - cashflows) / b.dividend as f64)
            })
            .collect();
        implied.sort_unstable_by(f64::total_cmp);
        let public_mid = implied
            .get(implied.len() / 2)
            .copied()
            .unwrap_or(private_mid);
        let public_mid = public_mid.clamp(t_lo, self.know.terminal(r, Estimate::High));
        let t_mid = private_mid + self.consensus * (public_mid - private_mid);
        let t_hi = self.know.terminal(r, Estimate::High);
        let eq = obs.account.equity.max(0) as f64;
        let n = obs.books.len();
        let mut want = vec![Want::default(); n];
        let mut values = vec![0.0; n];
        for (s, b) in obs.books.iter().enumerate() {
            let v = hold_value(r, obs.tick, s, b.dividend, &pending, t_mid).max(1.0);
            values[s] = v;
            let price = b.mid().unwrap_or(b.mark).max(1) as f64;
            let have = obs.account.shares[s];
            let lot = ((eq * self.order_frac / price) as Qty).max(1);
            let band = &mut self.bands[s];
            band.update(obs.tick, price / v, self.window, self.quantile, self.margin);
            let convergence = if self.settlement > 0.0 {
                (1.0 - r.ticks_total.saturating_sub(obs.tick) as f64 / self.settlement)
                    .clamp(0.0, 1.0)
            } else {
                0.0
            };
            // Long-horizon resale premiums disappear at settlement. Inventory tilts
            // both quotes continuously instead of waiting for a hard position cap.
            let inventory = have as f64 * price / eq.max(1.0);
            let tilt = self.skew * (inventory - 0.12);
            let lo = band.lo * (1.0 - convergence) + (1.0 - self.margin) * convergence - tilt;
            let hi = band.hi * (1.0 - convergence) + (1.0 + self.margin) * convergence - tilt;
            let add = self.pair_value(obs, t_mid, s, lot, true);
            let remove = self.pair_value(obs, t_mid, s, lot.min(have), false);
            let mut bid = round_tick(r, v * lo.min(1.0 + self.guard) + add);
            let mut ask = round_tick(r, v * hi.max(1.0 - self.guard) + remove);
            if let Some(bb) = b.best_bid() {
                bid = bid.min(bb + r.tick_size);
            }
            if let Some(ba) = b.best_ask() {
                ask = ask.max(ba - r.tick_size);
            }
            bid = bid.min(ask - r.tick_size).max(r.tick_size);
            ask = ask.max(bid + r.tick_size);
            let room = (((eq * self.cap / price) as Qty) - have).max(0);
            let qb = lot
                .min(room)
                .min(buy_room(&obs.account, r, bid, self.leverage) / n.max(1) as Qty);
            want[s].bid = (qb > 0).then_some((bid, qb));
            want[s].ask = (have > 0).then_some((ask, lot.min(have)));
            let v_lo = hold_value(r, obs.tick, s, b.dividend, &low, t_lo) + add;
            let v_hi = hold_value(r, obs.tick, s, b.dividend, &high, t_hi) + remove;
            if let Some(a) = b
                .best_ask()
                .filter(|&a| (a as f64) < v_lo * (1.0 - self.edge))
            {
                let q = (lot * 2)
                    .min(room)
                    .min(buy_room(&obs.account, r, a, self.leverage));
                want[s].bid = None;
                if want[s].ask.is_some_and(|(p, _)| p <= a) {
                    want[s].ask = None;
                }
                want[s].take = Some((Side::Buy, a, q));
            } else if let Some(bp) = b
                .best_bid()
                .filter(|&p| (p as f64) > v_hi * (1.0 + self.edge) && have > 0)
            {
                want[s].ask = None;
                if want[s].bid.is_some_and(|(p, _)| p >= bp) {
                    want[s].bid = None;
                }
                want[s].take = Some((Side::Sell, bp, (lot * 2).min(obs.account.sellable[s])));
            }
        }
        if let Some((s, side, p, q)) = credit_topup(obs, r, &values, 0.012) {
            match side {
                Side::Buy => want[s].ask = None,
                Side::Sell => want[s].bid = None,
            }
            want[s].take = Some((side, p, q));
        }
        requote(obs, r, &want, self.tolerance)
    }
}
