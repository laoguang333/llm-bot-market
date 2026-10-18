//! Swift: public-tape / order-book trading, without a fundamental-value channel.
//!
//! Only execution rules are retained from Start. No dividends, news, terminal
//! hints, stock metadata, other accounts or world seed enter the signal.
use bot_api::*;

#[derive(Default)]
struct Tape {
    volume: [f64; 2],
    signed: [f64; 2],
    price: [f64; 2],
    last_tick: Option<Tick>,
}

impl Tape {
    fn update(&mut self, tick: Tick, price: f64, trades: &[Trade]) {
        let dt = self.last_tick.map_or(1, |t| tick.saturating_sub(t)).max(1) as f64;
        let mut v = 0.0;
        let mut f = 0.0;
        for t in trades {
            let q = t.qty.max(0) as f64;
            v += q;
            f += if t.aggressor == Side::Buy { q } else { -q };
        }
        for (j, horizon) in [10.0, 30.0].into_iter().enumerate() {
            let a = (-dt / horizon).exp();
            self.volume[j] = self.volume[j] * a + v;
            self.signed[j] = self.signed[j] * a + f;
            self.price[j] = if self.last_tick.is_none() {
                price
            } else {
                self.price[j] * a + price * (1.0 - a)
            };
        }
        self.last_tick = Some(tick);
    }
    fn flow(&self, i: usize) -> f64 {
        self.signed[i] / self.volume[i].max(1.0)
    }
}

pub struct Swift {
    tape: Vec<Tape>,
    tick_size: Price,
    fee_bps: i64,
    actions: usize,
    leverage: f64,
    cap: f64,
    entry: f64,
    exit: f64,
    depth_weight: f64,
    momentum_weight: f64,
    max_spread: f64,
    target: Vec<f64>,
}

impl Swift {
    pub fn new(s: &Start) -> Self {
        let n = s.shares.len();
        Self {
            tape: (0..n).map(|_| Tape::default()).collect(),
            tick_size: s.rules.tick_size.max(1),
            fee_bps: s.rules.trade_fee_bps,
            actions: s.rules.max_actions_per_tick,
            leverage: s.param("leverage", 1.5).clamp(0.0, s.rules.max_leverage),
            cap: s.param("cap", 0.35).clamp(0.0, 1.0),
            entry: s.param("entry", 0.15),
            exit: s.param("exit", -0.3),
            depth_weight: s.param("depth", 0.3),
            momentum_weight: s.param("momentum", 5.0),
            max_spread: s.param("spread", 0.012),
            target: vec![0.0; n],
        }
    }
}

impl Bot for Swift {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
        let n = self.tape.len().min(obs.books.len());
        let mut out = Vec::new();
        let eq = obs.account.equity.max(0) as f64;
        let mut budget =
            (eq * self.leverage - obs.account.gross as f64 - obs.account.reserved_buy as f64)
                .max(0.0)
                .min(obs.account.buying_power.max(0) as f64);
        for k in 0..n {
            let s = (k + obs.tick as usize) % n;
            let b = &obs.books[s];
            let mid = b.mid().unwrap_or(b.last).max(1) as f64;
            let tape = &mut self.tape[s];
            tape.update(obs.tick, mid, obs.trades.get(s).map_or(&[], Vec::as_slice));
            let bd: f64 = b
                .bids
                .iter()
                .filter(|l| l.price as f64 >= mid * 0.98)
                .map(|l| l.qty.max(0) as f64)
                .sum();
            let ad: f64 = b
                .asks
                .iter()
                .filter(|l| l.price as f64 <= mid * 1.02)
                .map(|l| l.qty.max(0) as f64)
                .sum();
            let depth = (bd - ad) / (bd + ad).max(1.0);
            let mom = (mid / tape.price[1].max(1.0) - 1.0).clamp(-0.1, 0.1);
            // Flow is signed executed volume, depth is displayed liquidity within
            // 2% of mid, momentum is mid relative to its trailing 30-tick EMA.
            let score = 0.7 * tape.flow(0)
                + 0.3 * tape.flow(1)
                + self.depth_weight * depth
                + self.momentum_weight * mom;
            // Hysteresis prevents repeatedly crossing the spread around a single
            // entry threshold. The position is re-sized only within the cap.
            if score > self.entry && obs.tick >= 30 && tape.volume[0] > 10.0 {
                self.target[s] = self.cap;
            } else if score < self.exit {
                self.target[s] = 0.0;
            }
            if obs.tick.saturating_add(20) >= obs.ticks_total {
                self.target[s] = 0.0;
            }
            let target = (eq * self.target[s] / mid) as Qty;
            let have = obs.account.shares[s];
            if out.len() >= self.actions {
                continue;
            }
            let own: Vec<_> = obs.open_orders.iter().filter(|o| o.stock == s).collect();
            if !own.is_empty() {
                if out.len() + own.len() <= self.actions {
                    out.extend(own.iter().map(|o| Action::Cancel {
                        order_id: o.order_id,
                    }));
                }
                continue;
            }
            if have > target {
                if let Some(bb) = b.best_bid() {
                    let q = (have - target).min(obs.account.sellable[s]);
                    if q > 0 {
                        // IOC at the best price never walks through worse levels.
                        out.push(Action::ioc(s, Side::Sell, bb, q));
                    }
                }
            } else if target > have {
                if let (Some(bb), Some(ba)) = (b.best_bid(), b.best_ask()) {
                    if (ba - bb) as f64 / mid > self.max_spread {
                        continue;
                    }
                    let cost = ba + (ba * self.fee_bps + 9999) / 10000;
                    let q = (target - have).min((budget / cost.max(self.tick_size) as f64) as Qty);
                    if q > 0 {
                        out.push(Action::ioc(s, Side::Buy, ba, q));
                        budget -= (q * cost) as f64;
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
#[path = "swift_tests.rs"]
mod tests;
