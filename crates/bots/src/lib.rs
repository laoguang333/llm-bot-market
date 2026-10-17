//! Built-in bots. `factory` builds one by name.

pub mod baseline;
pub mod common;
pub mod strategic;

use bot_api::{Bot, Start};

pub const NAMES: &[&str] = &[
    "noise",
    "grid",
    "value",
    "momentum",
    "news",
    "hold",
    "fund",
    "combo",
    "vulture",
    "sniper",
    "harvester",
    "rider",
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
        "grid" => Box::new(baseline::Grid::new(start)),
        "value" => Box::new(baseline::Value::new(start)),
        "momentum" => Box::new(baseline::Momentum::new(start)),
        "news" => Box::new(baseline::News::new(start)),
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
        _ => return None,
    })
}
