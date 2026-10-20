//! Tide: buys the stocks whose dividends have fallen furthest below their starting
//! rate. Prices track the value of the current dividend held flat to the end, but a
//! low dividend tends to climb back: the business component of each dividend is
//! nonnegative and drifts back towards its normal level, and news shocks below half
//! the starting rate are sized on that half. Tide values each share with that
//! expected recovery, holds the stocks that trade furthest below it, and keeps the
//! rest in cash. Near the end the recovery has no time left to pay, so the value falls
//! back to the flat one and the stock is sold.
//! Uses only this account's observations and the published rules.

use crate::common::*;
use crate::strategic::{requote, Want};
use bot_api::*;

pub struct Tide {
    rules: Rules,
    know: Knowledge,
    /// Starting dividend rate of each stock: the level a low dividend recovers towards.
    base: Vec<f64>,
    /// Recovery target as a multiple of the starting rate.
    target: f64,
    /// Recovery time constant in ticks.
    tau: f64,
    leverage: f64,
    /// Largest share of equity in one stock.
    cap: f64,
    /// Smallest value / price worth holding.
    hurdle: f64,
    /// Largest order per decision, as a share of equity.
    lot: f64,
    /// Position gap (share of equity) left alone.
    band: f64,
    /// Position gap (share of equity) worth crossing the spread for.
    take: f64,
    tolerance: f64,
    /// Ticks before an idle charge to start buying the missing credit.
    wash_ticks: Tick,
    /// Widest relative spread worth crossing for credit.
    wash_spread: f64,
    /// How far past the touch each wash leg may fill.
    wash_slip: f64,
}

impl Tide {
    pub fn new(s: &Start) -> Self {
        Self {
            rules: s.rules.clone(),
            know: Knowledge::default(),
            base: s
                .rules
                .stocks
                .iter()
                .map(|x| x.initial_dividend.max(1) as f64)
                .collect(),
            target: s.param("target", 1.4),
            tau: s.param("tau", 4000.0).max(1.0),
            leverage: s.param("leverage", 2.0).clamp(0.0, s.rules.max_leverage),
            cap: s.param("cap", 0.3),
            hurdle: s.param("hurdle", 0.9),
            lot: s.param("lot", 0.02),
            band: s.param("band", 0.02),
            take: s.param("take", 0.1),
            tolerance: s.param("tolerance", 0.002),
            wash_ticks: s.param("wash_ticks", 10.0) as Tick,
            wash_spread: s.param("wash_spread", 0.012),
            wash_slip: s.param("wash_slip", 0.003),
        }
    }

    /// Expected value of one share held to the end, in cents. `rate` is the dividend
    /// with known pending changes; `left` and `terminal` count in-game and terminal payments.
    fn value(&self, stock: usize, rate: f64, left: f64, terminal: f64) -> f64 {
        let r = &self.rules;
        let flat = r.dividend_cents(1) * rate * (left + terminal);
        let goal = self.target * self.base[stock];
        if rate >= goal {
            return flat;
        }
        // Expected rate after h ticks: goal + (rate - goal) * exp(-h / tau).
        let decay = (-left / self.tau).exp();
        let gap = (rate - goal) * (self.tau * (1.0 - decay) + terminal * decay);
        r.dividend_cents(1) * (goal * (left + terminal) + gap)
    }

    /// Before an idle charge that trading has not covered, sell and buy back the same
    /// shares at the touch in the tightest books. Each round trip earns the credit on
    /// both legs, which is worth more than the spread and fees it costs.
    fn wash(&self, obs: &Observation, want: &[Want], acts: &mut Vec<Action>) {
        let r = &self.rules;
        let idle = &obs.account.idle;
        let left = idle.period_end_tick.saturating_sub(obs.tick);
        if idle.due <= 0 || left > self.wash_ticks || r.credit_ratio <= 0.0 {
            return;
        }
        let mut need = idle.due as f64 / r.credit_ratio / left.max(1) as f64;
        let mut books: Vec<(f64, usize, Price, Price, Qty)> = obs
            .books
            .iter()
            .enumerate()
            .filter(|(s, _)| {
                obs.account.sellable[*s] > 0
                    && want[*s].bid.is_none()
                    && want[*s].ask.is_none()
                    && want[*s].take.is_none()
                    && !obs.open_orders.iter().any(|o| o.stock == *s)
            })
            .filter_map(|(s, b)| {
                let (bid, ask) = (b.best_bid()?, b.best_ask()?);
                // Walk each side no further than `wash_slip` from the touch.
                let depth = |levels: &[Level], limit: f64, sell: bool| {
                    let mut q = 0;
                    let mut last = 0;
                    for l in levels {
                        if (sell && (l.price as f64) < limit) || (!sell && (l.price as f64) > limit)
                        {
                            break;
                        }
                        q += l.qty;
                        last = l.price;
                    }
                    (q, last)
                };
                let (qs, ps) = depth(&b.bids, bid as f64 * (1.0 - self.wash_slip), true);
                let (qb, pb) = depth(&b.asks, ask as f64 * (1.0 + self.wash_slip), false);
                let q = qs.min(qb).min(obs.account.sellable[s]);
                Some(((ask - bid) as f64 / ask as f64, s, ps, pb, q))
            })
            .filter(|x| x.0 < self.wash_spread && x.4 > 0)
            .collect();
        books.sort_by(|x, y| x.0.total_cmp(&y.0));
        for (_, s, bid, ask, q) in books {
            if need <= 0.0 || acts.len() + 2 > r.max_actions_per_tick {
                break;
            }
            let q = q
                .min((need / (bid + ask) as f64).ceil() as Qty)
                .min(buy_room(&obs.account, r, ask, r.max_leverage));
            if q <= 0 {
                continue;
            }
            acts.push(Action::ioc(s, Side::Sell, bid, q));
            acts.push(Action::ioc(s, Side::Buy, ask, q));
            need -= (q * (bid + ask)) as f64;
        }
    }
}

impl Bot for Tide {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
        self.know.update(obs);
        let r = &self.rules;
        let n = obs.books.len();
        let left = payments_after(r, obs.tick, r.ticks_total) as f64;
        let terminal = r.terminal_dividend_payments(self.know.terminal(r, Estimate::Mid));
        let mut rate: Vec<f64> = obs.books.iter().map(|b| b.dividend as f64).collect();
        for (s, d, _) in self.know.pending() {
            rate[s] += d as f64;
        }
        let eq = obs.account.equity.max(0) as f64;
        let price: Vec<f64> = obs
            .books
            .iter()
            .map(|b| b.mid().unwrap_or(b.last).max(1) as f64)
            .collect();
        let values: Vec<f64> = (0..n)
            .map(|s| self.value(s, rate[s].max(0.0), left, terminal))
            .collect();
        let score: Vec<f64> = (0..n).map(|s| values[s] / price[s]).collect();
        let mut edge: Vec<f64> = score.iter().map(|x| (x - self.hurdle).max(0.0)).collect();
        // Never hold both legs of a set that costs money.
        for c in r.combos.iter().filter(|c| c.bonus < 0) {
            if edge[c.a] > 0.0 && edge[c.b] > 0.0 {
                let worse = if score[c.a] < score[c.b] { c.a } else { c.b };
                edge[worse] = 0.0;
            }
        }
        let total: f64 = edge.iter().sum();
        let mut want = vec![Want::default(); n];
        for (s, b) in obs.books.iter().enumerate() {
            let w = if total > 0.0 {
                (edge[s] / total * self.leverage).min(self.cap)
            } else {
                0.0
            };
            let have = obs.account.shares[s];
            let goal = (w * eq / price[s]) as Qty;
            let gap = (goal - have) as f64 * price[s] / eq.max(1.0);
            let lot = ((eq * self.lot / price[s]) as Qty).max(1);
            if gap > self.band {
                let q = (goal - have).min(lot).min(buy_room(
                    &obs.account,
                    r,
                    b.best_ask().unwrap_or(b.last),
                    self.leverage,
                ));
                if q <= 0 {
                    continue;
                }
                match b.best_ask() {
                    Some(a) if gap > self.take && (a as f64) < values[s] => {
                        want[s].take = Some((Side::Buy, a, q))
                    }
                    _ => {
                        let mut p = b.best_bid().map_or(b.last, |x| x + r.tick_size);
                        if let Some(a) = b.best_ask() {
                            p = p.min(a - r.tick_size);
                        }
                        want[s].bid = Some((p.max(r.tick_size), q));
                    }
                }
            } else if gap < -self.band {
                let q = (have - goal).min(lot).min(obs.account.sellable[s]);
                if q <= 0 {
                    continue;
                }
                match b.best_bid() {
                    Some(p) if -gap > self.take => want[s].take = Some((Side::Sell, p, q)),
                    _ => {
                        let mut p = b.best_ask().map_or(b.last, |x| x - r.tick_size);
                        if let Some(bb) = b.best_bid() {
                            p = p.max(bb + r.tick_size);
                        }
                        want[s].ask = Some((p, q));
                    }
                }
            }
        }
        let mut acts = requote(obs, r, &want, self.tolerance);
        self.wash(obs, &want, &mut acts);
        acts
    }
}
