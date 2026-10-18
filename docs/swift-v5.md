# Swift: price-only v5 challenger

## Scope and frozen evaluation protocol

Swift uses the public order book, executed trades, recent prices, trading clock,
and its own account limits. It does **not** read dividends, news, private terminal
hints, stock loadings, combo rules, other accounts, or a world seed. Its signal
does not call Coral, Knowledge, hold_value, or any fundamental-value function.
Dividends still accrue under normal game rules; receiving income is different
from using it as a forecast input. Equity is used only to size positions.

The unchanged v5 challenge uses cash 2,000,000 cents and 20 shares of each stock,
3,000 ticks, and four threads. No starting-capital discount, altered opponent
population, fee waiver, scoring change, or skipped-decision advantage is used.
The existing shared idle-fee accounting fix remains in the engine.

The pass criteria are mean rank percentile >= 90 and at least 80% of worlds
finishing in the top quartile. Percentiles rank final/initial equity.

Development used historical public prices/books/trades from baseline seeds 1–20.
The first implemented strategy passed live challenge seeds 1–20 (95.840 mean)
and then seeds 21–60 (95.324 mean), without parameter changes or a parameter grid.
After these runs the unused 100-tick accumulator was removed and regression
tests were added; the trading equations and parameters were unchanged.

Before opening validation results, the following evaluation was fixed:

| Set | Seeds | Purpose |
| --- | --- | --- |
| Official legacy | 1001–1020 | Existing v5 gate; these worlds were used by earlier bots |
| Fresh A | 5001–5040 | New-world validation, no tuning afterward |
| Fresh B | 6001–6040 | Independent second batch, same frozen executable |

Raw challenge reports, commands, source hashes and executable hashes are kept
under `runs/bench/swift-*`. The concise permanent result artifact will accompany
this document. Validation worlds must not be used to revise this candidate and
then relabeled as untouched holdout data.

## Mechanism

Per stock, maintain exponentially decayed signed trade volume over 10 and 30
ticks and a trailing 30-tick midprice average. The score is:

```
0.7 * flow10 + 0.3 * flow30 + 0.3 * depth_imbalance + 5 * momentum30
```

Flow is (aggressive buy quantity - aggressive sell quantity) / total quantity.
Depth imbalance is (bid quantity - ask quantity) / total displayed quantity,
using visible levels within 2% of mid. Momentum is mid/EMA30 - 1, clipped to
[-0.1, 0.1]. All observed tape trades are included, including the bot's own trades
when present in the public tape; no participant identities are available to it.

After 30 ticks, score > 0.15 with sufficient recent volume enables a position
target of 35% of equity in that stock. Score < -0.3 disables that target. The gap
between these thresholds reduces spread-paying churn. Overall target leverage
is capped at 1.5, with one shared fee-inclusive buying budget across stocks.

Orders are IOC limits at the current best price; they do not sweep worse prices.
Buys require spread <= 1.2%. Unfilled quantity waits for another observation.
The bot starts exiting 20 ticks before the end; empty bids can prevent a complete
exit. Small capital helps because its desired trades can fit in top-level depth.
There is no short selling or hidden terminal-value prediction.

## Checks

- A multi-tick metamorphic test changes fundamental metadata, seed, news,
  dividends, combo income, season and terminal hints while keeping permitted
  market/account inputs fixed, and checks identical actual buy/sell orders.
- Execution tests cover aggregate buying power, sellable shares, end-of-game
  exits, absent asks and excessive spreads.

Reproduce a challenge after `cargo build --release`:

```
python tools/bench_bot.py swift-replay --bot swift --first-seed 5001 --seeds 40
```

The multithreaded simulator is not bit-for-bit reproducible; seed fixes the world,
but OS scheduling can change matching paths. A rerun need not have identical ranks.
