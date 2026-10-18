//! Read-only reconstruction of frozen Swift. Every returned order is compared
//! with the real bot, never submitted. A mismatch is prominently reported.
use bot_api::*;
use serde::Serialize;

#[derive(Default)]
struct Tape {
    volume: [f64; 2],
    signed: [f64; 2],
    price: [f64; 2],
    last: Option<Tick>,
}

#[derive(Clone, Default, Serialize)]
pub struct Signal {
    pub flow10: f64,
    pub flow30: f64,
    pub depth: f64,
    pub momentum: f64,
    pub score: f64,
    pub target_fraction: f64,
    pub target_shares: Qty,
    pub volume10: f64,
    pub ema30: f64,
    pub budget: f64,
    pub signal_reason: &'static str,
    pub execution_reason: &'static str,
}

pub struct Probe {
    start: Start,
    tape: Vec<Tape>,
    target: Vec<f64>,
    exiting: Vec<bool>,
}

impl Probe {
    pub fn new(s: &Start) -> Self {
        Self {
            start: s.clone(),
            tape: (0..s.shares.len()).map(|_| Tape::default()).collect(),
            target: vec![0.0; s.shares.len()],
            exiting: vec![false; s.shares.len()],
        }
    }
    pub fn evaluate(&mut self, o: &Observation) -> (Vec<Signal>, Vec<Action>) {
        let r = &self.start.rules;
        let n = self.tape.len().min(o.books.len());
        let eq = o.account.equity.max(0) as f64;
        let leverage = self.start.param("leverage", 1.5).clamp(0.0, r.max_leverage);
        let cap = self.start.param("cap", 0.35).clamp(0.0, 1.0);
        let mut budget = (eq * leverage - o.account.gross as f64 - o.account.reserved_buy as f64)
            .max(0.0)
            .min(o.account.buying_power.max(0) as f64);
        let mut signals = vec![Signal::default(); n];
        let mut actions = Vec::new();
        for k in 0..n {
            let s = (k + o.tick as usize) % n;
            let b = &o.books[s];
            let mid = b.mid().unwrap_or(b.last).max(1) as f64;
            let t = &mut self.tape[s];
            let dt = t.last.map_or(1, |x| o.tick.saturating_sub(x)).max(1) as f64;
            let (mut v, mut signed) = (0.0, 0.0);
            for trade in o.trades.get(s).map_or(&[][..], Vec::as_slice) {
                let q = trade.qty.max(0) as f64;
                v += q;
                signed += if trade.aggressor == Side::Buy { q } else { -q };
            }
            for (j, h) in [10.0, 30.0].into_iter().enumerate() {
                let a = (-dt / h).exp();
                t.volume[j] = t.volume[j] * a + v;
                t.signed[j] = t.signed[j] * a + signed;
                t.price[j] = if t.last.is_none() {
                    mid
                } else {
                    t.price[j] * a + mid * (1.0 - a)
                };
            }
            t.last = Some(o.tick);
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
            let momentum = (mid / t.price[1].max(1.0) - 1.0).clamp(-0.1, 0.1);
            let flow10 = t.signed[0] / t.volume[0].max(1.0);
            let flow30 = t.signed[1] / t.volume[1].max(1.0);
            let score = 0.7 * flow10
                + 0.3 * flow30
                + self.start.param("depth", 0.3) * depth
                + self.start.param("momentum", 5.0) * momentum;
            let mut reason = "hysteresis";
            if score > self.start.param("entry", 0.15) && o.tick >= 30 && t.volume[0] > 10.0 {
                self.target[s] = cap;
                reason = "entry";
            } else if score < self.start.param("exit", -0.3) {
                self.target[s] = 0.0;
                reason = "exit";
            } else if o.tick < 30 || t.volume[0] <= 10.0 {
                reason = "warmup";
            }
            if o.tick.saturating_add(20) >= o.ticks_total {
                self.target[s] = 0.0;
                reason = "end_exit";
            }
            let mut target = (eq * self.target[s] / mid) as Qty;
            let have = o.account.shares[s];
            let mut execution_reason = "at_target";
            if self.start.param("single_entry", 0.0) > 0.0 {
                if have == 0 {
                    self.exiting[s] = false;
                } else if self.target[s] == 0.0 {
                    self.exiting[s] = true;
                }
                if self.exiting[s] {
                    if self.target[s] != 0.0 {
                        reason = "finish_exit";
                    }
                    self.target[s] = 0.0;
                    target = 0;
                } else if have > 0 {
                    target = have;
                    execution_reason = "single_entry_hold";
                }
            }
            let p = &mut signals[s];
            *p = Signal {
                flow10,
                flow30,
                depth,
                momentum,
                score,
                target_fraction: self.target[s],
                target_shares: target,
                volume10: t.volume[0],
                ema30: t.price[1],
                budget,
                signal_reason: reason,
                execution_reason,
            };
            if actions.len() >= r.max_actions_per_tick {
                p.execution_reason = "action_limit";
                continue;
            }
            let own: Vec<_> = o.open_orders.iter().filter(|x| x.stock == s).collect();
            if !own.is_empty() {
                p.execution_reason = "cancel_existing";
                if actions.len() + own.len() <= r.max_actions_per_tick {
                    actions.extend(own.iter().map(|x| Action::Cancel {
                        order_id: x.order_id,
                    }));
                } else {
                    p.execution_reason = "action_limit";
                }
                continue;
            }
            if have > target {
                p.execution_reason = "no_bid";
                if let Some(bb) = b.best_bid() {
                    let q = (have - target).min(o.account.sellable[s]);
                    p.execution_reason = "reserved_shares";
                    if q > 0 {
                        actions.push(Action::ioc(s, Side::Sell, bb, q));
                        p.execution_reason = "sell";
                    }
                }
            } else if target > have {
                p.execution_reason = "one_sided_book";
                if let (Some(bb), Some(ba)) = (b.best_bid(), b.best_ask()) {
                    if (ba - bb) as f64 / mid > self.start.param("spread", 0.012) {
                        p.execution_reason = "wide_spread";
                        continue;
                    }
                    let cost = ba + (ba * r.trade_fee_bps + 9999) / 10000;
                    let q =
                        (target - have).min((budget / cost.max(r.tick_size.max(1)) as f64) as Qty);
                    p.execution_reason = "no_budget";
                    if q > 0 {
                        actions.push(Action::ioc(s, Side::Buy, ba, q));
                        budget -= (q * cost) as f64;
                        p.execution_reason = "buy";
                    }
                }
            }
        }
        (signals, actions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reconstructed_orders_match_frozen_swift_across_execution_gates() {
        let cfg = market::config::Config::load(std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scenarios/baseline.toml"
        )))
        .unwrap();
        for profile in cfg.bots.iter().filter(|b| b.bot == "swift") {
            for action_limit in [1, 20] {
                let mut rules = cfg.rules(&vec![1000; cfg.stocks.len()]);
                rules.max_actions_per_tick = action_limit;
                let start = Start {
                    account: "probe".into(),
                    owner: "probe".into(),
                    account_index: 0,
                    n_accounts: 1,
                    seed: 1,
                    cash: 2_000_000,
                    shares: vec![20; 10],
                    rules,
                    params: profile.params.clone(),
                };
                let mut real = bots::swift::Swift::new(&start);
                let mut probe = Probe::new(&start);
                for tick in 0..200 {
                    let mut o = Observation {
                        tick,
                        ticks_total: 200,
                        reference_tick: tick,
                        season: Season {
                            turbulent: false,
                            rate_mult: 1.0,
                            size_mult: 1.0,
                        },
                        account: AccountView {
                            cash: 100000,
                            equity: 300000,
                            gross: 100000,
                            buying_power: if tick % 11 == 0 { 0 } else { 200000 },
                            shares: vec![(tick % 21) as i64; 10],
                            sellable: vec![(tick % 15) as i64; 10],
                            ..Default::default()
                        },
                        books: vec![
                            BookView {
                                bids: vec![Level {
                                    price: 9999,
                                    qty: 10
                                }],
                                asks: vec![Level {
                                    price: 10001,
                                    qty: 20
                                }],
                                last: 10000,
                                mark: 10000,
                                dividend: 0
                            };
                            10
                        ],
                        trades: vec![
                            vec![Trade {
                                tick,
                                price: 10000,
                                qty: 50,
                                aggressor: if tick % 40 < 20 {
                                    Side::Buy
                                } else {
                                    Side::Sell
                                }
                            }];
                            10
                        ],
                        open_orders: vec![],
                        skipped: 0,
                        fills: vec![],
                        acks: vec![],
                        rejects: vec![],
                        news: vec![],
                        season_changes: vec![],
                        income: vec![],
                        terminal_hints: vec![],
                    };
                    if tick % 13 == 0 {
                        o.books[0].asks[0].price = 12000;
                    }
                    if tick % 17 == 0 {
                        o.books[1].bids.clear();
                    }
                    if tick % 19 == 0 {
                        o.books[2].asks.clear();
                    }
                    if tick % 7 == 0 {
                        o.open_orders.push(OpenOrder {
                            stock: 3,
                            order_id: 55,
                            side: Side::Buy,
                            price: 9990,
                            remaining: 1,
                            placed_tick: tick,
                        });
                    }
                    assert_eq!(
                        probe.evaluate(&o).1,
                        real.on_tick(&o),
                        "tick {tick}, actions {action_limit}"
                    );
                }
            }
        }
    }
}
