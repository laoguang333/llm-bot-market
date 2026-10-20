//! Execution for the two v19 behavioural crowds; signals live in separate modules.
use bot_api::{Action, Observation, Price, Qty, Rules, Side, Start};
use rand::Rng;
use rand_chacha::ChaCha8Rng;

pub(crate) fn parameter(
    start: &Start,
    rng: &mut ChaCha8Rng,
    name: &str,
    range: std::ops::RangeInclusive<f64>,
    bounds: std::ops::RangeInclusive<f64>,
) -> f64 {
    let default = rng.gen_range(range);
    let value = start.param(name, default);
    if value.is_finite() {
        value.clamp(*bounds.start(), *bounds.end())
    } else {
        default
    }
}

pub(crate) struct Execution {
    rules: Rules,
    pub clip: f64,
    pub chase: f64,
    cursor: usize,
}

impl Execution {
    pub fn new(start: &Start, clip: f64, chase: f64) -> Self {
        Self {
            rules: start.rules.clone(),
            clip,
            chase,
            cursor: start.account_index,
        }
    }

    /// No borrowing, shorting, or spending unconfirmed sell proceeds.
    /// Unfilled intent may rest, with bounded prices and at most 20 ticks per quote.
    /// `references` optionally bounds execution against the ORIGINAL intent price.
    pub fn orders(
        &mut self,
        obs: &Observation,
        targets: &[Qty],
        references: Option<&[Price]>,
    ) -> Vec<Action> {
        let n = targets.len();
        if n == 0 || obs.account.equity <= 0 {
            return if obs.open_orders.is_empty() {
                vec![]
            } else {
                vec![Action::CancelAll { stock: None }]
            };
        }
        let mut actions = Vec::new();
        let mut cash = obs
            .account
            .cash
            .saturating_sub(obs.account.reserved_buy)
            .max(0);
        let mut power = obs.account.buying_power.max(0);
        let step = self.rules.tick_size.max(1);
        for offset in 0..n {
            if actions.len() >= self.rules.max_actions_per_tick {
                break;
            }
            let s = (self.cursor + offset) % n;
            let book = &obs.books[s];
            let difference = targets[s].saturating_sub(obs.account.shares[s]);
            let reference = references.map_or(book.last, |p| p[s]);
            let existing: Vec<_> = obs.open_orders.iter().filter(|o| o.stock == s).collect();
            if difference == 0 || reference <= 0 {
                if !existing.is_empty() {
                    actions.push(Action::CancelAll { stock: Some(s) });
                }
                continue;
            }
            let side = if difference > 0 {
                Side::Buy
            } else {
                Side::Sell
            };
            let raw = reference as f64
                * if side == Side::Buy {
                    1.0 + self.chase
                } else {
                    1.0 - self.chase
                };
            let limit = if side == Side::Buy {
                (raw / step as f64).floor() as Price * step
            } else {
                (raw / step as f64).ceil() as Price * step
            }
            .max(step);
            if !existing.is_empty() {
                let keep = existing.len() == 1
                    && existing.iter().all(|o| {
                        o.side == side
                            && o.remaining <= difference.saturating_abs()
                            && obs.tick.saturating_sub(o.placed_tick) < 20
                            && if side == Side::Buy {
                                o.price <= limit
                            } else {
                                o.price >= limit
                            }
                    });
                if !keep {
                    actions.push(Action::CancelAll { stock: Some(s) });
                }
                // Released reservations are only used after the next observation.
                continue;
            }
            let available_quote = match side {
                Side::Buy => book.best_ask().is_some_and(|ask| ask <= limit),
                Side::Sell => book.best_bid().is_some_and(|bid| bid >= limit),
            };
            if !available_quote
                && obs.open_orders.len()
                    + actions
                        .iter()
                        .filter(|a| {
                            matches!(
                                a,
                                Action::Limit {
                                    tif: bot_api::Tif::Gtc,
                                    ..
                                }
                            )
                        })
                        .count()
                    >= self.rules.max_open_orders
            {
                continue;
            }
            let price = if available_quote {
                limit
            } else {
                match side {
                    Side::Buy => book
                        .best_bid()
                        .map_or(limit, |p| p.saturating_add(step).min(limit)),
                    Side::Sell => book
                        .best_ask()
                        .map_or(limit, |p| p.saturating_sub(step).max(limit)),
                }
            };
            // A very low sell limit must not inflate the amount of inventory sold.
            let clip_price = if side == Side::Sell {
                price
                    .max(book.best_bid().unwrap_or(book.last))
                    .max(book.mark)
            } else {
                price
            };
            let clip = (obs.account.equity as f64 * self.clip / clip_price as f64).floor() as Qty;
            let mut qty = difference.saturating_abs().min(clip);
            // Ignore tiny adjustments, but allow a final one-share exit.
            if targets[s] != 0
                && difference.saturating_abs() as f64 * (price as f64)
                    < obs.account.equity as f64 * 0.0025
            {
                continue;
            }
            match side {
                Side::Buy => {
                    let reserve = self.rules.buy_reserve_per_share(price).max(1);
                    qty = qty.min(cash / reserve).min(power / reserve);
                    cash -= qty * reserve;
                    power -= qty * reserve;
                }
                Side::Sell => qty = qty.min(obs.account.sellable[s]),
            }
            if qty > 0 {
                actions.push(if available_quote {
                    Action::ioc(s, side, price, qty)
                } else {
                    Action::limit(s, side, price, qty)
                });
            }
        }
        self.cursor = (self.cursor + 1) % n;
        actions
    }
}
