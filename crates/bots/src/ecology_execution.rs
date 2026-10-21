//! Cash-only execution for the v21 research strategies. Reservations are not
//! released optimistically: cancel first and wait for a fresh account snapshot.
use bot_api::*;

#[derive(Clone, Copy, Default)]
pub(crate) struct Quote {
    pub bid: Option<(Price, Qty)>,
    pub ask: Option<(Price, Qty)>,
}

pub(crate) fn execute(obs: &Observation, rules: &Rules, quotes: &[Quote]) -> Vec<Action> {
    let mut actions = Vec::new();
    let mut blocked = vec![[false; 2]; quotes.len()];
    for order in &obs.open_orders {
        let side = usize::from(order.side == Side::Sell);
        blocked[order.stock][side] = true;
        let desired = if side == 0 {
            quotes[order.stock].bid
        } else {
            quotes[order.stock].ask
        };
        let keep = desired.is_some_and(|(p, q)| {
            (p - order.price).abs() as f64 <= 0.004 * order.price as f64
                && order.remaining <= q.saturating_mul(2)
                && obs.tick.saturating_sub(order.placed_tick) < 35
        });
        if !keep && actions.len() < rules.max_actions_per_tick {
            actions.push(Action::Cancel {
                order_id: order.order_id,
            });
        }
    }
    // Leave room for idle cash charges while fills are in flight. Never rely on
    // sale proceeds from this call or on margin buying power.
    let cushion = (obs.account.equity.max(0) as f64 * 0.02) as Money;
    let mut cash = (obs.account.cash - obs.account.reserved_buy - cushion)
        .max(0)
        .min(obs.account.buying_power.max(0));
    let mut slots = rules.max_open_orders.saturating_sub(obs.open_orders.len());
    for offset in 0..quotes.len() {
        let s = (offset + obs.tick as usize / 5) % quotes.len();
        for side in [Side::Sell, Side::Buy] {
            if actions.len() >= rules.max_actions_per_tick || slots == 0 {
                return actions;
            }
            if blocked[s][usize::from(side == Side::Sell)] {
                continue;
            }
            let desired = if side == Side::Buy {
                quotes[s].bid
            } else {
                quotes[s].ask
            };
            let Some((p, q)) = desired else {
                continue;
            };
            if p < rules.tick_size || p % rules.tick_size != 0 || q <= 0 {
                continue;
            }
            let q = if side == Side::Buy {
                q.min(cash / rules.buy_reserve_per_share(p))
            } else {
                q.min(obs.account.sellable[s])
            };
            if q <= 0 {
                continue;
            }
            // Never cross another own resting order. Both new quotes are also
            // checked against one another, regardless of submission order.
            let crosses = obs.open_orders.iter().any(|o| {
                o.stock == s
                    && o.side != side
                    && if side == Side::Buy {
                        p >= o.price
                    } else {
                        p <= o.price
                    }
            });
            let crosses_new = match (quotes[s].bid, quotes[s].ask) {
                (Some((b, _)), Some((a, _))) => b >= a,
                _ => false,
            };
            if crosses || crosses_new {
                continue;
            }
            if side == Side::Buy {
                cash -= q * rules.buy_reserve_per_share(p);
            }
            actions.push(Action::limit(s, side, p, q));
            slots -= 1;
        }
    }
    actions
}

/// Incremental contract income of buying / selling a clip with the currently
/// held partner inventory. Respects units and active windows; no terminal combo.
pub(crate) fn combo_value(r: &Rules, obs: &Observation, stock: usize, delta: Qty) -> f64 {
    if delta == 0 {
        return 0.0;
    }
    let mut after = obs.account.shares.clone();
    after[stock] = (after[stock] + delta).max(0);
    r.combos
        .iter()
        .filter(|c| c.a == stock || c.b == stock)
        .map(|c| {
            let from = obs.tick.max(c.start_tick.saturating_sub(1));
            let until = c.end_tick.min(r.ticks_total.saturating_sub(1));
            let payments = if from <= until {
                (until / r.combo_period).saturating_sub(from / r.combo_period)
            } else {
                0
            };
            (c.sets(&after) - c.sets(&obs.account.shares)) as f64 * c.bonus as f64
                / r.combo_scale as f64
                * payments as f64
                / delta as f64
        })
        .sum()
}

/// Avoid initiating a negative contract through two simultaneous new purchases.
pub(crate) fn negative_pair_blocked(r: &Rules, obs: &Observation, stock: usize) -> bool {
    r.combos.iter().any(|c| {
        c.bonus < 0 && c.end_tick > obs.tick && (c.a == stock || c.b == stock) && {
            let other = if c.a == stock { c.b } else { c.a };
            obs.account.shares[other] > 0 || stock > other
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::news_test_support::{observation, start};

    #[test]
    fn total_buys_respect_cash_and_cancel_does_not_release_a_reservation() {
        let r = start().rules;
        let mut obs = observation(100, true);
        obs.account.cash = 100_000;
        obs.account.reserved_buy = 20_000;
        let quotes = vec![
            Quote {
                bid: Some((10_000, 100)),
                ask: Some((11_000, 100))
            };
            2
        ];
        let actions = execute(&obs, &r, &quotes);
        let spent: Money = actions
            .iter()
            .filter_map(|a| match a {
                Action::Limit {
                    side: Side::Buy,
                    price,
                    qty,
                    ..
                } => Some(r.buy_reserve_per_share(*price) * qty),
                _ => None,
            })
            .sum();
        assert!(spent > 0 && spent <= 32_000);
        assert!(actions
            .iter()
            .all(|a| !matches!(a, Action::Limit {side:Side::Sell,qty,..} if *qty>20)));
        obs.open_orders.push(OpenOrder {
            order_id: 1,
            stock: 0,
            side: Side::Buy,
            price: 5_000,
            remaining: 4,
            placed_tick: 0,
        });
        let actions = execute(&obs, &r, &quotes);
        assert!(actions.contains(&Action::Cancel { order_id: 1 }));
        assert!(!actions.iter().any(|a| matches!(
            a,
            Action::Limit {
                stock: 0,
                side: Side::Buy,
                ..
            }
        )));
    }

    #[test]
    fn combo_value_respects_units_windows_inventory_and_sign() {
        let mut r = start().rules;
        r.combos.push(ComboRule {
            a: 0,
            a_units: 2,
            b: 1,
            b_units: 3,
            bonus: 10,
            start_tick: 200,
            end_tick: 300,
        });
        let obs = observation(100, true);
        // Buying more A cannot help when B already limits complete sets.
        assert_eq!(combo_value(&r, &obs, 0, 2), 0.0);
        assert_eq!(combo_value(&r, &obs, 1, 3), 10.0);
        r.combos[0].bonus = -10;
        assert_eq!(combo_value(&r, &obs, 1, 3), -10.0);
        assert!(negative_pair_blocked(&r, &obs, 0));
        r.combos[0].end_tick = 50;
        assert_eq!(combo_value(&r, &obs, 1, 3), 0.0);
    }

    #[test]
    fn no_self_cross_and_no_orders_beyond_action_or_open_order_limits() {
        let mut r = start().rules;
        let mut obs = observation(100, true);
        let quotes = vec![
            Quote {
                bid: Some((10_000, 1)),
                ask: Some((9_000, 1))
            };
            2
        ];
        assert!(execute(&obs, &r, &quotes).is_empty());
        let quotes = vec![
            Quote {
                bid: Some((9_000, 1)),
                ask: Some((11_000, 1))
            };
            2
        ];
        r.max_open_orders = 1;
        assert_eq!(execute(&obs, &r, &quotes).len(), 1);
        r.max_actions_per_tick = 0;
        assert!(execute(&obs, &r, &quotes).is_empty());
        obs.account.cash = -10;
        r.max_actions_per_tick = 20;
        assert!(!execute(&obs, &r, &quotes).iter().any(|a| matches!(
            a,
            Action::Limit {
                side: Side::Buy,
                ..
            }
        )));
    }
}
