"""Auditable account journals and FIFO price-only pairings for the local debugger."""
from collections import deque
import csv
import gzip
import hashlib
import json
from pathlib import Path
from debugger_episodes import Episodes


def read_json(path):
    return json.loads(path.read_text(encoding="utf-8"))


def save_gzip(path, value):
    with gzip.open(path, "wt", encoding="utf-8", compresslevel=6) as f:
        json.dump(value, f, ensure_ascii=False, separators=(",", ":"), allow_nan=False)


def analyze_account(start, result, frames, final_marks):
    n=len(start["shares"])
    episodes=Episodes(start)
    lots=[deque() for _ in range(n)]
    for s,q in enumerate(start["shares"]):
        if q:
            lots[s].append(dict(qty=q,price=start["rules"]["stocks"][s]["initial_price"],fee=0,
                                tick=0,receipt_tick=0,fill_id=None,initial=True))
    fills=[]; pairs=[]; adjustments=[]; events=[]; incomes=[0]*n
    balance=start["cash"]
    trade_fees=0
    combo_income=0
    idle_cash_recorded=0
    peak=result["initial_equity"]
    max_drawdown=0
    curves=[]
    for frame in frames:
        tick=frame["tick"]
        if "idle_cash_fees" not in frame["stats"]:
            raise ValueError("Recording lacks idle cash/stock fee breakdown; record a new game with the current debug-run binary")
        # Only received fills are journalled. post_fills are a preview of the same
        # messages and MUST NOT be counted a second time. End flush closes the ledger.
        for raw in frame["fills"]:
            f=dict(raw,receipt_tick=tick,id=len(fills))
            fills.append(f)
            episodes.fill(f)
            s,q,p,fee=f["stock"],f["qty"],f["price"],f["fee"]
            trade_fees+=fee
            balance+=(-p*q-fee) if f["side"]=="Buy" else (p*q-fee)
            if f["side"]=="Buy":
                lots[s].append(dict(qty=q,price=p,fee=fee/q,tick=f["tick"],receipt_tick=tick,fill_id=f["id"],initial=False))
            else:
                left=q
                while left and lots[s]:
                    lot=lots[s][0]; take=min(left,lot["qty"])
                    pairs.append(dict(stock=s,qty=take,episode_id=f["episode_id"],entry_tick=lot["tick"],exit_tick=f["tick"],
                        entry_receipt_tick=lot["receipt_tick"],exit_receipt_tick=tick,
                        entry_price=lot["price"],exit_price=p,
                        price_pnl=None if lot.get("unknown") else (p-lot["price"]-lot["fee"]-fee/q)*take,
                        initial=lot["initial"],entry_fill=lot["fill_id"],exit_fill=f["id"],
                        holding_ticks=tick-lot["receipt_tick"]))
                    lot["qty"]-=take;left-=take
                    if not lot["qty"]:lots[s].popleft()
                if left:
                    adjustments.append(dict(tick=tick,stock=s,qty=-left,kind="unmatched_sell",cost=None))
        for income in frame["income"]:
            if isinstance(income,str):
                events.append(dict(tick=tick,kind=income,data={}))
                continue
            kind,data=next(iter(income.items()))
            if kind in ["Dividend","Combo","BadDebtWrittenOff"]:
                balance+=data["amount"]
                if kind=="Dividend":incomes[data["stock"]]+=data["amount"]
                if kind=="Combo":combo_income+=data["amount"]
            elif kind=="Interest":balance-=data["amount"]
            # IdleFee.paid includes destroyed STOCK value, not just cash.
            # The engine records its exact cash component separately below.
            if kind!="Dividend":events.append(dict(tick=tick,kind=kind,data=data))
        # Reconcile against the pre-decision inventory AFTER received messages.
        # Missing stock can be destroyed by fees or a risk writeoff. It is not a sale.
        inventory=frame["post"]["shares"] if frame["end"] else (frame["pre"]["shares"] if frame["pre"] else None)
        if inventory is not None:
            for s,target in enumerate(inventory):
                delta=target-sum(l["qty"] for l in lots[s])
                if delta:
                    episodes.adjustment(s,delta,tick)
                if delta<0:
                    left=-delta;removed=0
                    while left and lots[s]:
                        lot=lots[s][0];take=min(left,lot["qty"])
                        removed+=take*(lot["price"]+lot["fee"]);lot["qty"]-=take;left-=take
                        if not lot["qty"]:lots[s].popleft()
                    adjustments.append(dict(tick=tick,stock=s,qty=delta,kind="inventory_reduction",cost=removed))
                elif delta>0:
                    # Do not fabricate a profitable entry. Surface missing-cost inventory.
                    adjustments.append(dict(tick=tick,stock=s,qty=delta,kind="unknown_inventory",cost=None))
                    lots[s].append(dict(qty=delta,price=0,fee=0,tick=tick,receipt_tick=tick,fill_id=None,initial=True,unknown=True))
        post=frame["post"]
        if frame["end"]:
            post["gross"]=sum(q*p for q,p in zip(post["shares"],final_marks))
            post["equity"]=post["cash"]+post["gross"]
        eq=post["equity"];peak=max(peak,eq)
        drawdown=1-eq/peak if peak>0 else 0
        max_drawdown=max(max_drawdown,drawdown)
        st=frame["stats"]
        idle_cash=st["idle_cash_fees"]
        balance-=idle_cash-idle_cash_recorded
        idle_cash_recorded=idle_cash
        costs=st["trade_fees"]+st["idle_fees"]+st["interest"]
        profit=eq-result["initial_equity"]
        curves.append([tick,eq,post["cash"],st["dividends"],st["combo_dividends"],costs,
                       profit-st["dividends"]-st["combo_dividends"]+costs,drawdown])
    # Fill-to-order context and future observations are explicitly retrospective.
    decided={f["tick"]:f for f in frames if f["decided"]}
    for f in fills:
        decision=decided.get(f["tick"]) if not f["maker"] else None
        f["decision_tick"]=decision["tick"] if decision else None
        f["forward_bps"]={}
        for horizon in [10,30,100]:
            future=decided.get(f["receipt_tick"]+horizon)
            if future:
                quote=future["quotes"][f["stock"]]
                mid=(quote[0]+quote[1])/2 if quote[0] and quote[1] else quote[5]
                f["forward_bps"][str(horizon)]=(mid/f["price"]-1)*10000
    stock_stats=[]
    for s in range(n):
        mine=[p for p in pairs if p["stock"]==s]
        stock_stats.append(dict(stock=s,paired_price_pnl=sum(p["price_pnl"] for p in mine if p["price_pnl"] is not None),
            dividends=incomes[s],fills=sum(f["stock"]==s for f in fills),
            buys=sum(f["qty"] for f in fills if f["stock"]==s and f["side"]=="Buy"),
            sells=sum(f["qty"] for f in fills if f["stock"]==s and f["side"]=="Sell"),
            final_shares=result["shares"][s],open_lots=list(lots[s])))
    validation=dict(cash_error=balance-result["cash"],fill_count_error=len(fills)-result["stats"]["trades"],
        trade_fee_error=trade_fees-result["stats"]["trade_fees"],dividend_error=sum(incomes)-result["stats"]["dividends"],
        combo_error=combo_income-result["stats"]["combo_dividends"],
        frame_count=len(frames),expected_frame_count=start["rules"]["ticks_total"]+1,
        unknown_inventory=sum(a["kind"] in ["unknown_inventory","unmatched_sell"] for a in adjustments),
        swift_mismatches=sum(f["swift_verified"] is False for f in frames))
    return dict(start=start,result=result,frames=frames,fills=fills,pairs=pairs,adjustments=adjustments,
        episodes=episodes.finish(fills,frames[-1]["tick"]),
        events=events,curves=curves,stocks=stock_stats,max_drawdown=max_drawdown,validation=validation)


def build(run_dir):
    run_dir=Path(run_dir).resolve()
    manifest=read_json(run_dir/"trace-manifest.json")
    dest=run_dir/"data";dest.mkdir(exist_ok=True)
    rules=manifest["starts"][0]["start"]["rules"]
    names=[s["name"] for s in rules["stocks"]]
    stocks={name:i for i,name in enumerate(names)}
    bars=[[] for _ in names]
    with (run_dir/"market/bars.csv").open(encoding="utf-8",newline="") as f:
        for row in csv.DictReader(f):
            bars[stocks[row["stock"]]].append([int(row["tick"]),*[int(row[k]) if row[k] else None for k in ["open","high","low","close","volume","notional","dividend"]]])
    trades=[[] for _ in names]
    with (run_dir/"market/trades.csv").open(encoding="utf-8",newline="") as f:
        for i,row in enumerate(csv.DictReader(f)):
            trades[stocks[row["stock"]]].append([int(row["tick"]),int(row["price"]),int(row["qty"]),row["aggressor"],row["taker"],row["maker"],i])
    with (run_dir/"market/news.csv").open(encoding="utf-8",newline="") as f:
        news=[]
        for row in csv.DictReader(f):
            news.append(dict(id=int(row["id"]),layer=row["layer"],first_reveal=int(row["first_reveal"]),
                public_tick=int(row["public_tick"]),effective=int(row["effective"]),
                changes=[[stocks[s],int(d)] for s,d in (x.rsplit(":",1) for x in row["changes"].split())]))
    save_gzip(dest/"market.json.gz",dict(bars=bars,trades=trades,news=news))
    accounts=[]
    for meta in sorted(manifest["starts"],key=lambda m:m["start"]["account_index"]):
        start=meta["start"];idx=start["account_index"]
        result=next(a for a in manifest["result"]["accounts"] if a["account"]==start["account"])
        with (run_dir/f"trace/account-{idx}.jsonl").open(encoding="utf-8") as f:
            frames=[json.loads(line) for line in f]
        account=analyze_account(start,result,frames,manifest["result"]["final_marks"])
        save_gzip(dest/f"account-{idx}.json.gz",account)
        accounts.append(dict(index=idx,name=start["account"],strategy=meta["strategy"],result=result,
                             validation=account["validation"],max_drawdown=account["max_drawdown"]))
        print(f"Pack {idx+1}/{len(manifest['starts'])} {start['account']}: {account['validation']}",flush=True)
    output=dict(schema=2,rules=rules,result=manifest["result"],accounts=accounts,seed_display=str(manifest["result"]["seed"]),
        quote_fields=manifest["quote_fields"],clock_note=manifest["clock_note"],source_scenario=manifest["source_scenario"],
        source_trace_sha256=hashlib.sha256((run_dir/"trace-manifest.json").read_bytes()).hexdigest())
    (dest/"manifest.json").write_text(json.dumps(output,ensure_ascii=False,separators=(",",":")),encoding="utf-8")
    bad=[a for a in accounts if any(a["validation"][k] for k in ["cash_error","fill_count_error","trade_fee_error","dividend_error","combo_error","unknown_inventory","swift_mismatches"])
         or a["validation"]["frame_count"]!=a["validation"]["expected_frame_count"]]
    print(f"Built {dest}; {len(bad)} accounts with validation discrepancies",flush=True)
    if bad:
        raise ValueError("Journal validation failed; inspect data/manifest.json before using this recording")
    return output
