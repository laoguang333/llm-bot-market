use super::*;
use crate::news_test_support::{observation, start};

fn fixed() -> Start {
    let mut s = start();
    for (k, v) in [
        ("peer_weight", 1.0),
        ("own_weight", 0.0),
        ("price_weight", 0.0),
        ("confidence", 0.0),
        ("threshold", 0.0),
        ("sensitivity", 1.0),
        ("loss_weight", 1.5),
    ] {
        s.params.insert(k.into(), v);
    }
    s
}
fn view(peer: f64) -> Observation {
    let mut o = observation(100, false);
    o.social = Some(SocialView {
        memory_ticks: 100.0,
        peers: 3,
        own_stock: vec![0.0; 2],
        peer_stock: vec![peer, 0.0],
        own_net: 0.0,
    });
    o
}

#[test]
fn flat_prices_peer_profit_buys_peer_loss_sells_monotonically() {
    let s = fixed();
    let mut previous = 0.0;
    for gain in [-0.01, -0.003, 0.0, 0.003, 0.01] {
        let (sig, actions) = ProfitEffect::new(&s).evaluate(&view(gain));
        assert!(sig[0].target_fraction >= previous);
        previous = sig[0].target_fraction;
        if gain != 0.0 {
            let side = if gain > 0.0 { Side::Buy } else { Side::Sell };
            assert!(actions
                .iter()
                .any(|a| matches!(a,Action::Limit{stock:0,side:x,..} if *x==side)));
        }
    }
}

#[test]
fn peer_off_ignores_peers_and_unread_news_values_tape_and_terminal() {
    let mut s = fixed();
    s.params.insert("peer_weight".into(), 0.0);
    let a = view(0.01);
    let mut b = view(-0.1);
    b.books[0].dividend = 999_999;
    b.terminal_hints = vec![(999, 999)];
    b.trades[0].clear();
    b.season.turbulent = true;
    assert_eq!(
        ProfitEffect::new(&s).on_tick(&a),
        ProfitEffect::new(&s).on_tick(&b)
    );
}

#[test]
fn actual_cash_caps_purchases_and_loss_can_exit_to_zero() {
    let mut o = view(0.1);
    o.account.cash = 0;
    o.account.buying_power = 9_000_000;
    assert!(!ProfitEffect::new(&fixed())
        .on_tick(&o)
        .iter()
        .any(|a| matches!(
            a,
            Action::Limit {
                side: Side::Buy,
                ..
            }
        )));
    let (signals, orders) = ProfitEffect::new(&fixed()).evaluate(&view(-0.1));
    assert_eq!(signals[0].target_shares, 0);
    assert!(orders.iter().any(|a| matches!(
        a,
        Action::Limit {
            stock: 0,
            side: Side::Sell,
            ..
        }
    )));
}

#[test]
fn different_sensitivities_react_differently_without_added_delay() {
    let mut s = fixed();
    s.params.insert("sensitivity".into(), 0.5);
    let slow = ProfitEffect::new(&s).evaluate(&view(0.003)).0[0].target_fraction;
    s.params.insert("sensitivity".into(), 2.0);
    let fast = ProfitEffect::new(&s).evaluate(&view(0.003)).0[0].target_fraction;
    assert!(fast > slow);
}

#[test]
fn fading_loss_does_not_rebuy_and_small_residual_still_exits() {
    let mut bot = ProfitEffect::new(&fixed());
    assert_eq!(bot.evaluate(&view(-0.1)).0[0].target_shares, 0);
    assert_eq!(bot.evaluate(&view(-0.00001)).0[0].target_shares, 0);
    assert_eq!(bot.evaluate(&view(0.0)).0[0].target_shares, 0);
    let mut o = view(-0.1);
    o.account.shares[0] = 1;
    o.account.sellable[0] = 1;
    o.account.equity = 8_000_000;
    assert!(bot.on_tick(&o).iter().any(|a| matches!(
        a,
        Action::Limit {
            stock: 0,
            side: Side::Sell,
            qty: 1,
            ..
        }
    )));
    assert!(bot.evaluate(&view(0.003)).0[0].target_shares > 0);
}

#[test]
fn cash_only_group_can_restart_from_public_market_leaders() {
    let mut s = fixed();
    s.shares.fill(0);
    s.params.insert("leader_weight".into(), 1.0);
    let mut o = view(0.0);
    o.account.shares.fill(0);
    o.account.sellable.fill(0);
    o.books[0].last = s.rules.stocks[0].initial_price * 103 / 100;
    o.books[1].last = s.rules.stocks[1].initial_price * 106 / 100;
    let (signals, orders) = ProfitEffect::new(&s).evaluate(&o);
    assert_eq!(signals[1].leader_rank, Some(1));
    assert_eq!(signals[0].leader_rank, Some(2));
    assert!(signals[1].target_fraction > signals[0].target_fraction);
    assert!(orders.iter().any(|a| matches!(
        a,
        Action::Limit {
            stock: 1,
            side: Side::Buy,
            ..
        }
    )));
    s.params.insert("leader_weight".into(), 0.0);
    let (off, orders) = ProfitEffect::new(&s).evaluate(&o);
    assert!(off
        .iter()
        .all(|x| x.target_shares == 0 && x.leader_rank.is_none()));
    assert!(orders.is_empty());
}

#[test]
fn leader_requires_positive_rise_and_respects_cash_and_losses() {
    let mut s = fixed();
    s.shares.fill(0);
    s.params.insert("leader_weight".into(), 1.0);
    let mut o = view(0.0);
    o.account.shares.fill(0);
    o.account.sellable.fill(0);
    for movement in [-100, 0, 40] {
        for (b, stock) in o.books.iter_mut().zip(&s.rules.stocks) {
            b.last = stock.initial_price * (10000 + movement) / 10000;
        }
        let (sig, orders) = ProfitEffect::new(&s).evaluate(&o);
        assert!(sig.iter().all(|x| x.leader_rank.is_none()));
        assert!(orders.is_empty());
    }
    o.books[0].last = s.rules.stocks[0].initial_price * 106 / 100;
    o.account.cash = 0;
    assert!(ProfitEffect::new(&s).on_tick(&o).is_empty());
    o.social.as_mut().unwrap().peer_stock[0] = -0.1;
    let sig = ProfitEffect::new(&s).evaluate(&o).0;
    assert!(sig[0].leader_contribution > 0.0);
    assert_eq!(
        sig[0].target_shares, 0,
        "a market leader cannot override sufficiently bad group experience"
    );
}

#[test]
fn leader_uses_relative_returns_not_absolute_price_or_hidden_information() {
    let mut s = fixed();
    s.params.insert("leader_weight".into(), 0.8);
    let mut a = view(0.0);
    a.books[0].last = s.rules.stocks[0].initial_price * 104 / 100;
    a.books[1].last = s.rules.stocks[1].initial_price * 102 / 100;
    let mut b = a.clone();
    b.books[0].dividend = 999_999;
    b.terminal_hints = vec![(999, 999)];
    b.news.clear();
    b.trades.iter_mut().for_each(Vec::clear);
    assert_eq!(
        ProfitEffect::new(&s).on_tick(&a),
        ProfitEffect::new(&s).on_tick(&b)
    );
    let sig = ProfitEffect::new(&s).evaluate(&a).0;
    assert_eq!(sig[0].leader_rank, Some(1));
    assert_eq!(sig[1].leader_rank, Some(2));
    // Equal percentage changes give the same scores after rescaling prices.
    for stock in &mut s.rules.stocks {
        stock.initial_price *= 2;
    }
    for book in &mut b.books {
        book.last *= 2;
    }
    let scaled = ProfitEffect::new(&s).evaluate(&b).0;
    for (x, y) in sig.iter().zip(scaled) {
        assert_eq!(x.leader_contribution, y.leader_contribution);
    }
}
