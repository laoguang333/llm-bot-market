//! Almanac: trades the market's calendar and the slow digestion of news.
//!
//! Three observations about this market, measured on recorded games (seeds outside
//! the held-out set), drive it:
//!
//! 1. Prices take about a hundred ticks to digest a dividend change. Regressing the
//!    log price move on the news-implied value change, roughly 20% is in the price when
//!    the change becomes public, 50% about a hundred ticks later; little moves after.
//!    `CURVE` is that average response by ticks since the first private disclosure.
//!    Every known event contributes the part of its response still to come.
//! 2. The market drifts by game phase. The first tenth rises in almost every world, the
//!    middle is roughly flat after financing costs, and the final stretch falls, because
//!    late prices stay above what the frozen dividends will pay out at the end.
//!    The base book follows that calendar: levered early, unlevered in the middle, and
//!    in the end game it only keeps stocks priced below their terminal estimate.
//! 3. The idle fee costs 0.6% of equity per period unless traded volume offsets it, so
//!    before each settlement any shortfall is topped up with rebalancing trades first
//!    and a same-tick round trip in the tightest held stock last.
//!
//! Inputs are this account's own news, terminal hints, the public books and the rules.

use bot_api::*;
use std::collections::HashMap;

/// Average share of a news-implied value change already in the price, by ticks after
/// the first private disclosure (public at +60, effective at +120).
const CURVE: &[(f64, f64)] = &[
    (0.0, 0.01),
    (10.0, 0.03),
    (20.0, 0.04),
    (30.0, 0.07),
    (40.0, 0.11),
    (50.0, 0.14),
    (60.0, 0.20),
    (62.0, 0.26),
    (65.0, 0.31),
    (70.0, 0.37),
    (80.0, 0.43),
    (90.0, 0.47),
    (100.0, 0.48),
    (120.0, 0.50),
    (150.0, 0.51),
    (200.0, 0.52),
];

fn absorbed(tau: f64) -> f64 {
    if tau <= CURVE[0].0 {
        return CURVE[0].1;
    }
    for w in CURVE.windows(2) {
        let (a, b) = (w[0], w[1]);
        if tau <= b.0 {
            return a.1 + (b.1 - a.1) * (tau - a.0) / (b.0 - a.0);
        }
    }
    CURVE[CURVE.len() - 1].1
}

#[derive(Clone, Copy, Debug)]
struct Event {
    /// Expected change of the dividend rate, rate units.
    change: f64,
    first_reveal: Tick,
    effective: Tick,
}

pub struct Almanac {
    rules: Rules,
    n: usize,
    events: HashMap<(u32, usize), Event>,
    /// Intersection of received terminal hints.
    n_lo: i64,
    n_hi: i64,
    // parameters
    early_lev: f64,
    early_end: f64,
    mid_lev: f64,
    late_start: f64,
    late_lev: f64,
    ramp: f64,
    gain: f64,
    drift: f64,
    cap: f64,
    max_lev: f64,
    band: f64,
    clip: f64,
    slip: f64,
    end_margin: f64,
    top_up_ticks: Tick,
    wash_spread: f64,
    wash_depth: f64,
}

impl Almanac {
    pub fn new(start: &Start) -> Self {
        let r = &start.rules;
        Self {
            rules: r.clone(),
            n: r.stocks.len(),
            events: HashMap::new(),
            n_lo: r.terminal_payments_min,
            n_hi: r.terminal_payments_max,
            early_lev: start.param("early_lev", 2.4),
            early_end: start.param("early_end", 0.11),
            mid_lev: start.param("mid_lev", 1.0),
            late_start: start.param("late_start", 0.88),
            late_lev: start.param("late_lev", 0.0),
            ramp: start.param("ramp", 0.03),
            gain: start.param("gain", 4.0),
            drift: start.param("drift", 1.0),
            cap: start.param("cap", 0.6),
            max_lev: start.param("max_lev", 3.0),
            band: start.param("band", 0.015),
            clip: start.param("clip", 0.1),
            slip: start.param("slip", 0.3),
            end_margin: start.param("end_margin", 0.03),
            top_up_ticks: start.param("top_up_ticks", 10.0) as Tick,
            wash_spread: start.param("wash_spread", 0.012),
            wash_depth: start.param("wash_depth", 0.003),
        }
    }

    fn n_est(&self) -> f64 {
        (self.n_lo + self.n_hi) as f64 / 2.0
    }

    /// Base book gross exposure by game phase, as a multiple of equity.
    fn calendar(&self, tick: Tick) -> f64 {
        let f = tick as f64 / self.rules.ticks_total as f64;
        let ramp = self.ramp.max(1e-6);
        if f < self.early_end {
            self.early_lev
        } else if f < self.early_end + ramp {
            let a = (f - self.early_end) / ramp;
            self.early_lev + (self.mid_lev - self.early_lev) * a
        } else if f < self.late_start {
            self.mid_lev
        } else if f < self.late_start + ramp {
            let a = (f - self.late_start) / ramp;
            self.mid_lev + (self.late_lev - self.mid_lev) * a
        } else {
            self.late_lev
        }
    }

    fn update_info(&mut self, obs: &Observation) {
        for &(lo, hi) in &obs.terminal_hints {
            self.n_lo = self.n_lo.max(lo);
            self.n_hi = self.n_hi.min(hi);
        }
        if self.n_lo > self.n_hi {
            // Inconsistent hints cannot happen by the rules; fall back to the latest one.
            if let Some(&(lo, hi)) = obs.terminal_hints.last() {
                self.n_lo = lo;
                self.n_hi = hi;
            }
        }
        for item in &obs.news {
            for c in &item.changes {
                if c.stock >= self.n {
                    continue;
                }
                self.events.insert(
                    (item.id, c.stock),
                    Event {
                        change: c.mid(),
                        first_reveal: item.first_reveal_tick,
                        effective: item.effective_tick,
                    },
                );
            }
        }
        let t = obs.tick;
        self.events.retain(|_, e| t <= e.effective + 200);
    }

    /// Ordinary payments still to come after `tick`, including the terminal estimate.
    fn payments_left(&self, tick: Tick) -> f64 {
        let last = self.rules.last_tick();
        let in_game = last.saturating_sub(tick) as f64;
        in_game + self.rules.terminal_dividend_payments(self.n_est())
    }

    fn scale(&self) -> f64 {
        self.rules.dividend_scale as f64
    }

    /// Expected log move still to come per stock, and the dividend rate after known changes.
    fn signals(&self, obs: &Observation, prices: &[f64]) -> (Vec<f64>, Vec<f64>) {
        let t = obs.tick;
        let mut alpha = vec![0.0; self.n];
        let mut rate: Vec<f64> = obs.books.iter().map(|b| b.dividend as f64).collect();
        let last = self.rules.last_tick();
        let term = self.rules.terminal_dividend_payments(self.n_est());
        for (&(_, s), e) in &self.events {
            if e.effective > t {
                rate[s] += e.change;
            }
            if prices[s] <= 0.0 || e.effective > last {
                continue;
            }
            let after = (last - e.effective) as f64 + term;
            let x = e.change / self.scale() * after / prices[s];
            let tau = t.saturating_sub(e.first_reveal) as f64;
            let rest = (absorbed(f64::INFINITY) - absorbed(tau)).max(0.0);
            alpha[s] += self.drift * x * rest;
        }
        for r in rate.iter_mut() {
            *r = r.max(0.0);
        }
        (alpha, rate)
    }

    fn targets(&self, obs: &Observation, prices: &[f64]) -> (Vec<f64>, Vec<f64>) {
        let t = obs.tick;
        let (alpha, rate) = self.signals(obs, prices);
        let base = self.calendar(t) / self.n as f64;
        let late = t as f64 / self.rules.ticks_total as f64 >= self.late_start;
        let left = self.payments_left(t);
        let mut w = vec![0.0; self.n];
        for s in 0..self.n {
            if prices[s] <= 0.0 {
                continue;
            }
            let fair = rate[s] / self.scale() * left;
            let mut b = base;
            if late && prices[s] < fair * (1.0 - self.end_margin) {
                // Cheap against what it will pay out: keep a full mid-game weight.
                b = b.max(self.mid_lev / self.n as f64);
            }
            if late && prices[s] > fair * (1.0 + self.end_margin) {
                b = 0.0;
            }
            w[s] = (b + self.gain * alpha[s]).clamp(0.0, self.cap);
        }
        let gross: f64 = w.iter().sum();
        if gross > self.max_lev {
            let k = self.max_lev / gross;
            for x in w.iter_mut() {
                *x *= k;
            }
        }
        (w, alpha)
    }
}

fn mid(b: &BookView) -> Option<f64> {
    match (b.best_bid(), b.best_ask()) {
        (Some(bid), Some(ask)) => Some((bid + ask) as f64 / 2.0),
        _ => None,
    }
}

/// Shares available up to `limit` on the opposite side (asks for a buy).
fn depth_to(levels: &[Level], limit: Price, buy: bool) -> Qty {
    levels
        .iter()
        .take_while(|l| {
            if buy {
                l.price <= limit
            } else {
                l.price >= limit
            }
        })
        .map(|l| l.qty)
        .sum()
}

impl Bot for Almanac {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
        self.update_info(obs);
        let mut acts = Vec::new();
        if !obs.open_orders.is_empty() {
            acts.push(Action::CancelAll { stock: None });
        }
        let acct = &obs.account;
        let equity = acct.equity as f64;
        if equity <= 0.0 {
            return acts;
        }
        let prices: Vec<f64> = obs.books.iter().map(|b| mid(b).unwrap_or(0.0)).collect();
        let (w, alpha) = self.targets(obs, &prices);
        let fee = self.rules.trade_fee_bps as f64 / 10_000.0;
        let mut power = acct.buying_power as f64;
        let mut traded = 0.0;
        let max_actions = self
            .rules
            .max_actions_per_tick
            .saturating_sub(acts.len() + 2);

        // Sell first so that buys can use the freed buying power next tick.
        let mut order: Vec<usize> = (0..self.n).collect();
        order.sort_by(|&a, &b| {
            let da = w[a] - acct.shares[a] as f64 * prices[a] / equity;
            let db = w[b] - acct.shares[b] as f64 * prices[b] / equity;
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        });
        let over = acct.leverage > self.max_lev * 1.15;
        for &s in &order {
            if acts.len() >= max_actions {
                break;
            }
            let b = &obs.books[s];
            let (Some(bid), Some(ask)) = (b.best_bid(), b.best_ask()) else {
                continue;
            };
            let p = prices[s];
            let cur = acct.shares[s] as f64 * p / equity;
            let mut diff = w[s] - cur;
            if over && diff > 0.0 {
                continue;
            }
            if over {
                diff = diff.min(-cur * 0.05);
            }
            if diff.abs() < self.band && !(w[s] == 0.0 && acct.sellable[s] > 0) {
                continue;
            }
            let notional = diff.abs().min(self.clip) * equity;
            let slip = (self.slip * alpha[s].abs()).min(0.01);
            if diff > 0.0 {
                let limit = ((ask as f64) * (1.0 + slip)).round() as Price;
                let cost = limit as f64 * (1.0 + fee);
                let room = power.max(0.0) / cost;
                let qty =
                    ((notional / ask as f64).min(room) as Qty).min(depth_to(&b.asks, limit, true));
                if qty > 0 {
                    acts.push(Action::ioc(s, Side::Buy, limit, qty));
                    power -= qty as f64 * cost;
                    traded += qty as f64 * ask as f64;
                }
            } else {
                let limit = ((bid as f64) * (1.0 - slip)).round().max(1.0) as Price;
                let qty = ((notional / bid as f64) as Qty)
                    .min(acct.sellable[s])
                    .min(depth_to(&b.bids, limit, false));
                if qty > 0 {
                    acts.push(Action::ioc(s, Side::Sell, limit, qty));
                    traded += qty as f64 * bid as f64;
                }
            }
        }

        // Idle-fee top-up before settlement: same-tick round trips in the tightest held
        // stocks; with nothing held, a purchase that the rebalancing sells next tick.
        let idle = &acct.idle;
        if self.top_up_ticks > 0
            && idle.period_end_tick >= obs.tick
            && idle.period_end_tick - obs.tick < self.top_up_ticks
        {
            let mut short = (idle.fee - idle.credit) as f64 / self.rules.credit_ratio - traded;
            let busy: Vec<usize> = acts
                .iter()
                .filter_map(|a| match a {
                    Action::Limit { stock, .. } => Some(*stock),
                    _ => None,
                })
                .collect();
            let mut cands: Vec<(f64, usize)> = (0..self.n)
                .filter(|s| !busy.contains(s))
                .filter_map(|s| {
                    let b = &obs.books[s];
                    let (bid, ask) = (b.best_bid()?, b.best_ask()?);
                    let spread = (ask - bid) as f64 / ((ask + bid) as f64 / 2.0);
                    (spread <= self.wash_spread).then_some((spread, s))
                })
                .collect();
            cands.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            let held = cands.iter().any(|&(_, s)| acct.sellable[s] > 0);
            for &(_, s) in &cands {
                if short <= 0.0 || acts.len() + 2 > self.rules.max_actions_per_tick {
                    break;
                }
                let b = &obs.books[s];
                let (bid, ask) = (b.bids[0].price, b.asks[0].price);
                let sell_limit = ((bid as f64) * (1.0 - self.wash_depth)).round() as Price;
                let buy_limit = ((ask as f64) * (1.0 + self.wash_depth)).round() as Price;
                let want = (short / 2.0 / bid as f64).ceil() as Qty;
                let room = (power.max(0.0) / (buy_limit as f64 * (1.0 + fee))) as Qty;
                if held {
                    let qty = want
                        .min(acct.sellable[s])
                        .min(depth_to(&b.bids, sell_limit, false))
                        .min(depth_to(&b.asks, buy_limit, true));
                    if qty > 0 {
                        acts.push(Action::ioc(s, Side::Sell, sell_limit, qty));
                        acts.push(Action::ioc(s, Side::Buy, buy_limit, qty));
                        short -= 2.0 * qty as f64 * bid as f64;
                    }
                } else {
                    let qty = (2 * want).min(room).min(depth_to(&b.asks, buy_limit, true));
                    if qty > 0 {
                        acts.push(Action::ioc(s, Side::Buy, buy_limit, qty));
                        power -= qty as f64 * buy_limit as f64 * (1.0 + fee);
                        short -= qty as f64 * ask as f64;
                    }
                }
            }
        }
        acts
    }
}
