//! Built-in bots. `factory` builds one by name.

pub mod baseline;
pub mod common;

use bot_api::{Bot, Start};

pub const NAMES: &[&str] = &["noise", "maker", "value", "momentum", "news", "hold"];

pub fn factory(name: &str, start: &Start) -> Option<Box<dyn Bot>> {
    Some(match name {
        "noise" => Box::new(baseline::Noise::new(start)),
        "maker" => Box::new(baseline::Maker::new(start)),
        "value" => Box::new(baseline::Value::new(start)),
        "momentum" => Box::new(baseline::Momentum::new(start)),
        "news" => Box::new(baseline::News::new(start)),
        "hold" => Box::new(baseline::Hold::new(start)),
        _ => return None,
    })
}
