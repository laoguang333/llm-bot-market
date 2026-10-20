//! Built-in bots. `factory` builds one by name.

pub mod almanac;
pub mod astra_a;
pub mod astra_b;
pub mod astra_c;
pub mod baseline;
pub mod common;
pub mod coral;
pub mod falcon;
pub mod horizon;
pub mod horizon_v1;
pub mod horizon_v2;
pub mod keel;
mod keel_growth;
pub mod kestrel;
pub mod news;
#[cfg(test)]
mod news_test_support;
pub mod news_v2;
pub mod osprey;
pub mod osprey_v1;
pub mod profit_effect;
pub mod strategic;
pub mod swift;
pub mod tide;
pub mod titan;

use bot_api::{Bot, Start};

pub const NAMES: &[&str] = &[
    "noise",
    "grid",
    "value",
    "momentum",
    "news",
    "news_fixed",
    "news_v2",
    "hold",
    "fund",
    "combo",
    "vulture",
    "sniper",
    "harvester",
    "rider",
    "hunter",
    "shark",
    "drift",
    "tracker",
    "coral",
    "astra_a",
    "astra_b",
    "astra_c",
    "swift",
    "falcon",
    "titan",
    "horizon",
    "horizon_v1",
    "horizon_v2",
    "profit_effect",
    "tide",
    "kestrel",
    "osprey",
    "osprey_v1",
    "almanac",
    "keel",
];

/// `start` with extra parameter defaults; parameters set in the scenario win.
fn with_defaults(start: &Start, defaults: &[(&str, f64)]) -> Start {
    let mut s = start.clone();
    for (k, v) in defaults {
        s.params.entry(k.to_string()).or_insert(*v);
    }
    s
}

pub fn factory(name: &str, start: &Start) -> Option<Box<dyn Bot>> {
    Some(match name {
        "noise" => Box::new(baseline::Noise::new(start)),
        "profit_effect" => Box::new(profit_effect::ProfitEffect::new(start)),
        "grid" => Box::new(baseline::Grid::new(start)),
        "value" => Box::new(baseline::Value::new(start)),
        "momentum" => Box::new(baseline::Momentum::new(start)),
        "news" => Box::new(baseline::News::new(start)),
        "news_fixed" => Box::new(news::NewsFixed::new(start)),
        "news_v2" => Box::new(news_v2::NewsV2::new(start)),
        "hold" => Box::new(baseline::Hold::new(start)),
        "fund" => Box::new(strategic::Fund::new(start)),
        "combo" => Box::new(strategic::Fund::new(&with_defaults(
            start,
            &[("combo", 1.0)],
        ))),
        "vulture" => Box::new(strategic::Fund::new(&with_defaults(
            start,
            &[
                ("bid_margin", 0.10),
                ("ask_margin", 0.0),
                ("take_margin", 0.15),
                ("leverage", 2.0),
            ],
        ))),
        "sniper" => Box::new(strategic::Sniper::new(start)),
        "harvester" => Box::new(strategic::Harvester::new(start)),
        "rider" => Box::new(strategic::Rider::new(start)),
        "hunter" => Box::new(strategic::Hunter::new(start)),
        "shark" => Box::new(strategic::Shark::new(start)),
        "drift" => Box::new(strategic::Drift::new(start)),
        "tracker" => Box::new(strategic::Tracker::new(start)),
        "astra_a" => Box::new(astra_a::AstraA::new(start)),
        "astra_b" => Box::new(astra_b::AstraB::new(start)),
        "astra_c" => Box::new(astra_c::AstraC::new(start)),
        "coral" => Box::new(coral::Coral::new(start)),
        "swift" => Box::new(swift::Swift::new(start)),
        "falcon" => Box::new(falcon::Falcon::new(start)),
        "titan" => Box::new(titan::Titan::new(start)),
        "kestrel" => Box::new(kestrel::Kestrel::new(start)),
        "osprey" => Box::new(osprey::Osprey::new(start)),
        "osprey_v1" => Box::new(osprey_v1::Osprey::new(start)),
        "horizon" => Box::new(horizon::Horizon::new(start)),
        "horizon_v1" => Box::new(horizon_v1::Horizon::new(start)),
        "horizon_v2" => Box::new(horizon_v2::Horizon::new(start)),
        "tide" => Box::new(tide::Tide::new(start)),
        "almanac" => Box::new(almanac::Almanac::new(start)),
        "keel" => Box::new(keel::Keel::new(start)),
        _ => return None,
    })
}
