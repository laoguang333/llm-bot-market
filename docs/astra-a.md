# Astra A: frozen v5 entrant

Independent entrant A, developed from base `17a705b902e5fb81299135826f85f4b8aad0ea2f`. The author was told that two other Astra medium entrants were developing independently for a subsequent same-market competition. Their code and results were not used.

## Result and scope

**Training did not pass the mean-percentile threshold.** The frozen defaults scored **89.880 / 90% top quartile**, then **89.489 / 90%** on an unchanged repeat. Both are below the required mean of 90. No holdout or official competition was run by this entrant. All development runs used seeds 1–20, standard challenger capital and the unchanged v5 scenario.

15 twenty-world runs were performed (13 candidates plus two frozen repeats). Every market run used the shared arena gate. Normal scheduler nondeterminism explains some variation between identical-code repeats; the best individual run was not substituted for the frozen result.

## Strategy

The historical-band sampling, finite-lot combo valuation, and execution scaffolding are adapted from Coral, with explicit attribution. Astra A adds three mechanisms rather than wrapping or renaming Coral:

- Historical relative-price bands lose authority linearly over the last 500 ticks, converging toward expected payout value instead of carrying an obsolete resale premium through settlement.
- Inventory pressure shifts both quotes according to current stock value relative to its 25%-of-equity budget. The 1.5 leverage ceiling limits common-market exposure.
- For private multi-stock news, observed interval widths supply a relative shock scale. Normalized intervals are pooled and intersected, excluding very narrow ranges and obvious dividend-floor cases. Two-cent slack is a rounding heuristic, not a proof that the inferred range contains the truth. Public exact news remains authoritative.

Quotes use a 600-tick sampled window, 40th/60th percentiles, 0.5% minimum half-spread, 8% equity lot, 2% taker threshold and 50% finite-lot combo adjustment. Sampling is every 5 ticks; sorting every 20 ticks. Existing safe requote and idle-credit helpers are reused. Inputs are exclusively Start, Observation and published rules; there is no file I/O, RNG reconstruction, account-seed use, or world-seed branch in the bot.

## All training experiments

| Run | Mean percentile | SD | Top quartile | Pass |
|---|---:|---:|---:|---|
| astra-a-v1 | 86.051 | 20.000 | 85% | False |
| astra-a-v1-safe | 88.215 | 11.562 | 90% | False |
| astra-a-v1-wide | 88.780 | 9.124 | 90% | False |
| astra-a-v1-value | 78.031 | 11.333 | 70% | False |
| astra-a-v2-pool | 88.319 | 11.826 | 90% | False |
| astra-a-v2-active | 89.354 | 10.230 | 90% | False |
| astra-a-v3-rails | 88.161 | 9.725 | 90% | False |
| astra-a-v3-liquid | 88.203 | 8.766 | 85% | False |
| astra-a-v3-skew | 88.864 | 12.684 | 90% | False |
| astra-a-v4-health | 88.607 | 10.122 | 85% | False |
| astra-a-v4-core | 82.631 | 22.236 | 75% | False |
| astra-a-v5-consensus | 88.381 | 12.966 | 80% | False |
| astra-a-v5-balanced | 86.870 | 15.601 | 85% | False |
| astra-a-final-1 | 89.880 | 10.113 | 90% | False |
| astra-a-final-2 | 89.489 | 10.495 | 90% | False |

The early safe and wide candidates reduced the severe low-rank tail but did not pass. Pure or excessive value anchoring undertraded. Very tight fundamental rails, low-dividend budget scaling, and market-price consensus for terminal payments did not improve the result and were removed. Higher leverage without terminal anchoring increased the downside tail. The selected active candidate was re-expressed as fixed defaults, with unused experimentation mechanisms removed.

### Historical invocation parameters

These are the actual commands recorded at the time, against their then-current experimental implementation; removed experimental switches are not supported by the final source.

- `astra-a-v1`: `(then-current defaults)`
- `astra-a-v1-safe`: `leverage=1.4,cap=0.22,skew=0.02`
- `astra-a-v1-wide`: `leverage=1.6,cap=0.3,skew=0,convergence=1500,protection=0.7,quantile=0.25,margin=0.02`
- `astra-a-v1-value`: `leverage=1.6,cap=0.3,skew=0,convergence=6000,protection=0.7,quantile=0.25,margin=0.02`
- `astra-a-v2-pool`: `leverage=1.6,cap=0.3,skew=0,convergence=1500,protection=0.7,quantile=0.25,margin=0.02`
- `astra-a-v2-active`: `leverage=1.5,cap=0.25,skew=0.02,convergence=500,protection=0.5,quantile=0.4,margin=0.005,order_frac=0.08`
- `astra-a-v3-rails`: `leverage=1.5,cap=0.25,skew=0.02,convergence=500,protection=0.5,quantile=0.4,margin=0.005,order_frac=0.08`
- `astra-a-v3-liquid`: `leverage=1.2,cap=0.2,skew=0.02,convergence=500,protection=0.5,quantile=0.45,margin=0.003,order_frac=0.06,rail=1`
- `astra-a-v3-skew`: `leverage=1.5,cap=0.3,skew=0.08,convergence=500,protection=0.7,quantile=0.35,margin=0.01,order_frac=0.06,rail=1`
- `astra-a-v4-health`: `leverage=1.5,cap=0.25,skew=0.02,convergence=500,protection=0.5,quantile=0.4,margin=0.005,order_frac=0.08`
- `astra-a-v4-core`: `leverage=2,cap=0.35,skew=0,convergence=1,protection=0.5,quantile=0.35,margin=0.01,order_frac=0.04,rail=10`
- `astra-a-v5-consensus`: `leverage=1.5,cap=0.25,skew=0.02,convergence=500,protection=0.5,quantile=0.4,margin=0.005,order_frac=0.08`
- `astra-a-v5-balanced`: `leverage=1.5,cap=0.25,skew=0.02,convergence=1000,protection=0.8,quantile=0.35,margin=0.01,order_frac=0.06,consensus=0,distress=0,pooling=0`
- `astra-a-final-1`: `(then-current defaults)`
- `astra-a-final-2`: `(then-current defaults)`

## Frozen repeat diagnostics

### astra-a-final-1

```json
{
  "name": "astra-a-final-1",
  "mean": 89.88,
  "sd": 10.113,
  "top25": 0.9,
  "pass": false,
  "percentiles": [
    61.1,
    91.1,
    94.5,
    89.8,
    90.4,
    98.4,
    100.0,
    82.1,
    94.6,
    88.2,
    98.1,
    76.8,
    97.1,
    91.4,
    88.2,
    100.0,
    100.0,
    91.8,
    91.8,
    71.9
  ],
  "diagnostics": {
    "skipped": 0,
    "decisions": 3000,
    "rejects": 2.9,
    "trade_fees": 239612.75,
    "idle_fees": 65841.65,
    "interest": 90971.8,
    "dividends": 3309314.95,
    "combo_dividends": 305603.25,
    "liquidations": 0,
    "bot_errors": 0,
    "decision_us": 6.52
  },
  "all_accounts_skipped": 0,
  "all_accounts_disabled": 0,
  "wall_seconds": 16.88
}
```

### astra-a-final-2

```json
{
  "name": "astra-a-final-2",
  "mean": 89.489,
  "sd": 10.495,
  "top25": 0.9,
  "pass": false,
  "percentiles": [
    57.4,
    87.5,
    92.7,
    87.8,
    94.2,
    98.4,
    100.0,
    80.4,
    96.4,
    94.1,
    96.2,
    80.4,
    97.1,
    91.4,
    84.3,
    95.1,
    96.7,
    89.8,
    98.0,
    71.9
  ],
  "diagnostics": {
    "skipped": 0,
    "decisions": 3000,
    "rejects": 3.55,
    "trade_fees": 246232.8,
    "idle_fees": 66804.8,
    "interest": 93720.85,
    "dividends": 3324815.4,
    "combo_dividends": 306539.5,
    "liquidations": 0,
    "bot_errors": 0,
    "decision_us": 6.59
  },
  "all_accounts_skipped": 0,
  "all_accounts_disabled": 0,
  "wall_seconds": 16.97
}
```

Both frozen repeats have zero skipped decisions across all accounts, zero disabled accounts, zero challenger liquidations and zero bot errors. Average decision cost is about 6.5 microseconds. Rejections average roughly three per 3,000-decision world and are not represented as zero.

## Validation and reproduction

`cargo fmt --all`, `cargo clippy --workspace --all-targets -j 2 -- -D warnings`, and `cargo test --workspace -j 2` passed (23 tests, including five Astra A tests). The tests cover both adverse historical-band self-cross cases, finite-lot positive/negative combo adjustments, a pooled private-news trading signal, and the terminal reanchoring mechanism. Builds used `RUSTC_WRAPPER=""` and `cargo build --release -j 2`.

```text
python C:\Users\nillouise\Music\rust\llm-bot-market\tools\arena_gate.py --cwd C:\Users\nillouise\.codex\worktrees\astra-v5-a\llm-bot-market -- python tools/bench_bot.py astra-a-reproduce --bot astra_a --first-seed 1 --seeds 20
```

Use a fresh output name. Exact commands, source and executable SHA256 manifests, world summaries and compact diagnostics are retained locally under `runs/bench/<run>`. Those bulky generated outputs are not committed.

## Limitations

The training requirement was missed twice after freezing. Seed 1 is a particular weakness of the selected active variant, while seed 12 hurt earlier higher-inventory variants. The active strategy pays substantial turnover fees, and it can buy into a falling market before its historical ratio catches up. Private-news pooling assumes common proportional exposure and can misestimate effects near dividend floors or overlapping events. Competitors can exhaust the same liquidity; no claim about the unknown two-entrant competition is warranted before the host runs it. No further parameter changes are authorized by this freeze.
