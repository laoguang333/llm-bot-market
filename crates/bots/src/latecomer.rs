//! Delayed attention to visible price moves. Intent is timestamped when formed;
//! execution remains bounded by that old price and expires if it cannot complete.
use crate::crowd_execution::{parameter, Execution};
use bot_api::{Action, Bot, Observation, Price, Qty, Side, Start, Tick};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

#[derive(Clone)]
struct Intent {
    side: Side,
    due: Tick,
    expires: Tick,
    reference: Price,
    target: Qty,
}

#[derive(Clone, Default)]
struct Attention {
    anchor: f64,
    pending: Option<Intent>,
    cooldown_until: Tick,
}

pub struct Latecomer {
    execution: Execution,
    attention: Vec<Attention>,
    previous_tick: Option<Tick>,
    memory: f64,
    threshold: f64,
    delay: Tick,
    patience: Tick,
    cap: f64,
    invested: f64,
}

impl Latecomer {
    pub fn new(start: &Start) -> Self {
        let mut rng = ChaCha8Rng::seed_from_u64(start.seed);
        let memory = parameter(start, &mut rng, "memory", 30.0..=100.0, 1.0..=1000.0);
        let threshold = parameter(start, &mut rng, "threshold", 0.008..=0.025, 0.001..=0.2);
        let delay = parameter(start, &mut rng, "delay", 40.0..=160.0, 0.0..=1000.0).round() as Tick;
        let patience =
            parameter(start, &mut rng, "patience", 30.0..=90.0, 1.0..=500.0).round() as Tick;
        let cap = parameter(start, &mut rng, "cap", 0.15..=0.25, 0.01..=0.5);
        let invested = parameter(start, &mut rng, "invested", 0.75..=0.9, 0.1..=0.95);
        let clip = parameter(start, &mut rng, "clip", 0.015..=0.035, 0.001..=0.1);
        let chase = parameter(start, &mut rng, "chase", 0.06..=0.15, 0.001..=0.3);
        Self {
            execution: Execution::new(start, clip, chase),
            attention: vec![Attention::default(); start.rules.stocks.len()],
            previous_tick: None,
            memory,
            threshold,
            delay,
            patience,
            cap,
            invested,
        }
    }
}

impl Bot for Latecomer {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
        let elapsed = self.previous_tick.map_or(0, |t| obs.tick.saturating_sub(t));
        self.previous_tick = Some(obs.tick);
        let decay = (-(elapsed as f64) / self.memory).exp();
        let n = self.attention.len();
        let mut targets = obs.account.shares.clone();
        let mut references = vec![0; n];
        // Cash-only and total-stock limits apply jointly to all matured buy intents.
        let held_value: f64 = obs
            .account
            .shares
            .iter()
            .zip(&obs.books)
            .map(|(q, b)| *q as f64 * b.last.max(1) as f64)
            .sum();
        let mut additional =
            (obs.account.equity.max(0) as f64 * self.invested - held_value).max(0.0);
        // Rotate priority so concurrent intents do not always favour stock zero.
        for offset in 0..n {
            let s = (obs.tick as usize + offset) % n;
            let book = &obs.books[s];
            let state = &mut self.attention[s];
            let price = book.last.max(1);
            if state.anchor <= 0.0 {
                state.anchor = price as f64;
            }
            let deviation = price as f64 / state.anchor - 1.0;
            state.anchor = state.anchor * decay + price as f64 * (1.0 - decay);
            let signal = if deviation >= self.threshold {
                Some(Side::Buy)
            } else if deviation <= -self.threshold {
                Some(Side::Sell)
            } else {
                None
            };
            if let Some(intent) = &state.pending {
                // Opposite visible evidence invalidates an old intention; neutral does not.
                if obs.tick > intent.expires || signal.is_some_and(|side| side != intent.side) {
                    state.pending = None;
                }
            }
            if state.pending.is_none() && obs.tick >= state.cooldown_until {
                if let Some(side) = signal {
                    let target = match side {
                        Side::Buy => (obs.account.equity.max(0) as f64 * self.cap / price as f64)
                            .floor() as Qty,
                        Side::Sell => 0,
                    };
                    let needed = match side {
                        Side::Buy => target > obs.account.shares[s],
                        Side::Sell => obs.account.shares[s] > 0,
                    };
                    if needed {
                        let due = obs.tick.saturating_add(self.delay);
                        state.pending = Some(Intent {
                            side,
                            due,
                            expires: due.saturating_add(self.patience),
                            reference: price,
                            target,
                        });
                        state.cooldown_until = due.saturating_add(self.patience);
                    }
                }
            }
            if let Some(intent) = &state.pending {
                if obs.tick < intent.due {
                    continue;
                }
                references[s] = intent.reference;
                targets[s] = match intent.side {
                    Side::Sell => 0,
                    Side::Buy => {
                        // A fall in equity cannot turn a historical target into new leverage.
                        let current_cap = (obs.account.equity.max(0) as f64 * self.cap
                            / price as f64)
                            .floor() as Qty;
                        let wanted = intent
                            .target
                            .min(current_cap)
                            .saturating_sub(obs.account.shares[s])
                            .max(0);
                        let allowed = wanted.min((additional / price as f64).floor() as Qty);
                        additional -= allowed as f64 * price as f64;
                        obs.account.shares[s] + allowed
                    }
                };
            }
        }
        self.execution.orders(obs, &targets, Some(&references))
    }
}
