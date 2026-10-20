//! A simple, inventory-bounded price grid. No news, dividends or value model.
use crate::crowd_execution::parameter;
use bot_api::{Action, Bot, Observation, Price, Rules, Side, Start, Tick};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

pub struct GridV2 {
    rules: Rules,
    centers: Vec<f64>,
    previous_tick: Option<Tick>,
    cursor: usize,
    gap: f64,
    memory: f64,
    invested: f64,
    cap: f64,
    clip: f64,
    lifetime: Tick,
}

impl GridV2 {
    pub fn new(start: &Start) -> Self {
        let mut rng = ChaCha8Rng::seed_from_u64(start.seed);
        let gap = parameter(start, &mut rng, "gap", 0.003..=0.007, 0.001..=0.05);
        let memory = parameter(start, &mut rng, "memory", 30.0..=120.0, 1.0..=1000.0);
        let invested = parameter(start, &mut rng, "invested", 0.65..=0.80, 0.1..=0.9);
        let cap = parameter(start, &mut rng, "cap", 0.12..=0.18, 0.01..=0.3);
        let clip = parameter(start, &mut rng, "clip", 0.015..=0.03, 0.001..=0.05);
        let lifetime =
            parameter(start, &mut rng, "lifetime", 5.0..=12.0, 1.0..=50.0).round() as Tick;
        Self {
            rules: start.rules.clone(),
            centers: vec![0.0; start.rules.stocks.len()],
            previous_tick: None,
            cursor: start.account_index,
            gap,
            memory,
            invested,
            cap,
            clip,
            lifetime,
        }
    }
}

impl Bot for GridV2 {
    fn on_tick(&mut self, obs: &Observation) -> Vec<Action> {
        let n = self.centers.len();
        if n == 0 || obs.account.equity <= 0 {
            return if obs.open_orders.is_empty() {
                vec![]
            } else {
                vec![Action::CancelAll { stock: None }]
            };
        }
        let elapsed = self.previous_tick.map_or(0, |t| obs.tick.saturating_sub(t));
        self.previous_tick = Some(obs.tick);
        let decay = (-(elapsed as f64) / self.memory).exp();
        let equity = obs.account.equity as f64;
        let step = self.rules.tick_size.max(1);
        let mut cash = obs
            .account
            .cash
            .saturating_sub(obs.account.reserved_buy)
            .max(0);
        let mut power = obs.account.buying_power.max(0);
        let mut capacity = (equity * self.invested
            - obs.account.gross.max(0) as f64
            - obs.account.reserved_buy.max(0) as f64)
            .max(0.0) as i64;
        let mut actions = Vec::new();
        let mut new_orders = 0;
        for offset in 0..n {
            let s = (self.cursor + offset) % n;
            let book = &obs.books[s];
            let mark = book.mark.max(1);
            let last = book.last.max(1) as f64;
            if self.centers[s] <= 0.0 {
                self.centers[s] = last;
            }
            self.centers[s] = self.centers[s] * decay + last * (1.0 - decay);
            if actions.len() >= self.rules.max_actions_per_tick {
                continue;
            }
            let weight = obs.account.shares[s] as f64 * mark as f64 / equity;
            // Overstock lowers both grid prices; understock raises them. Bounded
            // skew encourages inventory circulation without any valuation signal.
            let skew = ((self.invested / n as f64 - weight) * 0.3).clamp(-0.02, 0.02);
            let center = self.centers[s] * (1.0 + skew);
            let mut bid =
                ((center * (1.0 - self.gap) / step as f64).floor() as Price * step).max(step);
            let mut ask =
                ((center * (1.0 + self.gap) / step as f64).ceil() as Price * step).max(step);
            // Do not deliberately cross the currently observed opposing quote.
            if let Some(p) = book.best_ask() {
                bid = bid.min(p.saturating_sub(step));
            }
            if let Some(p) = book.best_bid() {
                ask = ask.max(p.saturating_add(step));
            }
            ask = ask.max(bid.saturating_add(step));
            let existing: Vec<_> = obs.open_orders.iter().filter(|o| o.stock == s).collect();
            if !existing.is_empty() {
                let cancel = existing.iter().any(|o| {
                    obs.tick.saturating_sub(o.placed_tick) >= self.lifetime
                        || match o.side {
                            Side::Buy => weight >= self.cap || o.price > bid,
                            Side::Sell => o.price < ask,
                        }
                });
                if cancel {
                    actions.push(Action::CancelAll { stock: Some(s) });
                }
                continue;
            }
            for side in [Side::Sell, Side::Buy] {
                if actions.len() >= self.rules.max_actions_per_tick
                    || obs.open_orders.len() + new_orders >= self.rules.max_open_orders
                {
                    break;
                }
                let price = if side == Side::Buy { bid } else { ask };
                if price < step {
                    continue;
                }
                let clip_price = price.max(mark);
                let mut qty = (equity * self.clip / clip_price as f64).floor() as i64;
                match side {
                    Side::Sell => qty = qty.min(obs.account.sellable[s]),
                    Side::Buy => {
                        let position_room = ((equity * self.cap
                            - obs.account.shares[s] as f64 * mark as f64)
                            .max(0.0)
                            / price as f64)
                            .floor() as i64;
                        let reserve = self.rules.buy_reserve_per_share(price).max(1);
                        qty = qty
                            .min(position_room)
                            .min(capacity / reserve)
                            .min(cash / reserve)
                            .min(power / reserve);
                        capacity -= qty * reserve;
                        cash -= qty * reserve;
                        power -= qty * reserve;
                    }
                }
                if qty > 0 {
                    actions.push(Action::limit(s, side, price, qty));
                    new_orders += 1;
                }
            }
        }
        self.cursor = (self.cursor + 1) % n;
        actions
    }
}
