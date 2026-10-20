"""Standalone measured comparison figure; run after report_dragon_duration.py."""
import json
import os
from pathlib import Path
import statistics as st

from build_release import windows_build_budget

if os.name == 'nt':
    windows_build_budget()

import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt

ROOT = Path(__file__).resolve().parents[1]
d = json.loads((ROOT/'docs/dragon-duration-results.json').read_text(encoding='utf-8'))
rows = d['rows']
lengths = [10000, 20000, 40000]
lookup = {(r['seed'],r['ticks']):r for r in rows}
plt.rcParams.update({'font.sans-serif':['Microsoft YaHei','DejaVu Sans'], 'axes.unicode_minus':False,
                     'font.size':10, 'axes.spines.top':False,'axes.spines.right':False})
fig, axs = plt.subplots(2,2,figsize=(12,8.2),layout='constrained')
colors = {'on':'#147d92','off':'#b25c45'}
for ax, key, title in [(axs[0,0],'peak_multiple','有龙：股价峰值 / 开局参考价'),
                       (axs[0,1],'dividend_peak_multiple','有龙：股息峰值 / 初始股息')]:
    all_y=[]
    for seed in range(81001,81013):
        ys=[lookup[(seed,t)]['on']['target'][key] for t in lengths]
        all_y += ys
        ax.plot(range(3),ys,color='#9bafb6',alpha=.45,lw=.8,marker='o',ms=3)
    med=[st.median(lookup[(seed,t)]['on']['target'][key] for seed in range(81001,81013)) for t in lengths]
    ax.plot(range(3),med,color=colors['on'],lw=2.8,marker='o',label='12 世界中位数')
    ax.set(title=title,ylabel='倍数',xticks=range(3),xticklabels=['1 万 ticks','2 万 ticks','4 万 ticks'])
    ax.grid(axis='y',alpha=.2)
    if max(all_y)>40:
        ax.set_yscale('log')
        ax.set_ylabel('倍数（对数刻度）')
    if key=='peak_multiple':
        ax.axhline(5,color='#999',ls=':',lw=.8)
        ax.axhline(10,color='#b38845',ls='--',lw=.8,label='十倍价格')
    ax.legend(frameon=False,fontsize=9)
for ax, key, title in [(axs[1,0],'market_return_pct','全市场最终净收益（含费用与终值）'),
                       (axs[1,1],'two_sided','其他九股：双边盘口覆盖')]:
    for arm in ['off','on']:
        med=[]
        for x,t in enumerate(lengths):
            rr=[r[arm] for r in rows if r['ticks']==t]
            values=[r[key] if key=='market_return_pct' else r['other_mean'][key]*100 for r in rr]
            offset=-.10 if arm=='off' else .10
            for i,y in enumerate(values):
                ax.scatter(x+offset+((i*7%13)-6)*.006,y,s=16,color=colors[arm],alpha=.32)
            med.append(st.median(values))
        ax.plot([x+(-.10 if arm=='off' else .10) for x in range(3)],med,color=colors[arm],marker='o',lw=2,
                label='指定大龙' if arm=='on' else '仅候选成长')
    ax.set(title=title,ylabel='%',xticks=range(3),xticklabels=['1 万 ticks','2 万 ticks','4 万 ticks'])
    ax.grid(axis='y',alpha=.2)
    ax.legend(frameon=False,fontsize=9)
    if key=='two_sided':
        ax.set_ylim(0,103)
    else:
        ax.axhline(0,color='#888',lw=.8)
fig.suptitle('大龙与局长：12 个固定种子，72 场独立市场运行',fontsize=15)
fig.supxlabel('只改变总 ticks 与是否有指定者；费用周期、事件频率、初始现金/股数和策略保持原设置。',fontsize=9)
out=ROOT/'docs/assets/dragon-duration.png'
out.parent.mkdir(exist_ok=True)
fig.savefig(out,dpi=150)
fig.savefig(out.with_suffix('.svg'))
print(out)
