//! Research only: dump hidden world truth (public news, dragon outcome, N) as JSON lines.
use anyhow::{Context, Result};
use market::{config::Config, world::World};
use std::io::Write;
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let cfg = Config::load(std::path::Path::new(
        args.first()
            .context("scenario count first-seed output.jsonl")?,
    ))?;
    let count: u64 = args.get(1).context("count")?.parse()?;
    let first: u64 = args.get(2).context("first seed")?.parse()?;
    let mut out = std::io::BufWriter::new(std::fs::File::create(args.get(3).context("output")?)?);
    for seed in first..first + count {
        let w = World::generate(&cfg, seed, 0);
        let events: Vec<_> = w
            .events
            .iter()
            .map(|e| {
                serde_json::json!([
                    format!("{:?}", e.layer),
                    e.first_reveal,
                    e.public_tick,
                    e.effective,
                    e.changes
                ])
            })
            .collect();
        let d = w.dragon.as_ref();
        let row = serde_json::json!({
            "seed": seed, "n": w.terminal_payments, "open": w.opening_prices,
            "initial": cfg.stocks.iter().map(|s| s.dividend).collect::<Vec<_>>(),
            "candidates": d.map(|d| d.candidates.clone()), "selected": d.and_then(|d| d.selected_stock),
            "events": events, "seasons": w.season_changes,
        });
        writeln!(out, "{}", row)?;
    }
    Ok(())
}
