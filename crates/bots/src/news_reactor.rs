//! An information weakling: extrapolates received headlines, without pricing
//! the remaining cash flows or checking how much news the market has absorbed.
use crate::crowd_execution::{parameter, Execution};
use bot_api::{Action, Bot, Observation, Start, Tick};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use std::collections::BTreeMap;

struct Impression {
    first_seen: Tick,
    stage: usize,
    changes: Vec<(usize, f64)>,
}

pub struct NewsReactor {
    execution: Execution,
    impressions: BTreeMap<u32, Impression>,
    scale: Vec<f64>,
    anchors: Vec<f64>,
    memory: f64,
    amplification: f64,
    baseline: f64,
    cap: f64,
    invested: f64,
}

impl NewsReactor {
    pub fn new(start: &Start) -> Self {
        // Seed supplies individual behavioural traits only; no world reconstruction.
        let mut rng = ChaCha8Rng::seed_from_u64(start.seed);
        let memory = parameter(start, &mut rng, "memory", 180.0..=600.0, 10.0..=2000.0);
        let amplification = parameter(start, &mut rng, "amplification", 3.0..=7.0, 0.0..=12.0);
        let baseline = parameter(start, &mut rng, "baseline", 0.35..=0.65, 0.0..=0.9);
        let cap = parameter(start, &mut rng, "cap", 0.18..=0.30, 0.01..=0.5);
        let invested = parameter(start, &mut rng, "invested", 0.75..=0.92, 0.1..=0.95);
        let clip = parameter(start, &mut rng, "clip", 0.015..=0.035, 0.001..=0.1);
        let chase = parameter(start, &mut rng, "chase", 0.015..=0.04, 0.001..=0.1);
        Self {
            execution: Execution::new(start, clip, chase),
            impressions: BTreeMap::new(),
            scale: start
                .rules
                .stocks
                .iter()
                .map(|s| s.initial_dividend.max(1) as f64)
                .collect(),
            anchors: vec![0.0; start.rules.stocks.len()],
            memory,
            amplification,
            baseline,
            cap,
            invested,
        }
    }
}

impl Bot for NewsReactor {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
        for item in &obs.news {
            // A headline follower reacts only once news reaches the public feed.
            // Earlier private ranges are delivered normally but are not researched.
            if item.forecast || !item.public {
                continue;
            }
            // Revisions replace one impression; repeated delivery never adds a new bet.
            if self
                .impressions
                .get(&item.id)
                .is_some_and(|e| item.stage <= e.stage)
            {
                continue;
            }
            let first_seen = self
                .impressions
                .get(&item.id)
                .map_or(obs.tick, |e| e.first_seen);
            if !self.impressions.contains_key(&item.id) {
                for c in &item.changes {
                    if c.stock < self.anchors.len() {
                        self.anchors[c.stock] = obs.books[c.stock].last.max(1) as f64;
                    }
                }
            }
            let changes = item
                .changes
                .iter()
                .filter(|c| c.stock < self.scale.len())
                .map(|c| {
                    let denominator = (obs.books[c.stock].dividend as f64)
                        .max(self.scale[c.stock] * 0.5)
                        .max(1.0);
                    (c.stock, c.mid() / denominator)
                })
                .collect();
            self.impressions.insert(
                item.id,
                Impression {
                    first_seen,
                    stage: item.stage,
                    changes,
                },
            );
        }
        self.impressions
            .retain(|_, e| obs.tick.saturating_sub(e.first_seen) as f64 <= self.memory * 6.0);
        let n = self.scale.len();
        if n == 0 {
            return vec![];
        }
        let mut signals = vec![0.0; n];
        for e in self.impressions.values() {
            let decay = (-(obs.tick.saturating_sub(e.first_seen) as f64) / self.memory).exp();
            for &(stock, change) in &e.changes {
                signals[stock] += change * decay;
            }
        }
        let mut weights: Vec<f64> = signals
            .iter()
            .map(|s| {
                (self.baseline / n as f64 + self.amplification * s / n as f64).clamp(0.0, self.cap)
            })
            .collect();
        let total: f64 = weights.iter().sum();
        if total > self.invested {
            for w in &mut weights {
                *w *= self.invested / total;
            }
        }
        let targets = weights
            .iter()
            .zip(&obs.books)
            .map(|(w, b)| {
                (obs.account.equity.max(0) as f64 * w / b.last.max(1) as f64).floor() as i64
            })
            .collect::<Vec<_>>();
        // The same exaggerated headline changes willingness to pay, not just
        // portfolio weights. Anchors are visible prices at a NEW news event;
        // revisions do not compound the same price jump a second time.
        let references = signals
            .iter()
            .enumerate()
            .map(|(s, signal)| {
                if self.anchors[s] == 0.0 {
                    self.anchors[s] = obs.books[s].last.max(1) as f64;
                }
                let perceived_change = (self.amplification * signal).clamp(-0.35, 0.60);
                (self.anchors[s] * (1.0 + perceived_change)).max(1.0) as i64
            })
            .collect::<Vec<_>>();
        self.execution.orders(obs, &targets, Some(&references))
    }
}
