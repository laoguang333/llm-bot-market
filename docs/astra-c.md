# Astra C — independent v5 entry

## Scope and design

Only `Start`, `Observation`, and public `Rules` are used. No file/network access, world seed branching, hidden world access, or other entrants' code. The account seed is unused. Development worlds are exclusively 1–20. The entry is aware that two independent Astra medium entrants will compete after freezing, but has not inspected or coordinated with them.

The public Coral band sampler, lot-wise combo valuation, and shared quote execution helpers are reused. New mechanisms:

1. Infer a common terminal payment multiple from the median stock-implied multiple `(mid - remaining known cashflows) / observed dividend`. Mix 80% of this public estimate with 20% private midpoint after clipping to the private feasible interval. This pools prices across stocks without inferring hidden world randomness.
2. Apply a continuous inventory tilt around 12% of equity per stock, while enforcing 30% per-stock and 1.8 gross leverage caps.
3. Bound historical-band bids to 103% of calibrated share value and asks to at least 97%, before combo premiums. This prevents stale relative-price history from dominating fundamental risk controls.
4. Fade historical resale premiums toward fundamental value during the final 300 ticks.
5. Quote with a 300-tick ratio history; use 35/65% quantiles, 1% minimum half-spread, and 6% equity lots. Only 20% of long-horizon combo marginal value is capitalized because future pair holdings are not assured. Private-range-safe IOC opportunities retain a 2% edge threshold.

## Training record

All runs used baseline v5, four threads, 3000 ticks, standard 2,000,000 cash and 20 shares per stock, and serial `arena_gate.py`. Each candidate used exactly seeds 1–20. No holdout results were read or generated. Scheduling can affect outcomes, so the final default is repeated rather than treating a single run as deterministic.

Runs 01–09 used the current dividend as the terminal-implied-multiple denominator. Run 10 tested the expected terminal dividend after known pending changes as denominator (89.414 mean, 0.90 top25, failed). This amplified uncertain private projections and was reverted. Run 11 repeats candidate 06 as final defaults using the observed dividend scale.

| Run | Parameter variation from original candidate | Mean percentile | Top-25 share | SD | Pass |
|---|---|---:|---:|---:|---|
| 01 | Initial: window600, consensus.5, skew.03, leverage2, cap.35, combo.5, lot.04, guard disabled, settlement300 | 89.161 | 0.85 | 17.846 | False |
| 02 | consensus.8, skew.08, leverage1.5, cap.25 | 86.860 | 0.90 | 15.389 | False |
| 03 | leverage2.5, cap.5, combo.7, lot.06 | 83.006 | 0.85 | 28.582 | False |
| 04 | guard.08 | 89.534 | 0.90 | 12.220 | False |
| 05 | guard.03, window300, consensus.8 | 91.001 | 0.90 | 11.086 | True |
| 06 | 05 + combo.2, leverage1.8, cap.3, lot.06 | 91.095 | 1.00 | 8.476 | True |
| 07 | 06 + window150, consensus1, quantile.4 | 90.168 | 0.90 | 7.831 | True |
| 08 | 06 + quantile.25 | 90.451 | 0.95 | 10.574 | True |
| 09 | 06 + guard0 | 89.548 | 0.90 | 11.342 | False |

All first nine candidates had zero skipped decisions, bot errors, liquidations, or disabled accounts. Initial high-leverage experiments increased downside sharply. Candidate 06 gave the best observed tail coverage (20/20 top-quartile worlds); adjacent candidates also passed but were less attractive. This is in-sample selection, not an out-of-sample performance claim.

## Reproduction

```text
python C:\Users\nillouise\Music\rust\llm-bot-market\tools\arena_gate.py --cwd C:\Users\nillouise\.codex\worktrees\astra-v5-c\llm-bot-market -- python tools/bench_bot.py astra-c-reproduce --bot astra_c --first-seed 1 --seeds 20
```

Build with `RUSTC_WRAPPER` empty and `cargo build --release -j 2`. Detailed per-seed reports, command lines and source/binary hashes are retained locally under `runs/bench/astra-c-*`.

## Validation and limitations

`cargo fmt --all`, `cargo clippy --all-targets -j 2 -- -Dwarnings`, and the five Astra C / three Coral integration tests pass. Astra C tests cover terminal convergence, stale-band value guards, inventory skew, bounded public calibration, and self-cross-safe liquidity taking.

Consensus prices are endogenous and may be biased by other strong entrants. The strategy cannot distinguish information from crowd herding; guards and private bounds limit but do not remove that risk. Unknown future dividend shocks can overwhelm current hold-value estimates. The method sacrifices some profitable premium trading and combo yield for tail protection. Leverage constraints are based on delayed observable marks and cannot ensure an instantaneous hard ceiling. The official and blind same-market tests must occur after freezing; no claims about them are made here.

## Frozen default result

Candidate 06 parameters were repeated as the final default, with mean percentile **90.396**, top25 share **0.90**, SD **8.450**. The original same-parameter run was **91.095 / 1.00**. Both pass, but the margin over 90 is narrow and scheduling changes have measurable effects.

Final per-seed percentiles (seeds 1–20): 74.1, 92.9, 89.1, 93.9, 96.2, 100.0, 96.2, 92.9, 83.9, 98.0, 94.3, 71.4, 100.0, 84.5, 84.3, 98.4, 100.0, 91.8, 83.7, 82.5.

Final mean diagnostics: 3000 decisions, 6.53 microseconds/decision, 1.65 order rejections, zero skipped decisions, bot errors, liquidations, and disabled accounts. This sequence used 11 total 20-world batches (including failed experiments and the final repeat), all training worlds.
