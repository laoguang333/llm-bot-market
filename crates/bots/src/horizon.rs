//! Horizon v3: ordinary market execution plus persistent, staged research plans.
//! Uses only Rules and delivered observations. No world seed or dragon identity.
use crate::common::{hold_value, mid_or_last, round_tick, Estimate, Knowledge};
use crate::strategic::{requote, Want};
use bot_api::*;

#[derive(Default)]
struct State {
    premium: f64,
    signed: f64,
    volume: f64,
    tick: Option<Tick>,
    plan: Option<Plan>,
}
#[derive(Clone, Copy)]
struct Plan {
    premium: f64,
    until: Tick,
    adverse_until: Tick,
    trough_price: f64,
}
#[derive(Default)]
struct Path {
    value: f64,
    uncertainty: f64,
    last_positive: Tick,
    adverse_until: Tick,
    trough_rate: f64,
}
pub struct Horizon {
    rules: Rules,
    ordinary: Knowledge,
    knowledge: Knowledge,
    states: Vec<State>,
    leverage: f64,
    cap: f64,
    clip: f64,
    research: f64,
    staged: bool,
    terminal_guard: bool,
}

impl Horizon {
    pub fn new(s: &Start) -> Self {
        Self {
            rules: s.rules.clone(),
            ordinary: Knowledge::default(),
            knowledge: Knowledge::default(),
            states: (0..s.rules.stocks.len())
                .map(|_| State::default())
                .collect(),
            leverage: s.param("leverage", 2.0).clamp(0.0, s.rules.max_leverage),
            cap: s.param("cap", 0.45).clamp(0.0, 1.0),
            clip: s.param("clip", 0.025).clamp(0.0, 0.1),
            research: s.param("research", 1.0).clamp(0.0, 1.0),
            staged: s.param("staged", 1.0) >= 0.5,
            terminal_guard: s.param("terminal_guard", 1.0) >= 0.5,
        }
    }

    fn path(&self, stock: usize, obs: &Observation, terminal: f64) -> Path {
        let r = &self.rules;
        let mut changes: Vec<_> = self
            .knowledge
            .heard
            .iter()
            .filter_map(|(&id, h)| {
                if h.effective >= r.ticks_total {
                    return None;
                }
                h.ranges.get(&stock).map(|&(lo, hi)| {
                    (
                        h.effective,
                        id,
                        (lo as f64 + hi as f64) / 2.0,
                        (hi as f64 - lo as f64) / 12.0_f64.sqrt(),
                    )
                })
            })
            .collect();
        changes.sort_by_key(|v| (v.0, v.1));
        let mut rate = obs.books[stock].dividend as f64;
        let mut result = Path {
            trough_rate: rate,
            ..Path::default()
        };
        let mut at = obs.tick;
        let terminal_count = r.terminal_dividend_payments(terminal);
        let mut variance = 0.0;
        let mut i = 0;
        while i < changes.len() {
            let effective = changes[i].0;
            let before = effective.saturating_sub(1);
            result.value += rate.max(r.min_dividend as f64) / r.dividend_scale as f64
                * crate::common::payments_after(r, at, before) as f64;
            let mut delta = 0.0;
            while i < changes.len() && changes[i].0 == effective {
                delta += changes[i].2;
                let sensitivity = (crate::common::payments_after(r, before, r.ticks_total) as f64
                    + terminal_count)
                    / r.dividend_scale as f64;
                variance += (changes[i].3 * sensitivity).powi(2);
                i += 1;
            }
            // Keep the latent deficit of incomplete paths; floor only payments.
            rate += delta;
            if delta > 0.0 {
                result.last_positive = effective;
            }
            if rate < result.trough_rate {
                result.trough_rate = rate;
                result.adverse_until = effective;
            }
            at = before;
        }
        result.value += rate.max(r.min_dividend as f64) / r.dividend_scale as f64
            * (crate::common::payments_after(r, at, r.ticks_total) as f64 + terminal_count);
        result.uncertainty = variance.sqrt();
        result
    }

    fn phase(plan: Plan, tick: Tick, mid: f64, staged: bool) -> f64 {
        if !staged {
            return 1.0;
        }
        if tick >= plan.until.saturating_sub(120) {
            return 1.0;
        }
        if tick < plan.adverse_until && mid > plan.trough_price * 1.03 {
            0.2
        } else {
            0.65
        }
    }
}

impl Bot for Horizon {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
        self.knowledge.update(obs);
        // Ordinary disclosure remains available even for accounts with research.
        // Do not feed extra forecasts into the fast market calibration itself.
        for &(lo, hi) in &obs.terminal_hints {
            let range = self.ordinary.terminal.get_or_insert((lo, hi));
            *range = (range.0.max(lo), range.1.min(hi));
        }
        for news in obs.news.iter().filter(|n| !n.forecast) {
            let h = self
                .ordinary
                .heard
                .entry(news.id)
                .or_insert_with(|| crate::common::Heard {
                    ranges: Default::default(),
                    effective: news.effective_tick,
                    public: false,
                });
            h.public |= news.public;
            for c in &news.changes {
                let range = h.ranges.entry(c.stock).or_insert((c.low, c.high));
                *range = (range.0.max(c.low), range.1.min(c.high));
            }
        }
        self.ordinary.heard.retain(|_, h| h.effective > obs.tick);
        let r = &self.rules;
        let terminal = self.knowledge.terminal(r, Estimate::Mid);
        let terminal_low = self.knowledge.terminal(r, Estimate::Low);
        let pending: Vec<_> = self.ordinary.pending().collect();
        let eq = obs.account.equity.max(0) as f64;
        let n = self.states.len();
        let remaining = r.ticks_total.saturating_sub(obs.tick) as f64;
        let financed =
            (-(obs.account.cash as f64) / (obs.account.gross as f64).max(1.0)).clamp(0.0, 1.0);
        let mut weights = vec![0.0; n];
        let mut fairs = vec![0.0; n];
        let mut terminal_assets = 0.0;
        let mut reserve = 0.0;
        for s in 0..n {
            let b = &obs.books[s];
            let mid = mid_or_last(b).max(r.tick_size) as f64;
            let ordinary = hold_value(r, obs.tick, s, b.dividend, &pending, terminal).max(1.0);
            let thesis = self.path(s, obs, terminal);
            let st = &mut self.states[s];
            let dt = st.tick.map_or(1, |t| obs.tick.saturating_sub(t)).max(1) as f64;
            if st.tick.is_none() {
                st.premium = mid / ordinary;
            } else {
                let decay = (-dt / 150.0).exp();
                st.premium = decay * st.premium + (1.0 - decay) * mid / ordinary;
            }
            let decay = (-dt / 20.0).exp();
            st.signed *= decay;
            st.volume *= decay;
            for t in &obs.trades[s] {
                let q = t.qty.max(0) as f64;
                st.volume += q;
                st.signed += if t.aggressor == Side::Buy { q } else { -q };
            }
            st.tick = Some(obs.tick);
            let convergence = (remaining / 400.0).min(1.0);
            let premium = 1.0 + convergence * (st.premium.clamp(0.3, 3.0) - 1.0);
            let flow = st.signed / st.volume.max(1.0);
            let ordinary_fair = ordinary * premium * (1.0 + 0.02 * flow);
            fairs[s] = ordinary_fair;
            weights[s] = (12.0 * (ordinary_fair / mid - 1.0)).clamp(0.0, self.cap);
            if st.plan.is_some_and(|p| obs.tick > p.until) {
                st.plan = None;
            }
            // A plan is not a compulsory position. A favourable research path
            // gets a durable anchor; known bad/then-good news is valued jointly.
            let alpha = thesis.value / ordinary - 1.0;
            let extra =
                self.knowledge.heard.iter().any(|(id, h)| {
                    !self.ordinary.heard.contains_key(id) && h.ranges.contains_key(&s)
                });
            let actionable =
                self.research > 0.0 && extra && alpha > 0.02 && thesis.last_positive > obs.tick;
            if actionable {
                let until = thesis
                    .last_positive
                    .saturating_add(100)
                    .min(r.ticks_total - 1);
                let raw = thesis.trough_rate.max(r.min_dividend as f64) / r.dividend_scale as f64
                    * (crate::common::payments_after(r, obs.tick, r.ticks_total) as f64
                        + r.terminal_dividend_payments(terminal));
                let plan = st.plan.get_or_insert(Plan {
                    premium,
                    until,
                    adverse_until: thesis.adverse_until,
                    trough_price: raw * premium,
                });
                plan.until = plan.until.max(until);
                // Preserve the anchor, but revise the plan when actual news changes.
                if thesis.adverse_until > obs.tick {
                    plan.adverse_until = thesis.adverse_until;
                    plan.trough_price = raw * plan.premium;
                }
            }
            if let Some(plan) = st.plan.filter(|_| self.research > 0.0) {
                let wait = plan.until.saturating_sub(obs.tick) as f64;
                let financing = financed * r.loan_rate * wait / r.interest_period as f64;
                let plan_premium = 1.0 + convergence * (plan.premium - 1.0);
                let fair = thesis.value.max(0.0) * plan_premium / (1.0 + financing);
                let confidence = 1.0 / (1.0 + 4.0 * thesis.uncertainty / thesis.value.max(1.0));
                let costs = 2.0 * r.trade_fee_bps as f64 / 10000.0;
                let full = (4.0 * (fair / mid - 1.0 - costs)).clamp(0.0, self.cap)
                    * confidence
                    * self.research;
                let phase = Self::phase(plan, obs.tick, mid, self.staged);
                reserve += full * (1.0 - phase);
                weights[s] = weights[s].max(full * phase);
                // Preserve an existing sound thesis through its temporary dip.
                // This is a ceiling on retention, not permission to average down infinitely.
                if fair > mid * (1.0 + costs) && full > 0.0 {
                    let held = obs.account.shares[s] as f64 * mid / eq.max(1.0);
                    weights[s] = weights[s].max(held.min(full));
                    fairs[s] = fairs[s].max(fair);
                }
                // Adverse revisions end the research position; ordinary trading
                // may still justify inventory using its own contemporary signal.
                if fair < mid * 0.95 && alpha <= 0.0 {
                    st.plan = None;
                }
            }
            // A conservative terminal asset estimate excludes unearned growth.
            let risk_knowledge = if self.research > 0.0 {
                &self.knowledge
            } else {
                &self.ordinary
            };
            let negative: f64 = risk_knowledge
                .heard
                .values()
                .filter_map(|h| h.ranges.get(&s))
                .map(|&(lo, _)| lo.min(0) as f64)
                .sum();
            let rate = (b.dividend as f64 + negative).max(r.min_dividend as f64);
            terminal_assets += obs.account.shares[s] as f64 * rate / r.dividend_scale as f64
                * r.terminal_dividend_payments(terminal_low);
        }
        let leverage = (self.leverage - reserve.min(0.3)).max(0.0);
        let scale = (leverage / weights.iter().sum::<f64>().max(0.001)).min(1.0);
        let terminal_window = (r.ticks_total as f64 * 0.1).max(400.0);
        let debt = (-(obs.account.cash as f64)).max(0.0);
        let terminal_risk =
            self.terminal_guard && remaining < terminal_window && debt > 0.7 * terminal_assets;
        let marked_risk = eq <= 0.0 || obs.account.gross as f64 > eq * self.leverage * 1.15;
        let reduce_risk = terminal_risk || marked_risk;
        let mut budget =
            (eq * leverage - obs.account.gross as f64 - obs.account.reserved_buy as f64)
                .max(0.0)
                .min(obs.account.buying_power.max(0) as f64);
        if reduce_risk {
            budget = 0.0;
        }
        let mut want = vec![Want::default(); n];
        for k in 0..n {
            let s = (k + obs.tick as usize) % n;
            let b = &obs.books[s];
            let mid = mid_or_last(b).max(r.tick_size) as f64;
            let have = obs.account.shares[s];
            let mut target = (eq * weights[s] * scale / mid) as Qty;
            if reduce_risk {
                target = target.min(have * 9 / 10);
            }
            let lot = (eq * self.clip / mid).max(1.0) as Qty;
            if have < target {
                let limit = round_tick(r, (fairs[s] * 0.9985).min(mid * 1.005));
                let qty = (target - have)
                    .min(lot)
                    .min((budget / r.buy_reserve_per_share(limit) as f64) as Qty)
                    .max(0);
                if qty == 0 {
                    continue;
                }
                if b.best_ask().is_some_and(|p| p <= limit) {
                    want[s].take = Some((Side::Buy, limit, qty));
                } else {
                    want[s].bid = Some((
                        limit.min(b.best_bid().unwrap_or(limit).saturating_add(r.tick_size)),
                        qty,
                    ));
                }
                budget -= qty as f64 * r.buy_reserve_per_share(limit) as f64;
            } else if have > target {
                let qty = (have - target).min(lot).min(obs.account.sellable[s]);
                if qty <= 0 {
                    continue;
                }
                let limit = round_tick(
                    r,
                    if reduce_risk {
                        mid * 0.995
                    } else {
                        (fairs[s] * 1.0015).max(mid * 0.995)
                    },
                );
                if b.best_bid().is_some_and(|p| p >= limit) {
                    want[s].take = Some((Side::Sell, limit, qty));
                } else {
                    want[s].ask = Some((
                        limit.max(b.best_ask().unwrap_or(limit).saturating_sub(r.tick_size)),
                        qty,
                    ));
                }
            }
        }
        requote(obs, r, &want, 0.0)
    }
}

#[cfg(test)]
#[path = "horizon_tests.rs"]
mod tests;
