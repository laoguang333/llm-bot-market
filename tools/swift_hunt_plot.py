"""Export the predator study as a standalone research figure."""
import json
import os
from pathlib import Path
import sys

ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/"runs/swift-analysis-deps"))
os.environ.setdefault("MPLCONFIGDIR",str(ROOT/"runs/swift-mpl-cache"))
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib import font_manager
import numpy as np
from swift_hunt_writeup import NAMES


def main():
    rpt=json.loads((ROOT/"docs/swift-hunt-results.json").read_text(encoding="utf-8"))
    font=Path("C:/Windows/Fonts/msyh.ttc")
    if font.exists():
        font_manager.fontManager.addfont(str(font))
        plt.rcParams["font.family"]=font_manager.FontProperties(fname=str(font)).get_name()
    plt.rcParams.update({"font.size":11,"axes.unicode_minus":False,"axes.spines.top":False,
                        "axes.spines.right":False,"axes.spines.left":False,"figure.facecolor":"#f7f9fc",
                        "axes.facecolor":"#f7f9fc","savefig.facecolor":"#f7f9fc","text.color":"#172b4d"})
    cases={r["case"]:r for r in rpt["combined"]}
    keys=["relay","clock","cash","blind","coral"]
    fig,axes=plt.subplots(1,2,figsize=(13,6.3),gridspec_kw={"width_ratios":[1.2,1]},sharey=True)
    y=np.arange(len(keys))
    for ax in axes:
        ax.set_yticks(y)
        ax.set_ylim(len(keys)-.5,-.6)
        ax.axvline(0,color="#738298",linewidth=1)
        ax.grid(axis="x",alpha=.2)
        ax.tick_params(length=0,pad=8)
    axes[0].set_yticklabels([NAMES[k] for k in keys])
    for i,key in enumerate(keys):
        r=cases[key]
        for who,offset,color,label in [("target",-.13,"#007f73","Swift"),("predator",.13,"#bd4545","十倍资金对手")]:
            m=r[who]["mean_return_pct"];lo,hi=r[who]["return_ci95"]
            axes[0].errorbar(m,i+offset,xerr=[[m-lo],[hi-m]],fmt="o",color=color,capsize=3,
                             label=label if i==0 else None)
        m=r["mean_advantage_pp"];lo,hi=r["advantage_ci95"]
        axes[1].errorbar(m,i,xerr=[[m-lo],[hi-m]],fmt="o",color="#435a7a",capsize=4)
        axes[1].annotate(f"{r['wins']}/80 胜",(hi,i),xytext=(8,0),textcoords="offset points",va="center",fontsize=10)
    axes[0].set_title("双方平均游戏收益率",loc="left",fontweight="bold",pad=20)
    axes[0].set_xlabel("收益率（%），包含分红、费用与结算")
    axes[0].legend(frameon=False,loc="upper right")
    axes[1].set_title("对手收益率 − Swift 收益率",loc="left",fontweight="bold",pad=20)
    axes[1].set_xlabel("百分点；大于 0 才是相对胜出")
    lo,hi=axes[1].get_xlim();axes[1].set_xlim(lo,max(hi,12))
    fig.suptitle("单一十倍资金对手能否狩猎 Swift",x=.035,y=.98,ha="left",fontsize=21,fontweight="bold")
    fig.text(.035,.88,"固定 Swift · 40 个新种子 × 两种注册顺序 · 440 局冻结验收",fontsize=12,color="#42526e")
    fig.text(.035,.04,"横线：按种子聚类的 95% bootstrap 区间。每个双账户处理 80 局，另有 Swift 单独参赛 40 局。\n"
             "relay 使用公开信号；clock 只按时间换方向。所有环境来自同一模拟器，开发结果未混入本图。",fontsize=10,color="#56657d")
    fig.subplots_adjust(left=.18,right=.96,top=.76,bottom=.2,wspace=.18)
    path=ROOT/"docs/swift-hunt-overview.png"
    fig.savefig(path,dpi=160)
    svg=path.with_suffix(".svg");fig.savefig(svg)
    svg.write_text("\n".join(s.rstrip() for s in svg.read_text(encoding="utf-8").splitlines())+"\n",encoding="utf-8")
    print(path)


if __name__=="__main__":
    main()
