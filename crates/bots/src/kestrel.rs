//! Kestrel: news-adjusted market value, tape confirmation and spread-aware inventory.
//! Ordinary account only: Rules, delivered news/hints, public books/trades and own account.
//! No seed, identity, hidden world, external IO, or research access is used.
use crate::common::{mid_or_last, payments_after, Estimate, Knowledge};
use bot_api::*;

#[derive(Default)]
struct State {
    premium: f64,
    price: f64,
    volume: f64,
    signed: f64,
    tick: Option<Tick>,
}

pub struct Kestrel {
    rules: Rules,
    knowledge: Knowledge,
    states: Vec<State>,
    horizon: f64,
    gain: f64,
    cap: f64,
    leverage: f64,
    flow: f64,
    momentum: f64,
    edge: f64,
    clip: f64,
    base: f64,
}

impl Kestrel {
    pub fn new(s: &Start) -> Self {
        Self {
            rules: s.rules.clone(),
            knowledge: Knowledge::default(),
            states: (0..s.rules.stocks.len())
                .map(|_| State::default())
                .collect(),
            horizon: s.param("horizon", 180.0).max(1.0),
            gain: s.param("gain", 14.0).max(0.0),
            cap: s.param("cap", 0.55).clamp(0.0, 1.0),
            leverage: s.param("leverage", 1.8).clamp(0.0, s.rules.max_leverage),
            flow: s.param("flow", 0.025),
            momentum: s.param("momentum", 0.3),
            edge: s.param("edge", 0.002).max(0.0),
            clip: s.param("clip", 0.08).clamp(0.0, 1.0),
            base: s.param("base", 0.0).max(0.0),
        }
    }

    // Integrate only delivered events. Aggregate simultaneous changes before flooring
    // payments, retaining a latent negative rate just as the public world rules do.
    fn value(&self, obs: &Observation, stock: usize, terminal: f64) -> f64 {
        let r = &self.rules;
        let mut changes: Vec<_> = self
            .knowledge
            .heard
            .iter()
            .filter_map(|(&id, h)| {
                h.ranges
                    .get(&stock)
                    .filter(|_| h.effective < r.ticks_total)
                    .map(|&(lo, hi)| (h.effective, id, (lo as f64 + hi as f64) * 0.5))
            })
            .collect();
        changes.sort_by_key(|x| (x.0, x.1));
        let mut rate = obs.books[stock].dividend as f64;
        let mut at = obs.tick;
        let mut value = 0.0;
        let mut i = 0;
        while i < changes.len() {
            let effective = changes[i].0;
            let before = effective.saturating_sub(1);
            value += rate.max(r.min_dividend as f64) / r.dividend_scale as f64
                * payments_after(r, at, before) as f64;
            while i < changes.len() && changes[i].0 == effective {
                rate += changes[i].2;
                i += 1;
            }
            at = before;
        }
        value
            + rate.max(r.min_dividend as f64) / r.dividend_scale as f64
                * (payments_after(r, at, r.ticks_total) as f64
                    + r.terminal_dividend_payments(terminal))
    }
}

impl Bot for Kestrel {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
        self.decide(obs, &[])
    }
}

impl Kestrel {
    /// Optional bounded valuation premiums supplied by a public-signal strategy.
    /// Empty premiums preserve the original Kestrel decisions and risk limits.
    pub(crate) fn decide(&mut self, obs: &Observation, premiums: &[f64]) -> Vec<Action> {
        self.decide_with_holding(obs, premiums, &[])
    }

    /// Holding tolerance affects selling only; it never raises the buy price.
    pub(crate) fn decide_with_holding(
        &mut self,
        obs: &Observation,
        premiums: &[f64],
        holding: &[f64],
    ) -> Vec<Action> {
        self.knowledge.update(obs);
        let r = &self.rules;
        let n = self.states.len();
        let eq = obs.account.equity.max(0) as f64;
        let remaining = r.ticks_total.saturating_sub(obs.tick) as f64;
        let terminal = self.knowledge.terminal(r, Estimate::Mid);
        let terminal_low = self.knowledge.terminal(r, Estimate::Low);
        let mut fairs = vec![0.0; n];
        let mut weights = vec![0.0; n];
        for s in 0..n {
            let b = &obs.books[s];
            let mid = mid_or_last(b).max(r.tick_size) as f64;
            let value = self.value(obs, s, terminal).max(1.0);
            let st = &mut self.states[s];
            let dt = st.tick.map_or(1, |t| obs.tick.saturating_sub(t)).max(1) as f64;
            if st.tick.is_none() {
                st.premium = mid / value;
                st.price = mid;
            } else {
                let a = (-dt / self.horizon).exp();
                st.premium = a * st.premium + (1.0 - a) * mid / value;
                let a = (-dt / 60.0).exp();
                st.price = a * st.price + (1.0 - a) * mid;
            }
            let decay = (-dt / 20.0).exp();
            st.volume *= decay;
            st.signed *= decay;
            for trade in &obs.trades[s] {
                let q = trade.qty.max(0) as f64;
                st.volume += q;
                st.signed += if trade.aggressor == Side::Buy { q } else { -q };
            }
            st.tick = Some(obs.tick);
            let convergence = (remaining / 500.0).min(1.0);
            let premium = 1.0 + convergence * (st.premium.clamp(0.2, 5.0) - 1.0);
            let flow = st.signed / st.volume.max(1.0);
            let momentum = (mid / st.price.max(1.0) - 1.0).clamp(-0.05, 0.05);
            let fair = value
                * premium
                * (1.0 + convergence * (self.flow * flow + self.momentum * momentum))
                * (1.0 + premiums.get(s).copied().unwrap_or(0.0).clamp(0.0, 0.05));
            fairs[s] = fair;
            weights[s] = (self.base + self.gain * (fair / mid - 1.0)).clamp(0.0, self.cap);
        }
        let scale = (self.leverage / weights.iter().sum::<f64>().max(0.001)).min(1.0);
        let mut budget =
            (eq * self.leverage - obs.account.gross as f64 - obs.account.reserved_buy as f64)
                .max(0.0)
                .min(obs.account.buying_power.max(0) as f64);
        let terminal_assets: f64 = (0..n)
            .map(|s| {
                let negative: f64 = self
                    .knowledge
                    .heard
                    .values()
                    .filter_map(|h| h.ranges.get(&s))
                    .map(|&(lo, _)| lo.min(0) as f64)
                    .sum();
                obs.account.shares[s] as f64
                    * (obs.books[s].dividend as f64 + negative).max(r.min_dividend as f64)
                    / r.dividend_scale as f64
                    * r.terminal_dividend_payments(terminal_low)
            })
            .sum();
        let risk = eq <= 0.0
            || obs.account.gross as f64 > eq * self.leverage * 1.12
            || (remaining < (r.ticks_total as f64 * 0.1).max(400.0)
                && -(obs.account.cash as f64) > 0.7 * terminal_assets);
        if risk {
            budget = 0.0;
        }
        let mut out = Vec::new();
        // IOC orders cannot leave stale quotes. Cancel externally supplied/legacy
        // orders first; no released cash or shares are assumed available this tick.
        if !obs.open_orders.is_empty() {
            if r.max_actions_per_tick > 0 {
                out.push(Action::CancelAll { stock: None });
            }
            return out;
        }
        for k in 0..n {
            if out.len() >= r.max_actions_per_tick {
                break;
            }
            let s = (k + obs.tick as usize) % n;
            let b = &obs.books[s];
            let mid = mid_or_last(b).max(r.tick_size) as f64;
            let have = obs.account.shares[s];
            let lot = (eq * self.clip / mid) as Qty;
            // Ask/bid-specific targets plus an edge create a no-trade region.
            // Inventory does not churn merely because a midpoint moves one tick.
            let target_at = |price: Price, edge: f64, hold: f64| {
                let weight = (self.base
                    + self.gain
                        * (fairs[s] * (1.0 + hold) / price.max(r.tick_size) as f64 - 1.0 + edge))
                    .clamp(0.0, self.cap);
                (eq * scale * weight / mid) as Qty
            };
            let fee = r.trade_fee_bps as f64 / 10000.0;
            if let Some(bid) = b.best_bid() {
                let target = if risk {
                    0
                } else {
                    target_at(
                        bid,
                        self.edge + fee,
                        holding.get(s).copied().unwrap_or(0.0).clamp(0.0, 0.05),
                    )
                };
                let qty = (have - target).max(0).min(obs.account.sellable[s]).min(lot);
                if qty > 0 {
                    out.push(Action::ioc(s, Side::Sell, bid, qty));
                    continue;
                }
            }
            if let Some(ask) = b.best_ask() {
                let target = target_at(ask, -self.edge - fee, 0.0);
                let cost = r.buy_reserve_per_share(ask).max(1);
                let qty = (target - have)
                    .max(0)
                    .min(lot)
                    .min((budget / cost as f64) as Qty);
                if qty > 0 {
                    out.push(Action::ioc(s, Side::Buy, ask, qty));
                    budget -= qty as f64 * cost as f64;
                }
            }
        }
        out
    }
}

#[cfg(test)]
#[path = "kestrel_tests.rs"]
mod tests;
