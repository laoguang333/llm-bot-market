//! Cheap hidden-world-only shape evaluation. No bots or trades are run.
use anyhow::{Context, Result};
use market::{config::Config, world::World};
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let cfg = Config::load(std::path::Path::new(
        args.first()
            .context("scenario count first-seed output.json")?,
    ))?;
    let count: u64 = args.get(1).context("count")?.parse()?;
    let first: u64 = args.get(2).context("first seed")?.parse()?;
    let mut rows = Vec::new();
    for seed in first..first + count {
        let world = World::generate(&cfg, seed, 0);
        rows.push(serde_json::json!({"seed":seed,"world":market::diagnostics::world(&cfg,&world)}));
    }
    std::fs::write(
        args.get(3).context("output")?,
        serde_json::to_vec_pretty(&rows)?,
    )?;
    Ok(())
}
