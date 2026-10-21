use bot_api::{Bot, RandomEndRules, Start};
use market::config::Config;
use std::sync::{Arc, Mutex};

fn short_random_end() -> Config {
    let mut cfg = Config::load(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scenarios/baseline.toml"
    )))
    .unwrap();
    cfg.ticks = 1500;
    cfg.schedule.threads = 2;
    cfg.scoring.random_end = Some(RandomEndRules {
        min_tick: 400,
        hazard_per_tick: 0.002,
    });
    cfg.validate().unwrap();
    cfg
}

#[test]
fn end_draws_respect_the_public_rule_and_match_its_expectation() {
    let cfg = short_random_end();
    let ends: Vec<_> = (0..4000).map(|s| cfg.end_tick(s)).collect();
    assert!(ends.iter().all(|&t| t > 400 && t <= 1500));
    assert!(ends.contains(&1500));
    let mean = ends.iter().map(|&t| t as f64).sum::<f64>() / ends.len() as f64;
    assert!((mean / cfg.expected_ticks() - 1.0).abs() < 0.03, "{mean}");
    // the draw depends on the seed alone
    assert_eq!(cfg.end_tick(17), cfg.end_tick(17));
    let mut fixed = cfg.clone();
    fixed.scoring.random_end = None;
    assert_eq!(fixed.end_tick(17), 1500);
}

#[test]
fn bots_see_the_longest_game_while_the_match_stops_at_the_hidden_end() {
    let cfg = short_random_end();
    let seed = (0..100).find(|&s| cfg.end_tick(s) < 1000).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let factory = move |name: &str, start: &Start| -> Option<Box<dyn Bot>> {
        log.lock()
            .unwrap()
            .push((start.rules.ticks_total, start.rules.random_end));
        bots::factory(name, start)
    };
    let r = market::report::run_match(&cfg, seed, &factory, None).unwrap();
    assert_eq!(r.ticks, cfg.end_tick(seed));
    let seen = seen.lock().unwrap();
    assert!(!seen.is_empty());
    assert!(seen
        .iter()
        .all(|(t, e)| *t == 1500 && *e == cfg.scoring.random_end));
}
