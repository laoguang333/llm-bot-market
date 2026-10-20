"""All-world follower activity; no profitable-example selection."""
import json
import os
from pathlib import Path
from build_release import windows_build_budget

if os.name == 'nt': windows_build_budget()
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt

ROOT = Path(__file__).resolve().parents[1]
report = json.loads((ROOT / 'docs/profit-effect-leaders-results.json').read_text(encoding='utf-8'))
fig, axes = plt.subplots(1, 2, figsize=(12, 4.7), layout='constrained', sharey=True)
for ax, control, title in zip(axes, ['group', 'full'], ['Group/own experience only', 'With original individual price signal']):
    for arm, label, color in [(control, 'No market-leader members', '#949ca8'),
                               (control + '_leaders', 'Minority market-leader members', '#397caa')]:
        # Show ONLY the remaining two thirds: not the people given the new signal.
        data = report['arms'][arm]['subsets']['others']['active_by_1000']
        ax.plot(range(1, 11), [x * 100 for x in data], marker='o', lw=2, label=label, color=color)
    ax.set(title=title, xlabel='1,000-tick interval', xticks=range(1, 11), ylim=(-2, 103))
    ax.grid(alpha=.2); ax.legend(fontsize=9, loc='lower left')
axes[0].set_ylabel('Other members with any actual fill (%)')
fig.suptitle('Can a minority of market chasers keep the other members active?\n20 paired worlds; equal world weights; no news or valuation input', fontsize=12)
dest = ROOT / 'docs/assets/profit-effect-leaders-activity.png'
fig.savefig(dest, dpi=150)
print(dest)
