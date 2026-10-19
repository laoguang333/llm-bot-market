"""Plot measured Horizon results, with every world's return shown."""
import json,os
from pathlib import Path
from build_release import windows_build_budget
if os.name=='nt':windows_build_budget()
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
ROOT=Path(__file__).resolve().parents[1]
rows=json.loads((ROOT/'runs/v10-research/batch.json').read_text())
arms=['old1000','new0','new300','new600','new1000','titan1000']
labels=['Horizon v1\n1000','Horizon v2\nordinary','Horizon v2\n300','Horizon v2\n600','Horizon v2\n1000','Titan\n1000']
returns=[];ranks=[]
for arm in arms:
    accounts=[next(a for a in row['summary']['accounts'] if a['account']=='researcher') for row in rows if row['arm']==arm]
    returns.append([(a['return_ratio']-1)*100 for a in accounts]);ranks.append([a['percentile'] for a in accounts])
fig,axs=plt.subplots(2,1,figsize=(11,8),layout='constrained')
for ax,values,title,ylabel in [(axs[0],returns,'Net return: identical starting cash and shares','Return (%)'),(axs[1],ranks,'Rank percentile within each live market','Percentile')]:
    boxes=ax.boxplot(values,tick_labels=labels,showfliers=False,patch_artist=True)
    for box,color in zip(boxes['boxes'],['#8b97a0','#8b97a0','#48a89c','#48a89c','#48a89c','#e3a569']):box.set_facecolor(color);box.set_alpha(.35)
    for x,ys in enumerate(values,1):
        jitter=[x+((i*17%37)/36-.5)*.35 for i in range(len(ys))]
        ax.scatter(jitter,ys,s=13,alpha=.5,color='#326f86',zorder=3)
    ax.set(title=title,ylabel=ylabel);ax.grid(axis='y',alpha=.2)
axs[0].axhline(0,color='#777',lw=.7)
axs[1].axhline(75,color='#d78361',ls='--',lw=.8)
axs[1].set_ylim(0,102)
fig.suptitle('v10 held-out worlds 31001–31040 · one additional account per match')
dest=ROOT/'docs/assets/v10-horizon.png';dest.parent.mkdir(exist_ok=True)
fig.savefig(dest,dpi=150);print(dest)
