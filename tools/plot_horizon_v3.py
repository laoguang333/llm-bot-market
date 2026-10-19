"""Plot every held-out world, retaining weak outcomes and comparison controls."""
import json
import os
from pathlib import Path
from build_release import windows_build_budget
if os.name == 'nt':
    windows_build_budget()
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt

ROOT = Path(__file__).resolve().parents[1]
rows = json.loads((ROOT/'runs/horizon-v3-research/batch.json').read_text())
assert len(rows) == 160
arms = ['v2_long', 'v3_control', 'v3_short', 'v3_long']
labels = ['v2 / 1000', 'v3 / research off', 'v3 / 300', 'v3 / 1000']
accounts = {arm: {r['seed']: next(a for a in r['summary']['accounts'] if a['account'] == 'researcher') for r in rows if r['arm'] == arm} for arm in arms}
fig, axs = plt.subplots(3, 1, figsize=(11, 11), layout='constrained')
for ax, field, title in [(axs[0], 'return_ratio', 'Net return (%)'), (axs[1], 'percentile', 'Rank percentile in each live market')]:
    values = [[(a[field]-1)*100 if field == 'return_ratio' else a[field] for _, a in sorted(accounts[arm].items())] for arm in arms]
    boxes = ax.boxplot(values, tick_labels=labels, showfliers=False, patch_artist=True)
    for box, color in zip(boxes['boxes'], ['#9aa3af', '#9aa3af', '#54b8a7', '#448bc9']):
        box.set_facecolor(color); box.set_alpha(.35)
    for x, ys in enumerate(values, 1):
        ax.scatter([x+((i*17 % 37)/36-.5)*.3 for i in range(len(ys))], ys, s=15, alpha=.6, color='#245876')
    ax.set_title(title, loc='left'); ax.grid(axis='y', alpha=.2)
axs[0].axhline(0, color='#777', lw=.7)
axs[1].axhline(75, color='#d78361', ls='--', lw=.8)
axs[1].set_ylim(0, 102)
seeds = sorted(accounts['v3_long'])
diffs = [(accounts['v3_long'][s]['return_ratio']-accounts['v3_control'][s]['return_ratio'])*100 for s in seeds]
axs[2].bar([s-32000 for s in seeds], diffs, color=['#3a9c89' if d > 0 else '#d47569' for d in diffs])
axs[2].axhline(0, color='#777', lw=.7)
axs[2].set(title='Increment from research: v3 / 1000 minus v3 / research off', xlabel='World index (seed = 32000 + index)', ylabel='Return difference (percentage points)')
axs[2].grid(axis='y', alpha=.2)
fig.suptitle('Horizon v3: 40 fixed held-out worlds | identical capital | no dragon-specific tuning')
dest = ROOT/'docs/assets/horizon-v3.png'
fig.savefig(dest, dpi=150)
print(dest)
