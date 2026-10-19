//! Followers of their group's actual recent profits/losses. No valuation,
//! news, forecast, terminal value, chart-pattern or order-flow inference.
use crate::common::{affordable, jittered, round_tick};
use crate::strategic::{requote, Want};
use bot_api::*;
use serde::Serialize;

#[derive(Clone, Debug, Default, Serialize)]
pub struct Signal {
    pub peer_profit: f64,
    pub own_profit: f64,
    pub price_return: f64,
    /// Eligible positive-return rank; absent when disabled or below the floor.
    pub leader_rank: Option<usize>,
    /// Contribution to score, including this member's sensitivity.
    pub leader_contribution: f64,
    pub score: f64,
    pub target_fraction: f64,
    pub target_shares: Qty,
}

pub struct ProfitEffect {
    rules: Rules,
    neutral: Vec<f64>,
    targets: Vec<f64>,
    previous_prices: Vec<Price>,
    price_returns: Vec<f64>,
    last_tick: Tick,
    sensitivity: f64,
    peer_weight: f64,
    own_weight: f64,
    price_weight: f64,
    leader_weight: f64,
    leader_min_return: f64,
    loss_weight: f64,
    threshold: f64,
    confidence: f64,
    cap: f64,
    clip: f64,
    chase: f64,
}

impl ProfitEffect {
    pub fn new(s: &Start) -> Self {
        let prices: Vec<_> = s.rules.stocks.iter().map(|x| x.initial_price).collect();
        let equity = (s.cash
            + s.shares
                .iter()
                .zip(&prices)
                .map(|(q, p)| q * p)
                .sum::<Money>())
        .max(1) as f64;
        let cap = s.param("cap", 0.30).clamp(0.01, 0.9);
        let neutral: Vec<_> = s
            .shares
            .iter()
            .zip(&prices)
            .map(|(q, p)| (*q as f64 * *p as f64 / equity).min(cap))
            .collect();
        Self {
            rules: s.rules.clone(),
            // Existing endowed stock seeds experience without mandatory random
            // purchases. A cash-only member can join when peers report profits.
            targets: neutral.clone(),
            neutral,
            price_returns: vec![0.0; prices.len()],
            previous_prices: prices,
            last_tick: 0,
            sensitivity: jittered(s, "sensitivity", 1.0, 0.6).clamp(0.0, 10.0),
            peer_weight: jittered(s, "peer_weight", 1.0, 0.5).clamp(0.0, 10.0),
            own_weight: jittered(s, "own_weight", 0.6, 0.5).clamp(0.0, 10.0),
            price_weight: jittered(s, "price_weight", 0.15, 0.8).clamp(0.0, 10.0),
            // Opt-in per member: group-only followers remain group-only.
            leader_weight: s.param("leader_weight", 0.0).clamp(0.0, 10.0),
            leader_min_return: s.param("leader_min_return", 0.005).clamp(0.0, 1.0),
            loss_weight: jittered(s, "loss_weight", 1.5, 0.3).clamp(1.0, 5.0),
            threshold: jittered(s, "threshold", 0.15, 0.7).clamp(0.0, 5.0),
            confidence: s.param("confidence", 0.25).clamp(0.0, 1.0),
            cap,
            clip: s.param("clip", 0.02).clamp(0.001, 0.2),
            chase: s.param("chase", 0.03).clamp(0.001, 0.2),
        }
    }

    /// Also used by the read-only debugger probe; its actions are checked
    /// against the actual bot. Signals never include hidden world information.
    pub fn evaluate(&mut self, o: &Observation) -> (Vec<Signal>, Vec<Action>) {
        let n = self.neutral.len();
        let mut signals = vec![Signal::default(); n];
        let decay = (-(o.tick.saturating_sub(self.last_tick) as f64) / 100.0).exp();
        self.last_tick = o.tick;
        let confidence = o.social.as_ref().map_or(1.0, |x| {
            (1.0 + self.confidence * x.own_net / 0.03).clamp(0.25, 1.5)
        });
        for s in 0..n {
            // Optional plain price change, not a chart pattern or MA crossover.
            let price = o.books[s].last.max(1);
            self.price_returns[s] = self.price_returns[s] * decay
                + price as f64 / self.previous_prices[s].max(1) as f64
                - 1.0;
            self.previous_prices[s] = price;
        }
        let mut leaders = Vec::new();
        if self.leader_weight > 0.0 {
            // Only public observed prices; rising relative leaders can restart
            // participation when everybody in the group holds cash. A stock
            // merely falling less than the others is never a positive story.
            leaders.extend((0..n).filter(|&s| self.price_returns[s] > self.leader_min_return));
            leaders.sort_by(|&a, &b| {
                self.price_returns[b]
                    .total_cmp(&self.price_returns[a])
                    .then(a.cmp(&b))
            });
            leaders.truncate(2);
        }
        for s in 0..n {
            let (own, peer) = o.social.as_ref().map_or((0.0, 0.0), |x| {
                (
                    x.own_stock.get(s).copied().unwrap_or(0.0),
                    x.peer_stock.get(s).copied().unwrap_or(0.0),
                )
            });
            let amplify_loss = |x: f64| if x < 0.0 { x * self.loss_weight } else { x };
            let leader_rank = leaders.iter().position(|&stock| stock == s).map(|r| r + 1);
            let leader = leader_rank.map_or(0.0, |rank| {
                self.leader_weight * (self.price_returns[s] / 0.03).min(2.0) / rank as f64
            });
            let raw = self.peer_weight * amplify_loss(peer / 0.003)
                + self.own_weight * amplify_loss(own / 0.003)
                + self.price_weight * amplify_loss(self.price_returns[s] / 0.03)
                + leader;
            let score = self.sensitivity * raw;
            let drive = score.signum() * (score.abs() - self.threshold).max(0.0);
            let desired = (self.neutral[s] + self.cap * 0.4 * drive).clamp(0.0, self.cap);
            if drive > 0.0 {
                self.targets[s] = desired;
            } else if drive < 0.0 {
                // Fading losses alone never cause an automatic re-entry.
                self.targets[s] = self.targets[s].min(desired);
            }
            let fraction = (self.targets[s] * confidence).clamp(0.0, self.cap);
            signals[s] = Signal {
                peer_profit: peer,
                own_profit: own,
                price_return: self.price_returns[s],
                leader_rank,
                leader_contribution: self.sensitivity * leader,
                score,
                target_fraction: fraction,
                target_shares: 0,
            };
        }
        // Fixed cash-only group: popularity reallocates a finite pool, never
        // creates buying power. Simultaneous hot stocks share the 95% ceiling.
        let total: f64 = signals.iter().map(|x| x.target_fraction).sum();
        let scale = if total > 0.95 { 0.95 / total } else { 1.0 };
        let equity = o.account.equity.max(0) as f64;
        let mut cash = (o.account.cash - o.account.reserved_buy)
            .max(0)
            .min(o.account.buying_power);
        let mut want = vec![Want::default(); n];
        for k in 0..n {
            let s = (k + o.tick as usize) % n;
            let b = &o.books[s];
            let mark = b.mark.max(1);
            signals[s].target_fraction *= scale;
            let target = (equity * signals[s].target_fraction / mark as f64) as Qty;
            signals[s].target_shares = target;
            let diff = target - o.account.shares[s];
            // A small quantity dead band limits fee-driven churning, without
            // locking a losing holding or delaying a signal change.
            let band = ((equity * 0.005 / mark as f64) as Qty).max(1);
            if diff.abs() < band && target != 0 {
                continue;
            }
            let clip = ((equity * self.clip / mark as f64) as Qty).max(1);
            if diff > 0 {
                let ceiling = round_tick(&self.rules, b.last.max(1) as f64 * (1.0 + self.chase));
                let price = b.best_ask().unwrap_or(ceiling).min(ceiling);
                let qty = diff.min(clip).min(affordable(&self.rules, cash, price));
                if qty > 0 {
                    if b.best_ask().is_some_and(|p| p <= ceiling) {
                        want[s].take = Some((Side::Buy, price, qty));
                    } else {
                        want[s].bid = Some((price, qty));
                    }
                    cash -= qty * self.rules.buy_reserve_per_share(price);
                }
            } else {
                let floor = round_tick(&self.rules, b.last.max(1) as f64 * (1.0 - self.chase));
                let price = b.best_bid().unwrap_or(floor).max(floor);
                if b.best_bid().is_some_and(|p| p >= floor) {
                    let qty = (-diff).min(clip).min(o.account.sellable[s]);
                    if qty > 0 {
                        want[s].take = Some((Side::Sell, price, qty));
                    }
                } else {
                    let qty = (-diff).min(clip).min(o.account.shares[s]);
                    if qty > 0 {
                        want[s].ask = Some((price, qty));
                    }
                }
            }
        }
        (signals, requote(o, &self.rules, &want, 0.001))
    }
}

impl Bot for ProfitEffect {
    fn on_tick(&mut self, o: &Observation) -> Vec<Action> {
        self.evaluate(o).1
    }
}

#[cfg(test)]
#[path = "profit_effect_tests.rs"]
mod tests;
