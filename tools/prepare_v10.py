"""Create the scaled-terminal scene, without changing archived scenes."""
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
text=(ROOT/'scenarios/baseline-v9.toml').read_text(encoding='utf-8')
start=text.index('[scoring]');end=text.index('\n[[stocks]]',start)
text=text[:start]+'''[scoring]
# Random terminal duration scales with game length. Rounded to 50-tick units.
# At 3000 ticks: 10..50 units; at 10000 ticks: 33..167 (mean 5000 ticks).
terminal_payment_ticks = 50
terminal_ratio_range = [0.16666666666666666, 0.8333333333333334]
# (time / total ticks, uncertainty width / total ticks), also scales with length.
terminal_hint_fractions = [[0.0, 0.5], [0.3333333333333333, 0.26666666666666666], [0.6666666666666666, 0.1]]
objective = "rank_percentile"
series_seeds = 20

'''+text[end:]
text=text.replace('# v9:', '# v10:')
(ROOT/'scenarios/baseline-v10.toml').write_text(text,encoding='utf-8')
