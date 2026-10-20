//! Ebb: Coral's value quotes and tracker's informed-flow following, plus an endgame
//! exit. Prices stay well above the value of holding to the end right up to the last
//! tick, so in the final `endgame` ticks Ebb stops buying above its low value estimate
//! and sells its stock down on a straight-line schedule, as long as the bid pays at
//! least that estimate.

use crate::common::*;
use crate::coral::Coral;
use crate::strategic::Tracker;
use bot_api::*;

pub struct Ebb {
    rules: Rules,
    know: Knowledge,
    core: Box<dyn Bot>,
    endgame: Tick,
    floor: f64,
    chunk: f64,
    /// Holdings when the endgame began; the schedule sells these down to zero.
    start_shares: Option<Vec<Qty>>,
}

impl Ebb {
    pub fn new(s: &Start) -> Self {
        // Fixed follow settings: a stricter trigger and smaller stake than tracker's.
        let mut follow = s.clone();
        for (k, v) in [("trigger", 3.0), ("size", 0.15), ("hold", 100.0)] {
            follow.params.entry(k.to_string()).or_insert(v);
        }
        Self {
            rules: s.rules.clone(),
            know: Knowledge::default(),
            core: if s.param("follow", 1.0) > 0.0 {
                Box::new(Tracker::with_core(&follow, Some(Box::new(Coral::new(s)))))
            } else {
                Box::new(Coral::new(s))
            },
            endgame: s.param("endgame", 300.0) as Tick,
            floor: s.param("floor", 1.0),
            chunk: s.param("chunk", 0.1),
            start_shares: None,
        }
    }
}

impl Bot for Ebb {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
        self.know.update(obs);
        let mut acts = self.core.on_tick(obs);
        let r = &self.rules;
        let total = r.ticks_total;
        let begin = total.saturating_sub(self.endgame);
        if obs.tick < begin {
            return acts;
        }
        let start = self
            .start_shares
            .get_or_insert_with(|| obs.account.shares.clone())
            .clone();
        let low: Vec<_> = self.know.pending_with(Estimate::Low).collect();
        let t_lo = self.know.terminal(r, Estimate::Low);
        let lows: Vec<f64> = obs
            .books
            .iter()
            .enumerate()
            .map(|(s, b)| hold_value(r, obs.tick, s, b.dividend, &low, t_lo) * self.floor)
            .collect();
        // Only buy below the low estimate from here on.
        acts.retain(|a| match a {
            Action::Limit {
                stock,
                side: Side::Buy,
                price,
                ..
            } => (*price as f64) < lows[*stock],
            Action::Market {
                side: Side::Buy, ..
            } => false,
            _ => true,
        });
        for o in &obs.open_orders {
            if o.side == Side::Buy && o.price as f64 >= lows[o.stock] {
                acts.insert(
                    0,
                    Action::Cancel {
                        order_id: o.order_id,
                    },
                );
            }
        }
        let left = (total - obs.tick) as f64 / self.endgame.max(1) as f64;
        for (s, b) in obs.books.iter().enumerate() {
            let target = (start[s] as f64 * left) as Qty;
            let excess = obs.account.sellable[s].min(obs.account.shares[s] - target);
            if excess <= 0 {
                continue;
            }
            if let Some(bp) = b.best_bid().filter(|&p| p as f64 >= lows[s]) {
                let q = excess.min(((start[s] as f64 * self.chunk) as Qty).max(1));
                acts.insert(0, Action::ioc(s, Side::Sell, bp, q));
            }
        }
        acts.truncate(r.max_actions_per_tick);
        acts
    }
}
