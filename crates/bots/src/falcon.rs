//! Falcon: public-data adversary for the frozen Swift, with no account identity channel.
//! Modes are distinct research hypotheses, not online switches selected by world seed.
use bot_api::*;

#[derive(Default)]
struct Signal {
    vol: [f64; 2],
    signed: [f64; 2],
    ema: f64,
    last: Option<Tick>,
    active: bool,
}

impl Signal {
    fn update(&mut self, tick: Tick, b: &BookView, trades: &[Trade]) -> f64 {
        let mid = b.mid().unwrap_or(b.last).max(1) as f64;
        let dt = self.last.map_or(1, |t| tick.saturating_sub(t)).max(1) as f64;
        let v: f64 = trades.iter().map(|t| t.qty.max(0) as f64).sum();
        let signed: f64 = trades
            .iter()
            .map(|t| t.qty.max(0) as f64 * if t.aggressor == Side::Buy { 1.0 } else { -1.0 })
            .sum();
        for (i, h) in [10.0, 30.0].into_iter().enumerate() {
            let a = (-dt / h).exp();
            self.vol[i] = self.vol[i] * a + v;
            self.signed[i] = self.signed[i] * a + signed;
        }
        let a = (-dt / 30.0).exp();
        self.ema = if self.last.is_none() {
            mid
        } else {
            self.ema * a + mid * (1.0 - a)
        };
        self.last = Some(tick);
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
        let score = 0.7 * self.signed[0] / self.vol[0].max(1.0)
            + 0.3 * self.signed[1] / self.vol[1].max(1.0)
            + 0.3 * (bd - ad) / (bd + ad).max(1.0)
            + 5.0 * (mid / self.ema.max(1.0) - 1.0).clamp(-0.1, 0.1);
        if tick >= 30 && self.vol[0] > 10.0 && score > 0.15 {
            self.active = true;
        } else if score < -0.3 {
            self.active = false;
        }
        score
    }
}

pub struct Falcon {
    signals: Vec<Signal>,
    enabled: Vec<bool>,
    pulse: Vec<Qty>,
    mode: i32,
    leverage: f64,
    cap: f64,
    entry: f64,
    exit: f64,
    actions: usize,
    tick_size: Price,
    fee: i64,
}

impl Falcon {
    pub fn new(s: &Start) -> Self {
        Self {
            signals: (0..s.shares.len()).map(|_| Signal::default()).collect(),
            enabled: vec![false; s.shares.len()],
            pulse: s.shares.iter().map(|q| (q / 5).max(1)).collect(),
            mode: s.param("mode", 0.0) as i32,
            leverage: s.param("leverage", 1.0).clamp(0.0, s.rules.max_leverage),
            cap: s.param("cap", 0.25).clamp(0.0, 1.0),
            entry: s.param("entry", 0.0),
            exit: s.param("exit", -0.05),
            actions: s.rules.max_actions_per_tick,
            tick_size: s.rules.tick_size.max(1),
            fee: s.rules.trade_fee_bps,
        }
    }

    fn price(&self, p: f64, up: bool) -> Price {
        let units = p / self.tick_size as f64;
        (if up { units.ceil() } else { units.floor() } as i64).max(1) * self.tick_size
    }
}

impl Bot for Falcon {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
        let n = self.signals.len().min(obs.books.len());
        let mut scores = vec![0.0; n];
        for (s, score) in scores.iter_mut().enumerate() {
            *score = self.signals[s].update(
                obs.tick,
                &obs.books[s],
                obs.trades.get(s).map_or(&[], Vec::as_slice),
            );
        }
        let eq = obs.account.equity.max(0) as f64;
        let mut budget =
            (eq * self.leverage - obs.account.gross as f64 - obs.account.reserved_buy as f64)
                .max(0.0)
                .min(obs.account.buying_power.max(0) as f64);
        let mut out = Vec::new();
        for k in 0..n {
            let s = (k + obs.tick as usize) % n;
            let b = &obs.books[s];
            let score = scores[s];
            let mid = b.mid().unwrap_or(b.last).max(1) as f64;
            let ready = obs.tick >= 30 && self.signals[s].vol[0] > 10.0;
            let closing = obs.tick.saturating_add(40) >= obs.ticks_total;
            match self.mode {
                1 => {
                    // Buy while the modeled target exits; distribute into its buy signal.
                    if ready && score < -0.3 {
                        self.enabled[s] = true;
                    } else if score > 0.15 {
                        self.enabled[s] = false;
                    }
                }
                4 => {
                    self.enabled[s] = false;
                } // Same-wealth cash control.
                5 => {
                    self.enabled[s] = true;
                } // Signal-blind constant-exposure control.
                6..=9 => {
                    self.enabled[s] = true;
                }
                _ => {
                    // Earlier entry/exit than Swift; no claim that target trades next.
                    if ready && score > self.entry {
                        self.enabled[s] = true;
                    } else if score < self.exit {
                        self.enabled[s] = false;
                    }
                }
            }
            if closing {
                self.enabled[s] = false;
            }
            let target = if self.enabled[s] {
                (eq * self.cap / mid) as Qty
            } else {
                0
            };
            let have = obs.account.shares[s];
            let sellable = obs.account.sellable[s];
            let own: Vec<_> = obs.open_orders.iter().filter(|o| o.stock == s).collect();
            if !own.is_empty() {
                // Keep a quote briefly, but cancel immediately on a changed direction.
                let stale = own
                    .iter()
                    .any(|o| obs.tick.saturating_sub(o.placed_tick) >= 3)
                    || closing
                    || !self.enabled[s];
                if stale && out.len() < self.actions {
                    out.push(Action::CancelAll { stock: Some(s) });
                }
                continue; // Never place against our own resting order, nor reuse its reserve.
            }
            if out.len() >= self.actions {
                continue;
            }
            let (Some(bb), Some(ba)) = (b.best_bid(), b.best_ask()) else {
                continue;
            };
            if closing || !self.enabled[s] || have > target {
                let q = if closing || !self.enabled[s] {
                    sellable
                } else {
                    (have - target).min(sellable)
                };
                if q > 0 {
                    out.push(Action::ioc(s, Side::Sell, bb, q));
                }
                continue;
            }
            if (ba - bb) as f64 / mid > 0.012 {
                continue;
            }
            if (6..=9).contains(&self.mode) {
                // Inventory-limited relays test feedback rather than carrying a
                // full contrarian position. The target's state is only inferred.
                let buy = match self.mode {
                    7 => (obs.tick / 20).is_multiple_of(2), // Signal-blind pulse control.
                    8 => score < 0.15, // Maintain positive flow, supply passively above entry.
                    _ => !self.signals[s].active,
                };
                let pulse = self.pulse[s] * if self.mode == 9 { 5 } else { 1 };
                if buy {
                    let price = if self.mode == 9 {
                        self.price(ba as f64 * 1.005, true)
                    } else {
                        ba
                    };
                    let cost = price + (price * self.fee + 9999) / 10000;
                    let q = pulse
                        .min((target - have).max(0))
                        .min((budget / cost as f64) as Qty);
                    if q > 0 {
                        out.push(Action::ioc(s, Side::Buy, price, q));
                        budget -= (q * cost) as f64;
                    }
                } else {
                    let q = pulse.min(sellable);
                    if q > 0 {
                        let price = if self.mode == 9 {
                            self.price(bb as f64 * 0.995, false)
                        } else {
                            bb
                        };
                        out.push(if self.mode == 8 {
                            Action::limit(s, Side::Sell, ba, q)
                        } else {
                            Action::ioc(s, Side::Sell, price, q)
                        });
                    }
                }
                continue;
            }
            if (self.mode == 2 || self.mode == 3) && score > 0.15 && sellable > 0 {
                // Supply the side Swift is predicted to consume; all quotes are funded.
                let ask = self.price(ba as f64, true);
                out.push(Action::limit(s, Side::Sell, ask, sellable));
                if self.mode == 3 && out.len() < self.actions {
                    // A real bid within Swift's 2% depth window can alter its depth input.
                    // It can be filled by anyone, carrying inventory and adverse-selection risk.
                    let bid = self.price((mid * 0.99).min(bb as f64), false);
                    let cost = bid + (bid * self.fee + 9999) / 10000;
                    let q =
                        ((eq * self.cap / cost as f64) as Qty).min((budget / cost as f64) as Qty);
                    if q > 0 {
                        out.push(Action::limit(s, Side::Buy, bid, q));
                        budget -= (q * cost) as f64;
                    }
                }
                continue;
            }
            if target > have {
                let cost = ba + (ba * self.fee + 9999) / 10000;
                let q = (target - have).min((budget / cost.max(1) as f64) as Qty);
                if q > 0 {
                    out.push(Action::ioc(s, Side::Buy, ba, q));
                    budget -= (q * cost) as f64;
                }
            }
        }
        out
    }
}

#[cfg(test)]
#[path = "falcon_tests.rs"]
mod tests;
