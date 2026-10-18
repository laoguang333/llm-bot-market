"""Fixed news/flow experiments; every child shares the Windows half-core budget."""
import argparse
import copy
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import shutil
import statistics as stats
import subprocess
import sys
import time
import tomllib
import zipfile

from build_release import windows_build_budget
from swift_stress import toml_text

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "runs/news-swift-20261019"


def sha(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()


def write(p, value):
    p.write_text(json.dumps(value, ensure_ascii=False, indent=2)+"\n", encoding="utf-8")


def source_hashes():
    files = sorted((ROOT/"crates").rglob("*.rs")) + sorted((ROOT/"crates").rglob("Cargo.toml"))
    files += [ROOT/p for p in ["Cargo.toml","Cargo.lock","docs/news-swift-mechanism-protocol.md",
                              "docs/news-swift-bot-review.md","tools/news_swift_study.py"]]
    return {p.relative_to(ROOT).as_posix():sha(p) for p in files}


def command(cmd, label, env):
    print("START",label,flush=True)
    start=time.perf_counter()
    with (OUT/f"{label}.log").open("w",encoding="utf-8") as log:
        p=subprocess.run(cmd,cwd=ROOT,env=env,stdout=log,stderr=subprocess.STDOUT)
    record={"command":cmd,"wall_seconds":time.perf_counter()-start,"exit_code":p.returncode}
    write(OUT/f"{label}.command.json",record)
    if p.returncode:
        print((OUT/f"{label}.log").read_text(encoding="utf-8")[-12000:])
        raise SystemExit(f"Failed {label}")
    print(f"PASS {label} ({record['wall_seconds']:.2f}s)",flush=True)
    return record


def build(budget,env):
    for label,cmd in [
        ("fmt",["cargo","fmt","--all"]),
        ("clippy",["cargo","clippy","--workspace","--all-targets","--jobs",str(budget),"--","-D","warnings"]),
        ("tests",["cargo","test","--workspace","--jobs",str(budget),"--","--test-threads=1"]),
        ("release",["cargo","build","--release","--bins","--jobs",str(budget)]),
    ]:
        command(cmd,label,env)


def prepare(budget):
    if (OUT/"plan.json").exists(): raise SystemExit("Plan exists; frozen inputs cannot be replaced")
    cfgs={v:tomllib.loads((ROOT/("scenarios/baseline-v6-five-swift.toml" if v=="v6" else "scenarios/baseline-v5.toml")).read_text(encoding="utf-8")) for v in ["v5","v6"]}
    cfgs["no_news"]=copy.deepcopy(cfgs["v5"])
    for layer in ["market","sector","stock"]:
        cfgs["no_news"]["world"]["news"][layer]["events_per_1000_ticks"]=0.0
    cfgs["active_no_news"]=copy.deepcopy(cfgs["no_news"])
    for b in cfgs["active_no_news"]["bots"]:
        if b["bot"]=="noise":
            b["params"]={"prob":1.0,"max_qty":80.0,"spread":0.003,"value_weight":0.0,"ttl":5.0}
    scenarios={}
    for name,cfg in cfgs.items():
        p=OUT/f"{name}.toml"
        p.write_text(toml_text(cfg),encoding="utf-8")
        scenarios[name]={"path":str(p),"sha256":sha(p)}
    jobs=[]
    def add(market,bot="swift",mode="plain",trace=False):
        jobs.append({"id":f"{market}-{bot}-{mode}","market":market,"bot":bot,"mode":mode,"trace":trace,"first_seed":21001,"seeds":40})
    for market in ["v5","v6"]:
        for bot in ["news","news_fixed","news_v2","swift"]: add(market,bot,trace=bot=="swift")
    for mode in ["flow","random","reverse","exclude_green","only_green","green"]: add("v5",mode=mode)
    for market in ["no_news","active_no_news"]:
        for mode in ["plain","flow","random"]: add(market,mode=mode,trace=mode=="plain")
    for mode in ["flow","random","exclude_green"]: add("v6",mode=mode)
    assert len(jobs)==23
    binary=OUT/("flow-eval-frozen.exe" if os.name=="nt" else "flow-eval-frozen")
    shutil.copy2(ROOT/"target/release"/binary.name.replace("-frozen",""),binary)
    sources=source_hashes()
    with zipfile.ZipFile(OUT/"sources.zip","w",zipfile.ZIP_DEFLATED) as z:
        for rel in sources: z.write(ROOT/rel,rel)
    plan={"created_utc":datetime.now(timezone.utc).isoformat(),"cpu_budget":budget,"threads":4,
          "ticks":3000,"repeats":1,"sources":sources,"binary":str(binary),"binary_sha256":sha(binary),
          "scenarios":scenarios,"jobs":jobs,"world_runs":920,
          "git_head":subprocess.check_output(["git","rev-parse","HEAD"],cwd=ROOT,text=True).strip(),
          "note":"Includes pre-existing uncommitted engine corrections. Source snapshot, not HEAD alone, identifies the build."}
    write(OUT/"plan.json",plan)
    shutil.copy2(OUT/"plan.json",ROOT/"docs/news-swift-plan.json")
    print("Frozen 23 conditions / 920 world-runs",flush=True)


def verify(plan):
    assert source_hashes()==plan["sources"],"Frozen source changed"
    assert sha(Path(plan["binary"]))==plan["binary_sha256"]
    for s in plan["scenarios"].values(): assert sha(Path(s["path"]))==s["sha256"]


def run(env):
    plan=json.loads((OUT/"plan.json").read_text(encoding="utf-8"))
    verify(plan)
    for j in plan["jobs"]:
        dest=OUT/j["id"]
        if (dest/"done.json").exists(): continue
        if dest.exists(): raise SystemExit(f"Incomplete output; preserve and investigate: {dest}")
        cmd=[plan["binary"],plan["scenarios"][j["market"]]["path"],"--out",str(dest),"--bot",j["bot"],
             "--mode",j["mode"],"--first-seed",str(j["first_seed"]),"--seeds",str(j["seeds"]),"--trace",str(int(j["trace"]))]
        record=command(cmd,j["id"],env)
        worlds=[json.loads((dest/f"seed-{s}"/"summary.json").read_text(encoding="utf-8")) for s in range(j["first_seed"],j["first_seed"]+j["seeds"])]
        a=[next(a for a in w["accounts"] if a["owner"]=="challenger") for w in worlds]
        write(dest/"done.json",record)
        print(f"RESULT {j['id']} rank={stats.mean(x['percentile'] for x in a):.2f} return={stats.mean((x['return_ratio']-1)*100 for x in a):.2f}%",flush=True)
    verify(plan)


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("command",choices=["build","prepare","run"])
    a=p.parse_args()
    budget=windows_build_budget() if os.name=="nt" else (os.cpu_count() or 1)
    env=dict(os.environ,RUSTC_WRAPPER="",CARGO_BUILD_JOBS=str(budget),RUST_TEST_THREADS="1")
    print(f"Shared CPU budget {budget}",flush=True)
    if a.command=="build": build(budget,env)
    elif a.command=="prepare": prepare(budget)
    else: run(env)


if __name__=="__main__": main()
