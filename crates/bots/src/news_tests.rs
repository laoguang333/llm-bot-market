use super::*;
use crate::news_test_support::*;

fn item(id: u32, public: bool, value: Money) -> NewsItem {
    NewsItem {
        id,
        layer: NewsLayer::Market,
        sector: None,
        changes: (0..2)
            .map(|stock| Change {
                stock,
                low: value,
                high: value,
            })
            .collect(),
        stage: usize::from(public),
        forecast: false,
        first_reveal_tick: 0,
        public_tick: 60,
        effective_tick: 120,
        public,
    }
}

fn orders(actions: Vec<Action>, side: Side) -> Vec<(usize, Qty)> {
    let mut out: Vec<_> = actions
        .into_iter()
        .filter_map(|a| match a {
            Action::Limit {
                stock,
                side: s,
                qty,
                ..
            } if s == side => Some((stock, qty)),
            _ => None,
        })
        .collect();
    out.sort_unstable();
    out
}

#[test]
fn all_stocks_and_unfilled_entries_survive_three_attempts() {
    let mut s = start();
    s.params.insert("lot".into(), 30.0);
    let mut bot = NewsFixed::new(&s);
    for tick in 0..5 {
        let mut o = observation(tick, true);
        if tick == 0 {
            o.news.push(item(1, false, 20));
        }
        assert_eq!(orders(bot.on_tick(&o), Side::Buy), vec![(0, 30), (1, 30)]);
    }
    // No fills: public expiry must not sell fictitious acquired inventory.
    assert!(bot.on_tick(&observation(60, true)).is_empty());
}

#[test]
fn public_expiry_closes_only_actual_extra_inventory_and_retries_partial_exit() {
    let mut bot = NewsFixed::new(&start());
    let mut o = observation(0, true);
    o.news.push(item(1, false, 20));
    bot.on_tick(&o);
    let mut o = observation(60, true);
    o.news.push(item(1, true, 20));
    o.account.shares[0] = 27;
    o.account.sellable[0] = 27;
    assert_eq!(orders(bot.on_tick(&o), Side::Sell), vec![(0, 7)]);
    o.tick += 1;
    o.news.clear();
    o.account.shares[0] = 23;
    o.account.sellable[0] = 23;
    assert_eq!(orders(bot.on_tick(&o), Side::Sell), vec![(0, 3)]);
}

#[test]
fn repeated_delivery_does_not_duplicate_positions_and_overlap_is_netted() {
    let mut s = start();
    s.params.insert("lot".into(), 30.0);
    let mut bot = NewsFixed::new(&s);
    let mut o = observation(0, true);
    o.news = vec![item(1, false, 20), item(1, false, 20)];
    bot.on_tick(&o);
    assert_eq!(bot.events.0.len(), 2);
    o.tick = 1;
    let mut later = item(2, false, 20);
    later.public_tick = 100;
    o.news = vec![later];
    bot.on_tick(&o);
    o.tick = 60;
    o.news.clear();
    o.account.shares = vec![130, 110];
    o.account.sellable = o.account.shares.clone();
    assert_eq!(orders(bot.on_tick(&o), Side::Sell), vec![(0, 20)]);
    assert_eq!(bot.events.0.len(), 2);
}

#[test]
fn shared_budget_and_action_limit_bound_many_stock_news() {
    let mut s = start();
    s.rules.max_actions_per_tick = 1;
    let mut bot = NewsFixed::new(&s);
    let mut o = observation(0, true);
    o.news = vec![item(1, false, 20)];
    o.account.buying_power = 20_030;
    let actions = bot.on_tick(&o);
    assert_eq!(actions.len(), 1);
    assert_eq!(orders(actions, Side::Buy), vec![(0, 1)]);
}

#[test]
fn updated_ranges_replace_uncertain_direction_and_negative_news_cannot_short() {
    let mut bot = NewsFixed::new(&start());
    let mut o = observation(0, true);
    o.news = vec![item(1, false, -20)];
    for (_, q) in orders(bot.on_tick(&o), Side::Sell) {
        assert_eq!(q, 20);
    }
    o.account.shares = vec![0, 0];
    o.account.sellable = vec![0, 0];
    o.tick = 1;
    o.news.clear();
    assert!(bot.on_tick(&o).is_empty());
    o.tick = 60;
    assert_eq!(orders(bot.on_tick(&o), Side::Buy).len(), 2);
}
