"""Post-study checks of two observed exceptions; keep all repetitions."""
import json
import os
from pathlib import Path
import statistics

from build_release import windows_build_budget
from per_tick_study import OUT, ROOT, command, verify, write


def main():
    if os.name == "nt":
        windows_build_budget()
    plan = json.loads((OUT / "plan.json").read_text(encoding="utf-8"))
    verify(plan)
    follow = dict(reason="Post hoc: v6 background seed 23014 two-sided share 0.9483; v6 Swift seed 23004 three tracker liquidations.",
                  seeds={"background":23014,"swift":23004}, repeats=5, threads=4,
                  conditions=["periodic","per-tick"], parameters="unchanged", selection="all runs retained")
    frozen = OUT / "followup-plan.json"
    if frozen.exists():
        assert json.loads(frozen.read_text(encoding="utf-8")) == follow
    else:
        write(frozen,follow)
    rows=[]
    for bot,seed in follow["seeds"].items():
        for repeat in range(5):
            order=follow["conditions"] if repeat%2==0 else follow["conditions"][::-1]
            for regime in order:
                dest=OUT/"followup"/f"{bot}-{regime}-s{seed}-r{repeat}"
                cmd=[str(OUT/"after.exe"),"run" if bot=="background" else "challenge",
                     str(OUT/f"{regime}-v6.toml"),"--out",str(dest),"--threads","4"]
                if bot=="background":
                    cmd += ["--seed",str(seed)]
                else:
                    cmd += ["--bot","swift","--seeds","1","--first-seed",str(seed),"--cash","2000000","--shares","20"]
                command(cmd,dest)
                report=dest/"summary.json" if bot=="background" else dest/f"seed-{seed}"/"summary.json"
                w=json.loads(report.read_text(encoding="utf-8"))
                rows.append(dict(bot=bot,seed=seed,repeat=repeat,regime=regime,
                    quality=w["quality"], liquidations={a["owner"]:a["stats"]["liquidations"] for a in w["accounts"] if a["stats"]["liquidations"]},
                    bad_debt=sum(a["stats"]["bad_debt"] for a in w["accounts"]),
                    skipped=sum(a["stats"]["skipped"] for a in w["accounts"])))
    write(ROOT/"docs/per-tick-followup.json",dict(plan=follow,runs=rows))
    for bot in follow["seeds"]:
        for regime in follow["conditions"]:
            group=[r for r in rows if r["bot"]==bot and r["regime"]==regime]
            cover=[next(c["value"] for c in r["quality"]["checks"] if c["name"]=="two_sided_share") for r in group]
            print(bot,regime,dict(coverage_range=[min(cover),max(cover)],coverage_median=statistics.median(cover),
                quality_pass=sum(r["quality"]["pass"] for r in group),
                liquidation_runs=sum(bool(r["liquidations"]) for r in group),
                liquidations=sum(sum(r["liquidations"].values()) for r in group)),flush=True)
    verify(plan)


if __name__=="__main__":
    main()
