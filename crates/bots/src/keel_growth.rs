//! Which stock, if any, is the designated grower? A logistic score over the public
//! dividend path only: projected dividend level against the initial rate, its lead over
//! the next highest stock, the drift of its stock-layer news since the divergence tick
//! and since the start, the share of those news that were increases, and the change over
//! the last 1500 ticks. Coefficients were fitted per checkpoint on 3000 generated worlds
//! (seeds 100000-102999), where the hidden labels are known; the bot never sees labels.

use bot_api::{NewsLayer, Rules, Tick};

const FIRST: Tick = 3500;
const STEP: Tick = 250;
const SAMPLE: Tick = 50;
const LOOKBACK: usize = 30;

/// [bias, L, max(L-1,0), L - max other L, stock drift since divergence, stock drift since
/// start, L change over 1500 ticks, increase share - 0.5], one row per 250 ticks from 3500.
const COEF: [[f64; 8]; 25] = [
    [
        -3.5450, 0.6589, -1.4768, 0.3835, -0.0456, 0.7864, 0.3584, 0.5366,
    ],
    [
        -3.6401, 0.8221, -1.6506, 0.4669, 0.0683, 0.8120, 0.2808, 0.4943,
    ],
    [
        -3.7694, 1.0093, -1.8565, 0.6104, 0.3212, 0.8577, 0.1729, 0.4125,
    ],
    [
        -4.0590, 1.2754, -2.0785, 0.6735, 0.4415, 0.9215, 0.1289, 0.5695,
    ],
    [
        -4.2881, 1.4713, -2.1341, 0.7681, 0.6428, 0.9627, -0.0189, 0.6736,
    ],
    [
        -4.6031, 1.7455, -2.3092, 0.8550, 0.8695, 0.9739, -0.1459, 0.8381,
    ],
    [
        -4.9503, 1.9649, -2.4079, 0.9628, 0.9650, 1.0125, -0.1019, 0.9876,
    ],
    [
        -5.3794, 2.2565, -2.6656, 1.0588, 1.0499, 1.0356, 0.0268, 1.1980,
    ],
    [
        -5.7859, 2.4868, -2.7502, 1.2276, 1.2146, 1.0403, 0.0243, 1.4952,
    ],
    [
        -6.1674, 2.6278, -2.7427, 1.3434, 1.3494, 1.0782, 0.0028, 1.8530,
    ],
    [
        -6.3872, 2.7087, -2.6559, 1.4893, 1.4117, 1.0382, -0.0348, 2.1053,
    ],
    [
        -6.6143, 2.6622, -2.4359, 1.5748, 1.4807, 1.0976, -0.1108, 2.2733,
    ],
    [
        -6.7080, 2.4706, -1.9609, 1.6191, 1.5637, 1.1104, -0.1784, 2.3279,
    ],
    [
        -6.7819, 2.3865, -1.6866, 1.5759, 1.5367, 1.1420, -0.3009, 2.3429,
    ],
    [
        -6.6657, 2.2289, -1.4866, 1.5842, 1.6510, 1.1371, -0.6993, 2.2092,
    ],
    [
        -6.5177, 2.0568, -1.2600, 1.6760, 1.7143, 1.0794, -0.9434, 2.0636,
    ],
    [
        -6.6183, 2.0514, -1.1526, 1.6378, 1.6933, 1.0834, -1.0572, 1.9563,
    ],
    [
        -6.6957, 1.8980, -0.7976, 1.6201, 1.6197, 1.0943, -1.0658, 2.0885,
    ],
    [
        -6.7134, 1.7061, -0.4332, 1.6792, 1.5977, 1.1066, -1.2213, 1.9728,
    ],
    [
        -6.6767, 1.4323, -0.0258, 1.7554, 1.5389, 1.0912, -1.1264, 1.9115,
    ],
    [
        -6.6107, 1.1482, 0.3029, 1.7582, 1.4891, 1.0654, -0.8073, 1.7846,
    ],
    [
        -6.6810, 0.9504, 0.5216, 1.7542, 1.4537, 1.0747, -0.5917, 1.7756,
    ],
    [
        -6.7222, 0.6796, 0.8500, 1.7250, 1.3928, 1.0824, -0.3114, 1.8311,
    ],
    [
        -6.7736, 0.5295, 1.0366, 1.6842, 1.3760, 1.0599, -0.2053, 1.7691,
    ],
    [
        -6.7796, 0.3776, 1.2174, 1.7566, 1.3294, 1.0406, -0.0918, 1.6371,
    ],
];

pub struct Growth {
    enabled: bool,
    initial: Vec<f64>,
    /// Stock-layer news made public after this tick count as post-divergence.
    divergence_public: Tick,
    since_div: Vec<f64>,
    since_start: Vec<f64>,
    ups: Vec<f64>,
    count: Vec<f64>,
    history: Vec<Vec<f64>>,
    next_sample: Tick,
    prob: Vec<f64>,
}

impl Growth {
    pub fn new(rules: &Rules) -> Self {
        let n = rules.stocks.len();
        let public = rules.news_stages.last().map(|s| s.0).unwrap_or(0);
        Growth {
            // The fit is for this market's growth model, ten stocks and three candidates.
            enabled: rules.dragon.as_ref().is_some_and(|d| {
                d.model == bot_api::DragonModel::Growth && d.candidates == 3 && n == 10
            }),
            initial: rules
                .stocks
                .iter()
                .map(|s| s.initial_dividend as f64)
                .collect(),
            divergence_public: rules.dragon.as_ref().map_or(0, |d| d.divergence_tick) + public,
            since_div: vec![0.0; n],
            since_start: vec![0.0; n],
            ups: vec![0.0; n],
            count: vec![0.0; n],
            history: Vec::new(),
            next_sample: 0,
            prob: vec![0.0; n],
        }
    }

    /// One public stock-layer change, with the projected dividend just before it.
    pub fn record(
        &mut self,
        layer: NewsLayer,
        public_tick: Tick,
        stock: usize,
        before: i64,
        delta: i64,
    ) {
        if layer != NewsLayer::Stock || stock >= self.initial.len() {
            return;
        }
        let after = before + delta;
        let step = ((after.max(1)) as f64 / before.max(1) as f64).ln();
        self.since_start[stock] += step;
        if public_tick > self.divergence_public {
            self.since_div[stock] += step;
            self.count[stock] += 1.0;
            if after > before {
                self.ups[stock] += 1.0;
            }
        }
    }

    /// Call every tick with the projected dividend of every stock.
    pub fn update(&mut self, tick: Tick, projected: &[i64]) {
        if tick < self.next_sample {
            return;
        }
        self.next_sample = (tick / SAMPLE + 1) * SAMPLE;
        let level: Vec<f64> = projected
            .iter()
            .zip(&self.initial)
            .map(|(k, i)| ((*k).max(1) as f64 / i.max(1.0)).ln())
            .collect();
        self.history.push(level.clone());
        if self.history.len() > LOOKBACK + 1 {
            self.history.remove(0);
        }
        if !self.enabled || tick < FIRST {
            return;
        }
        let row = (((tick - FIRST) / STEP) as usize).min(COEF.len() - 1);
        let c = &COEF[row];
        let old = (self.history.len() > LOOKBACK).then(|| &self.history[0]);
        for s in 0..level.len() {
            let other = level
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != s)
                .map(|(_, l)| *l)
                .fold(f64::NEG_INFINITY, f64::max);
            let x = [
                1.0,
                level[s],
                (level[s] - 1.0).max(0.0),
                level[s] - other,
                self.since_div[s],
                self.since_start[s],
                level[s] - old.map_or(0.0, |o| o[s]),
                (self.ups[s] + 1.0) / (self.count[s] + 2.0) - 0.5,
            ];
            let z: f64 = x.iter().zip(c).map(|(a, b)| a * b).sum();
            self.prob[s] = 1.0 / (1.0 + (-z).exp());
        }
    }

    /// Probability that each stock is the designated grower, from the last sample.
    pub fn prob(&self) -> &[f64] {
        &self.prob
    }
}
