use super::*;
use crate::news_test_support::*;

#[test]
fn zero_news_weight_matches_frozen_swift_even_with_delivered_news() {
    let mut s = start();
    s.params.insert("news_weight".into(), 0.0);
    let mut a = NewsV2::new(&s);
    let mut b = crate::swift::Swift::new(&s);
    for t in 0..150 {
        let mut o = observation(t, t % 40 < 25);
        if t == 30 {
            o.news.push(NewsItem {
                id: 1,
                layer: NewsLayer::Stock,
                sector: None,
                changes: vec![Change {
                    stock: 0,
                    low: 10,
                    high: 20,
                }],
                stage: 0,
                forecast: false,
                first_reveal_tick: 30,
                public_tick: 60,
                effective_tick: 120,
                public: false,
            });
        }
        assert_eq!(a.on_tick(&o), b.on_tick(&o));
    }
}

#[test]
fn private_information_changes_decision_but_world_seed_does_not() {
    let s = start();
    let mut other = s.clone();
    other.seed = 4321;
    let mut a = NewsV2::new(&s);
    let mut b = NewsV2::new(&other);
    let mut plain = crate::swift::Swift::new(&s);
    let mut o = observation(30, true);
    for ts in &mut o.trades {
        ts.push(Trade {
            tick: 30,
            price: 10000,
            qty: 20,
            aggressor: Side::Sell,
        });
    }
    o.news.push(NewsItem {
        id: 1,
        layer: NewsLayer::Stock,
        sector: None,
        changes: vec![Change {
            stock: 0,
            low: 20,
            high: 20,
        }],
        stage: 0,
        forecast: false,
        first_reveal_tick: 30,
        public_tick: 60,
        effective_tick: 120,
        public: false,
    });
    let actions = a.on_tick(&o);
    assert_eq!(actions, b.on_tick(&o));
    assert_ne!(actions, plain.on_tick(&o));
    assert!(actions.iter().any(|a| matches!(
        a,
        Action::Limit {
            stock: 0,
            side: Side::Buy,
            ..
        }
    )));
}
