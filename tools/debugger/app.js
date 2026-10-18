/* Local, recorded observations only. No strategy orders are generated here. */
"use strict";
const $ = id => document.getElementById(id);
const esc = x => String(x ?? "—").replace(/[&<>"']/g, c => ({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;","'":"&#39;"}[c]));
const num = (x, d=2) => x == null || !Number.isFinite(+x) ? "—" : (+x).toLocaleString("zh-CN", {minimumFractionDigits:d,maximumFractionDigits:d});
const money = x => x == null ? "—" : num(x / 100);
const pct = x => num(x*100) + "%";
const signed = x => (x > 0 ? "+" : "") + num(x);
const color = x => x >= 0 ? "positive" : "negative";
const C = {up:"#ff8b79",down:"#5dd4ba",cyan:"#63d6cb",gold:"#e8c27a",purple:"#b3a0f4",muted:"#91a2ba",line:"#263449"};
const state = {index:null, account:null, compare:null, stock:0, period:5, tick:0, tab:"episodes", selectedEpisode:null, selectedPair:null, page:0, rows:[], zoom:[0,100], timer:null, cache:new Map(), request:0};
let manifest, market, priceChart, equityChart, buckets=[];
let priceLegend={"主动方向 30":false,"盘口不平衡":false,"动量（加权）":false};
let equityLegend={"现金":false,"累计费用":false};
const stockName = s => manifest.rules.stocks[s]?.name ?? s;
const meta = i => manifest.accounts.find(a => a.index === i);
const frameAt = (account,t) => account.frames[Math.min(t,account.frames.length-1)];
const reasonNames = {finish_exit:"正在清仓，信号反弹也不重新加仓",single_entry_hold:"单次入场模式：持有至退出，不补仓、不按权益微调",entry:"买盘信号高于入场门槛",exit:"信号转弱，目标降至零",hysteresis:"滞回区间，保持原目标",warmup:"尚未满足预热 / 成交量要求",end_exit:"接近期末，收敛目标至零",at_target:"仓位已达目标",action_limit:"动作额度耗尽",cancel_existing:"先撤已有订单",no_bid:"无买价，无法卖出",reserved_shares:"可卖股份不足",sell:"按最优买价发出 IOC 卖单",one_sided_book:"盘口单边，暂不买入",wide_spread:"价差超过限制，暂不买入",no_budget:"可用预算不足",buy:"按最优卖价发出 IOC 买单"};
function error(e) { $("error").hidden=false; $("error").textContent=String(e.message||e); console.error(e); }
function setBusy(busy) {document.querySelectorAll("main button,main select,main input").forEach(el=>{el.disabled=busy;});$("layout").setAttribute("aria-busy",String(busy));}
async function fetchJSON(url) { const r=await fetch(url); if(!r.ok) throw Error(`无法读取 ${url}: ${r.status}`); return r.json(); }
async function loadAccount(i) {
  if(state.cache.has(i)) return state.cache.get(i);
  const a=await fetchJSON(`/data/account-${i}.json.gz`);
  state.cache.set(i,a);
  for(const key of state.cache.keys()) { if(state.cache.size<=3) break; if(key!==state.index && key!==Number($("compare").value) && key!==i) state.cache.delete(key); }
  return a;
}
function renderAccounts() {
  const q=$("search").value.toLowerCase(), sort=$("sort").value;
  const all=manifest.accounts.filter(a=>(a.name+" "+a.strategy).toLowerCase().includes(q));
  all.sort((a,b)=>sort==="return"?b.result.return_ratio-a.result.return_ratio:sort==="drawdown"?b.max_drawdown-a.max_drawdown:sort==="trades"?b.result.stats.trades-a.result.stats.trades:(b.strategy==="swift")-(a.strategy==="swift")||a.index-b.index);
  $("accountCount").textContent=all.length;
  $("accounts").innerHTML=all.map(a=>`<button class="account ${state.index===a.index?"active":""}" data-index="${a.index}" aria-pressed="${state.index===a.index}"><span class="name">${esc(a.name)}</span><span class="row"><span class="sub">${esc(a.strategy)} · #${a.result.rank}</span><span class="${color(a.result.return_ratio-1)}">${pct(a.result.return_ratio-1)}</span></span><span class="sub">${a.result.stats.trades} 笔 · 回撤 ${pct(a.max_drawdown)}</span></button>`).join("");
}
async function selectAccount(i) {
  pause(); setBusy(true); const request=++state.request; state.index=i; renderAccounts();
  $("accountTitle").textContent=`正在读取 ${meta(i).name}…`;
  const account=await loadAccount(i); if(request!==state.request) return;
  state.account=account; state.page=0; state.selectedEpisode=null; state.selectedPair=null; $("focusEpisode").checked=false; $("selection").hidden=true;
  $("accountTitle").textContent=meta(i).name; $("strategy").textContent=meta(i).strategy.toUpperCase()+" / ACCOUNT "+i+(account.start.params.single_entry>0?" · 同一标的单次入场":"");
  setBusy(false);renderAll();
}
function renderMetrics() {
  const a=state.account, f=frameAt(a,state.tick), r=a.result, st=r.stats;
  const cards=[
    ["游标时点 · 标记权益",money(f.post.equity),`tick ${state.tick} · 现金 ${money(f.post.cash)}`],
    ["全局 · 最终结算收益率",pct(r.return_ratio-1),`初始权益 ${money(r.initial_equity)} · 排名 ${r.rank}`],
    ["全局 · 普通分红",money(st.dividends),`组合分红 ${money(st.combo_dividends)}`],
    ["全局 · 总费用",money(st.trade_fees+st.idle_fees+st.interest),`成交 ${money(st.trade_fees)} · 闲置 ${money(st.idle_fees)}`],
    ["全局 · 最大标记回撤",pct(a.max_drawdown),`${a.fills.length} 笔成交 · ${st.skipped} 次跳过`]
  ];
  $("metrics").innerHTML=cards.map(([label,value,sub])=>`<div class="metric"><div class="label">${label}</div><div class="value">${value}</div><div class="sub">${sub}</div></div>`).join("");
}
function aggregateBars() {
  const p=state.period, limit=$("asof").checked?state.tick:manifest.rules.ticks_total;
  const size=Math.floor(limit/p)+1;
  buckets=Array.from({length:size},(_,i)=>({tick:i*p,open:null,close:null,high:null,low:null,volume:0,frame:null}));
  for(const row of market.bars[state.stock]) {
    if(row[0]>limit) continue;
    const b=buckets[Math.floor(row[0]/p)]; if(!b) continue;
    if(row[1]!=null) { b.open??=row[1]; b.close=row[4]; b.high=Math.max(b.high??row[2],row[2]); b.low=Math.min(b.low??row[3],row[3]); }
    b.volume+=row[5]||0;
  }
  for(const f of state.account.frames) if(f.tick<=limit) buckets[Math.floor(f.tick/p)].frame=f;
  return buckets;
}
const axisStyle={axisLine:{lineStyle:{color:C.line}},axisLabel:{color:C.muted,fontSize:10},splitLine:{lineStyle:{color:C.line,opacity:.45}}};
function line(name,data,axis,color,extra={}) { return {name,type:"line",data,xAxisIndex:axis,yAxisIndex:axis,showSymbol:false,connectNulls:false,lineStyle:{width:1.2,color},itemStyle:{color},...extra}; }
function renderPrice() {
  const bs=aggregateBars(), a=state.account, s=state.stock, p=state.period, sw=meta(state.index).strategy==="swift";
  const labels=bs.map(b=>String(b.tick));
  const series=[{name:"价格",type:"candlestick",data:bs.map(b=>b.open==null?["-","-","-","-"]:[b.open,b.close,b.low,b.high].map(v=>v/100)),itemStyle:{color:C.up,color0:C.down,borderColor:C.up,borderColor0:C.down},markLine:{symbol:"none",silent:true,label:{show:false},lineStyle:{color:"#ffffff80",type:"dotted"},data:[{xAxis:String(Math.floor(state.tick/p)*p)}]}}];
  if($("quotes").checked) for(const [n,col,idx] of [["观察买一",C.down,0],["观察卖一",C.gold,1]]) series.push(line(n,bs.map(b=>b.frame?.quotes?.[s]?.[idx]?b.frame.quotes[s][idx]/100:null),0,col,{lineStyle:{width:1,color:col,opacity:.65}}));
  function fillSeries(account,name,side,col,compare=false) {
    return {name,type:"scatter",xAxisIndex:0,yAxisIndex:0,symbol:compare?"circle":"triangle",symbolRotate:side==="Sell"?180:0,symbolSize:compare?6:10,z:6,itemStyle:{color:col,opacity:.85},data:account.fills.filter(f=>f.stock===s && f.side===side && f.tick<=bs.at(-1).tick+p-1 && (!$("asof").checked||f.receipt_tick<=state.tick) && (compare||!$("focusEpisode").checked||state.selectedEpisode==null||f.episode_id===state.selectedEpisode)).map(f=>({value:[String(Math.floor(f.tick/p)*p),f.price/100],fillId:f.id,comparison:compare,details:`${name} ${f.qty} 股 · 撮合 ${f.tick} / 回报 ${f.receipt_tick}`})),tooltip:{formatter:v=>esc(v.data.details)+"<br>成交价 "+num(v.value[1])}};
  }
  if($("lineMode").value!=="none") series.push(tradeLinks(a, {stock:s,period:p,limit:$("asof").checked?state.tick:manifest.rules.ticks_total,
    mode:$("lineMode").value,includeInitial:$("initialLines").checked,selectedEpisode:state.selectedEpisode,selectedPair:state.selectedPair,onlySelected:$("focusEpisode").checked}));
  const linkCount=series.find(x=>x.type==="custom")?.data.length??0;
  $("linkNote").textContent=`${linkCount} 条连线 · `+(state.selectedEpisode==null?"点击连线或整轮记录聚焦":`整轮 #${state.selectedEpisode} · ${$("focusEpisode").checked?"只看该轮":"显示全部"}`);
  renderEpisodeBanner();
  series.push(fillSeries(a,"买入","Buy",C.up),fillSeries(a,"卖出","Sell",C.down));
  if(state.compare) series.push(fillSeries(state.compare,"对比买入","Buy",C.purple,true),fillSeries(state.compare,"对比卖出","Sell",C.gold,true));
  series.push({name:"收到新闻",type:"scatter",symbol:"pin",symbolSize:14,itemStyle:{color:C.gold},data:a.frames.filter(f=>f.tick<=($("asof").checked?state.tick:manifest.rules.ticks_total)).flatMap(f=>f.news.filter(n=>n.changes.some(c=>c.stock===s)).map(n=>({value:[String(Math.floor(f.tick/p)*p),(bs[Math.floor(f.tick/p)]?.high??f.quotes?.[s]?.[5]??manifest.rules.stocks[s].initial_price)/100],newsTick:f.tick,details:`收到新闻 #${n.id} · tick ${f.tick} · ${n.public?"公开":"提前披露"} · 生效 ${n.effective_tick}`}))),tooltip:{formatter:v=>esc(v.data.details)}});
  if($("newsTruth").checked) {
    for(const [field,label,col] of [["public_tick","新闻公开（事后）",C.gold],["effective","股息生效（事后）",C.purple]]) series.push({name:label,type:"scatter",symbol:"diamond",symbolSize:8,itemStyle:{color:col},data:market.news.filter(n=>n.changes.some(c=>c[0]===s)&&n[field]<=($("asof").checked?state.tick:manifest.rules.ticks_total)).map(n=>({value:[String(Math.floor(n[field]/p)*p),(bs[Math.floor(n[field]/p)]?.high??manifest.rules.stocks[s].initial_price)/100],details:`新闻 #${n.id} · ${label} tick ${n[field]} · 变化 ${n.changes.find(c=>c[0]===s)[1]}`})),tooltip:{formatter:v=>esc(v.data.details)}});
  }
  series.push({name:"成交量",type:"bar",xAxisIndex:1,yAxisIndex:1,data:bs.map(b=>({value:b.volume,itemStyle:{color:b.close>=b.open?C.up:C.down,opacity:.6}}))});
  if(sw) {
    for(const [key,name,col] of [["score","Swift 总信号",C.cyan],["flow10","主动方向 10",C.up],["flow30","主动方向 30",C.gold],["depth","盘口不平衡",C.purple],["momentum","动量（加权）","#7cabee"]]) series.push(line(name,bs.map(b=>b.frame?.swift?.[s]?.[key] == null?null:b.frame.swift[s][key]*(key==="momentum"?(a.start.params.momentum??5):1)),2,col));
    series.at(-5).markLine={symbol:"none",silent:true,label:{fontSize:9,color:C.muted,textBorderWidth:0,textBorderColor:"transparent",position:"insideEndTop",formatter:"{b}"},data:[{name:"入场",yAxis:a.start.params.entry??.15},{name:"退出",yAxis:a.start.params.exit??-.3}],lineStyle:{color:C.muted,width:1,type:"dashed"}};
  } else {
    series.push(line("观察成交方向（非内部信号）",bs.map(b=>{const q=b.frame?.quotes?.[s];return q?(q[9]-q[10])/Math.max(1,q[9]+q[10]):null;}),2,C.cyan));
  }
  series.push(line("持股",bs.map(b=>b.frame?.post.shares[s]??null),3,C.cyan,{step:"end",areaStyle:{color:C.cyan,opacity:.1}}));
  if(sw) series.push(line("目标股数",bs.map(b=>b.frame?.swift?.[s]?.target_shares??null),3,C.gold,{step:"end",lineStyle:{type:"dashed",color:C.gold,width:1}}));
  $("chartTitle").textContent=`${stockName(s)} · ${p} tick K 线 · ${sw?"Swift 信号与执行":"实际交易与观察"}`;
  priceChart.setOption({animation:false,backgroundColor:"transparent",textStyle:{color:C.muted},legend:{type:"scroll",top:0,textStyle:{color:C.muted,fontSize:10},selected:priceLegend},tooltip:{trigger:"axis",axisPointer:{type:"cross"},backgroundColor:"#172537",borderColor:C.line,textStyle:{color:"#e1eaf6",fontSize:11},confine:true},axisPointer:{link:[{xAxisIndex:"all"}]},grid:[{left:64,right:35,top:42,height:"39%"},{left:64,right:35,top:"49%",height:"9%"},{left:64,right:35,top:"63%",height:"13%"},{left:64,right:35,top:"81%",height:"10%"}],xAxis:[0,1,2,3].map(i=>({type:"category",data:labels,gridIndex:i,boundaryGap:true,...axisStyle,axisLabel:{...axisStyle.axisLabel,show:i===3},axisPointer:{label:{show:i===0}}})),yAxis:["元","量","信号","股"].map((name,i)=>({type:"value",name,gridIndex:i,scale:true,...axisStyle,nameTextStyle:{color:C.muted,fontSize:10},splitNumber:i===0?5:2})),dataZoom:[{type:"inside",filterMode:"weakFilter",xAxisIndex:[0,1,2,3],start:state.zoom[0],end:state.zoom[1]},{type:"slider",filterMode:"weakFilter",xAxisIndex:[0,1,2,3],start:state.zoom[0],end:state.zoom[1],bottom:0,height:15,borderColor:C.line,textStyle:{color:C.muted},dataBackground:{lineStyle:{color:C.cyan},areaStyle:{color:C.cyan}}}],series},true);
}
function renderEquity() {
  const a=state.account, limit=$("asof").checked?state.tick:manifest.rules.ticks_total;
  const curves=a.curves.filter(c=>c[0]<=limit), series=[];
  for(const [idx,name,col] of [[1,"标记权益",C.cyan],[2,"现金",C.purple],[3,"累计普通分红",C.gold],[6,"交易及库存残差",C.up],[5,"累计费用","#98a6bb"]]) series.push({name,type:"line",showSymbol:false,data:curves.map(c=>[c[0],c[idx]/100]),lineStyle:{color:col,width:1.4},itemStyle:{color:col}});
  if(state.compare) series.push({name:"对比权益收益率",type:"line",yAxisIndex:1,showSymbol:false,data:state.compare.curves.filter(c=>c[0]<=limit).map(c=>[c[0],(c[1]/state.compare.result.initial_equity-1)*100]),lineStyle:{color:C.purple,type:"dashed"},itemStyle:{color:C.purple}});
  if(limit===manifest.rules.ticks_total) series.push({name:"期末结算权益",type:"scatter",symbol:"diamond",symbolSize:11,itemStyle:{color:C.gold},data:[[limit,a.result.cash+a.result.terminal_value].map((v,i)=>i?v/100:v)]});
  series[0].markLine={symbol:"none",silent:true,label:{show:false},lineStyle:{color:"#ffffff70",type:"dotted"},data:[{xAxis:state.tick}]};
  equityChart.setOption({animation:false,textStyle:{color:C.muted},legend:{type:"scroll",textStyle:{color:C.muted,fontSize:10},selected:equityLegend},grid:{left:64,right:45,top:40,bottom:42},tooltip:{trigger:"axis",confine:true},xAxis:{type:"value",max:limit,...axisStyle},yAxis:[{type:"value",...axisStyle,scale:true,name:"元"},{type:"value",...axisStyle,scale:true,name:"%"}],dataZoom:[{type:"inside",start:state.zoom[0],end:state.zoom[1]}],series},true);
  $("pnlNote").textContent="交易及库存残差 = 标记权益 − 初始权益 − 普通分红 − 组合分红 + 总费用；含库存价格变化及可能的坏账核销，不等同已实现交易利润。期末结算按现金流终值，与标记市值分开。";
}
function kv(items) {return `<div class="kv">${items.map(([k,v])=>`<div><span>${esc(k)}</span><b>${esc(v)}</b></div>`).join("")}</div>`;}
function renderSnapshot() {
  const a=state.account, f=frameAt(a,state.tick), s=state.stock, q=f.quotes?.[s], sig=f.swift?.[s];
  $("inspectTitle").textContent=`tick ${f.tick} · ${f.end?"期末回报归集":f.decided?"决策快照":"本 tick 未决策"}`;
  let html=kv([["现金 前 → 后",`${money(f.pre?.cash)} → ${money(f.post.cash)}`],[`${stockName(s)} 持股 前 → 后`,`${f.pre?.shares[s]??"—"} → ${f.post.shares[s]}`],["观察参考 tick",f.reference_tick],["当 tick 动作 / 新拒单",`${f.actions.length} / ${f.post_rejects.length}`]]);
  html+=kv([["决策前杠杆",f.pre?num(f.pre.leverage,3)+" 倍":"—"],["该股本次收到普通分红",money(f.income.reduce((sum,i)=>sum+(i.Dividend?.stock===s?i.Dividend.amount:0),0))],["累计普通 / 组合分红",`${money(f.stats.dividends)} / ${money(f.stats.combo_dividends)}`],["累计成交费 / 利息",`${money(f.stats.trade_fees)} / ${money(f.stats.interest)}`]]);
  if(q) html+=kv([["观察买一 / 卖一",`${money(q[0]||null)} / ${money(q[1]||null)}`],["价差 / 中间价",q[0]&&q[1]?pct((q[1]-q[0])/((q[0]+q[1])/2)):"单边盘口"],["本次观察 主买 / 主卖量",`${q[9]} / ${q[10]}`],["每股每次派息（元）",num(q[6]/a.start.rules.dividend_scale/100,4)]]);
  if(sig) html+=`<div class="reason"><b>${esc(reasonNames[sig.signal_reason]||sig.signal_reason)}</b><br>${esc(reasonNames[sig.execution_reason]||sig.execution_reason)}</div>`+kv([["总信号",num(sig.score,4)],["目标比例 / 股数",`${pct(sig.target_fraction)} / ${sig.target_shares}`],["主动方向 10 / 30",`${num(sig.flow10,4)} / ${num(sig.flow30,4)}`],["盘口不平衡",num(sig.depth,4)],["动量 / EMA30",`${num(sig.momentum,5)} / ${money(sig.ema30)}`],["成交量 EMA10",num(sig.volume10,1)],["预算",money(sig.budget)],["与真实动作逐项核对",f.swift_verified?"一致":"不一致，请停止据此归因"]])+`<div class="small muted">score = 0.7 × flow10 + 0.3 × flow30 + ${a.start.params.depth??.3} × depth + ${a.start.params.momentum??5} × momentum。目标信号与实际成交分开记录。</div>`;
  else if(f.decided) html+='<div class="reason">此账户展示实际输入和动作；没有推断其未记录的内部交易理由。</div>';
  const book=f.books?.[s];
  if(book) html+=`<div class="mini-heading">当时可见的 10 档盘口 · 聚合数量，不含账户身份</div><div class="book">${["bids","asks"].map((side,i)=>`<div><b>${i?"卖盘":"买盘"}</b>${book[side].map(l=>`<div class="book-row"><span>${money(l.price)}</span><span>${l.qty}</span></div>`).join("")}</div>`).join("")}</div>`;
  const actions=f.actions.filter(x=>!x.Limit||x.Limit.stock===s);
  html+=`<div class="mini-heading">该标的动作 / 撤单</div><div class="orders">${actions.length?actions.map(x=>esc(JSON.stringify(x))).join("<br>"):"无动作"}</div>`;
  html+=`<details><summary>订单回报、拒单、实际观察逐笔与原始快照</summary><pre>${esc(JSON.stringify(f,null,2))}</pre></details>`;
  $("snapshot").innerHTML=html;
}
function setCursor(t,redraw=true) {
  if(!state.account) return;
  state.tick=Math.max(0,Math.min(manifest.rules.ticks_total,Math.round(Number(t)||0)));
  $("cursor").value=state.tick; $("tick").value=state.tick;
  renderMetrics();renderSnapshot();
  if(redraw) {
    if($("asof").checked) {renderPrice();renderEquity();}
    else {
      priceChart.setOption({series:[{name:"价格",markLine:{data:[{xAxis:String(Math.floor(state.tick/state.period)*state.period)}]}}]});
      equityChart.setOption({series:[{name:"标记权益",markLine:{data:[{xAxis:state.tick}]}}]});
    }
  }
}
function renderAll() {if(!state.account)return;renderMetrics();renderPrice();renderEquity();renderSnapshot();renderTable();}
function filtered(items) {return $("stockOnly").checked?items.filter(x=>x.stock===state.stock):items;}
const episodeStatus = e => (e.closed?"已平仓":"未平仓")+(e.adjusted?" · 库存调整":"")+(e.initial?" · 含初始库存":"");
function renderEpisodeBanner() {
  const e=state.account.episodes?.[state.selectedEpisode];
  $("episodeBanner").hidden=!e;if(!e)return;
  $("episodeBanner").innerHTML=`<span>${esc(state.account.start.account)} · 整轮 #${e.id} · ${stockName(e.stock)} · ${e.entry_tick} → ${e.exit_tick??"未平仓"} · 买 ${e.buy_qty} / 卖 ${e.sell_qty} 股 · <b class="${color(e.price_pnl??0)}">净价差 ${money(e.price_pnl)} 元</b></span><button id="prevEpisode">上一轮</button><button id="nextEpisode">下一轮</button><button id="episodeDetails">整轮明细 ↓</button>`;
  const same=state.account.episodes.filter(x=>x.stock===e.stock&&($("initialLines").checked||!x.initial));
  const index=same.findIndex(x=>x.id===e.id);
  $("prevEpisode").disabled=index<=0;$("nextEpisode").disabled=index<0||index>=same.length-1;
  $("prevEpisode").onclick=()=>selectEpisode(same[index-1].id);$("nextEpisode").onclick=()=>selectEpisode(same[index+1].id);
  $("episodeDetails").onclick=()=>{selectEpisode(e.id);$("selection").scrollIntoView({behavior:"smooth",block:"start"});};
}
function tableData() {
  const a=state.account, s=state.stock;
  switch(state.tab) {
  case "episodes": {
    const episodes=filtered(a.episodes??[]).filter(e=>!$("tradedOnly").checked||!e.initial);
    const sorting=$("rowSort").value;
    episodes.sort((a,b)=>sorting==="worst"?(a.price_pnl??Infinity)-(b.price_pnl??Infinity):sorting==="best"?(b.price_pnl??-Infinity)-(a.price_pnl??-Infinity):(a.exit_receipt_tick??Infinity)-(b.exit_receipt_tick??Infinity));
    return {headers:["整轮","标的","首次入场","最终出场","状态","买 / 卖股数","入 / 出均价（元）","净价差（元）","买 / 卖订单数","持有回报 ticks"],help:"从空仓到持仓再回到空仓为一轮，加仓和部分卖出都合并。均价按股数加权，净价差扣全部交易费；不含分红等现金流。未平仓 / 库存调整不冒充完整交易收益。点击一轮可看所有组成成交。",rows:episodes.map(e=>({action:"episode",key:e.id,cells:[`#${e.id}`,stockName(e.stock),e.entry_tick,e.exit_tick??"—",episodeStatus(e),`${e.buy_qty} / ${e.sell_qty}`,`${money(e.entry_price)} / ${money(e.exit_price)}`,money(e.price_pnl),`${e.buy_orders} / ${e.sell_orders}`,e.holding_ticks]}))};
  }
  case "fills":return {headers:["撮合 tick","回报 tick","标的","方向","价格 / 元","股数","费用 / 元","身份","后 30 tick 价格变化 / bp"],help:"只记录真实成交。点击行回到决策（主动成交）或收报时刻（被动成交）；未来价格变化是事后信息。",rows:filtered(a.fills).filter(f=>$("side").value==="all"||f.side===$("side").value).map(f=>({action:"fill",key:f.id,cells:[f.tick,f.receipt_tick,stockName(f.stock),f.side==="Buy"?"买入":"卖出",money(f.price),f.qty,money(f.fee),f.maker?"Maker":"Taker",num(f.forward_bps["30"])]}))};
  case "pairs": {
    const pairs=filtered(a.pairs.map((p,id)=>({...p,id}))).filter(p=>!$("tradedOnly").checked||!p.initial);
    const sorting=$("rowSort").value;
    pairs.sort((a,b)=>sorting==="worst"?(a.price_pnl??0)-(b.price_pnl??0):sorting==="best"?(b.price_pnl??0)-(a.price_pnl??0):a.exit_receipt_tick-b.exit_receipt_tick);
    return {headers:["标的","入场撮合","出场撮合","来源","股数","入价 / 元","出价 / 元","净价差 / 元","持有回报 ticks"],help:"FIFO 按实际回报顺序配对，部分成交会拆行；已扣两端交易费，不含分红、闲置费与利息。初始库存不是一次交易入场。",rows:pairs.map(p=>({action:"pair",key:p.id,cells:[stockName(p.stock),p.entry_tick,p.exit_tick,p.initial?"初始 / 调整库存":"买入成交",p.qty,money(p.entry_price),money(p.exit_price),money(p.price_pnl),p.holding_ticks]}))};
  }
  case "news": {
    let rows=[];
    for(const f of a.frames) {
      if($("eventKind").value!=="cash") {
        for(const n of f.news) if(!$("stockOnly").checked||!n.changes||n.changes.some(c=>c.stock===s)) rows.push({action:"frame",key:f.tick,cells:[f.tick,"收到新闻",JSON.stringify(n)]});
        for(const n of f.terminal_hints) rows.push({action:"frame",key:f.tick,cells:[f.tick,"收到终值提示",JSON.stringify(n)]});
      }
      if($("eventKind").value!=="news") for(const item of f.income) {const [k,v]=typeof item==="string"?[item,{}]:Object.entries(item)[0];if(k==="Dividend"&&($("stockOnly").checked&&v.stock!==s))continue;rows.push({action:"frame",key:f.tick,cells:[f.tick,k,JSON.stringify(v)]});}
    }
    if($("newsTruth").checked) for(const n of market.news) if(!$("stockOnly").checked||n.changes.some(c=>c[0]===s)) rows.push({action:"frame",key:n.effective,cells:[n.effective,"全局真值（事后）",JSON.stringify(n)]});
    rows.sort((a,b)=>a.key-b.key);
    return {headers:["回报 / 生效 tick","事件","原始内容（单位见说明）"],rows,help:`普通消息按实际收报时点；新闻股息变化单位为 1/${a.start.rules.dividend_scale} 分，现金流金额为分，终值提示是支付次数。公开与生效分开；全局真值为事后信息。`};
  }
  case "counterparties": {
    const sums=new Map();
    for(let st=0;st<market.trades.length;st++) if(!$("stockOnly").checked||st===s) for(const t of market.trades[st]) {
      const name=a.start.account; if(t[4]!==name&&t[5]!==name)continue;
      const other=t[4]===name?t[5]:t[4], buy=(t[4]===name)===(t[3].toLowerCase()==="buy");
      const v=sums.get(other)||{notional:0,buy:0,sell:0,count:0};v.notional+=t[1]*t[2];v[buy?"buy":"sell"]+=t[2];v.count++;sums.set(other,v);
    }
    return {headers:["对手","成交额 / 元","笔数","从其买入股数","向其卖出股数"],help:"身份只来自赛后全市场撮合日志；不是 Swift 决策时可见的数据。点击对手加入对比。",rows:[...sums].sort((a,b)=>b[1].notional-a[1].notional).map(([name,v])=>({action:"compare",key:manifest.accounts.find(a=>a.name===name)?.index,cells:[name,money(v.notional),v.count,v.buy,v.sell]}))};
  }
  case "stocks":return {headers:["标的","成交数","买入股数","卖出股数","FIFO 净价差 / 元","普通分红 / 元","期末股数"],help:"价差只含已配对部分；初始库存按开局价格计成本，未平仓库存及销毁股份不算卖出。点击标的切换图表。",rows:a.stocks.map(x=>({action:"stock",key:x.stock,cells:[stockName(x.stock),x.fills,x.buys,x.sells,money(x.paired_price_pnl),money(x.dividends),x.final_shares]}))};
  default:return {headers:["核对项","记录"],help:"现金 / 成交 / 手续费 / 股息 / Swift 动作误差应为零。原始 journal 和规则随运行目录保存。",rows:[...Object.entries(a.validation),["idle_cash_fees（分）",a.result.stats.idle_cash_fees],["idle_stock_fees（分）",a.result.stats.idle_stock_fees],["实际参数",JSON.stringify(a.start.params)],["被动库存调整次数",a.adjustments.length],["观察信息",manifest.clock_note],["源录制 SHA256",manifest.source_trace_sha256],["规则",JSON.stringify(manifest.rules)]].map(cells=>({cells}))};
  }
}
function renderTable() {
  if(!state.account)return;
  const data=tableData();state.rows=data.rows;state.headers=data.headers;
  $("tableHelp").textContent=data.help;$("sideLabel").hidden=state.tab!=="fills";$("rowSortLabel").hidden=!["pairs","episodes"].includes(state.tab);$("tradedOnlyLabel").hidden=!["pairs","episodes"].includes(state.tab);$("eventKindLabel").hidden=state.tab!=="news";
  const pages=Math.max(1,Math.ceil(data.rows.length/50));state.page=Math.min(state.page,pages-1);
  $("table").innerHTML=data.rows.length?`<table><thead><tr>${data.headers.map(h=>`<th>${esc(h)}</th>`).join("")}</tr></thead><tbody>${data.rows.slice(state.page*50,state.page*50+50).map(r=>`<tr ${r.action?`data-action="${r.action}" data-key="${r.key}" tabindex="0"`:""}>${r.cells.map(c=>`<td>${esc(c)}</td>`).join("")}</tr>`).join("")}</tbody></table>`:'<div class="empty">此筛选条件下没有记录。</div>';
  $("pageInfo").textContent=`${data.rows.length} 条 · ${state.page+1} / ${pages}`;$("pagePrev").disabled=state.page===0;$("pageNext").disabled=state.page+1>=pages;
}
function focusInterval(from,to) {
  const total=manifest.rules.ticks_total;
  state.zoom=[Math.max(0,from-30)/total*100,Math.min(total,Math.max(to+30,from+100))/total*100];
  renderPrice();renderEquity();
}
function selectFill(id,comparison=false) {
  const a=comparison?state.compare:state.account, f=a.fills[id];if(!f)return;
  state.selectedEpisode=comparison?null:f.episode_id??null;state.selectedPair=null;
  if(comparison)$("focusEpisode").checked=false;
  state.stock=f.stock;$("stock").value=f.stock;
  setCursor(f.decision_tick??f.receipt_tick,false);focusInterval(f.tick,f.receipt_tick);renderTable();
  $("selection").hidden=false;
  $("selection").innerHTML=`<h3>${comparison?"对比账户 · ":""}${esc(a.start.account)} · ${f.side==="Buy"?"买入":"卖出"} ${stockName(f.stock)}</h3>${kv([["真实成交",`${f.qty} 股 × ${money(f.price)} 元`],["交易费",money(f.fee)],["撮合 / 收报 tick",`${f.tick} / ${f.receipt_tick}`],["身份 / 订单",`${f.maker?"Maker":"Taker"} / ${f.order_id}`],["决策 tick",f.decision_tick??"被动成交，不能将撮合 tick 当成挂单时刻"],["未来 10 / 30 / 100 tick 价格变化（bp）",[10,30,100].map(x=>num(f.forward_bps[x])).join(" / ")]])}<p class="small muted">未来变化从收到成交回报之后计算，参考未来实际观察中间价；仅用于事后评价，卖出也保留原始价格变化方向。${comparison?"右侧决策快照仍属于主选账户。":""}</p><details><summary>原始成交</summary><pre>${esc(JSON.stringify(f,null,2))}</pre></details>`;
  if(!comparison&&f.episode_id!=null) {
    const button=document.createElement("button");button.textContent=`查看所属整轮 #${f.episode_id}`;button.onclick=()=>selectEpisode(f.episode_id);$("selection").prepend(button);
  }
}
function selectEpisode(id) {
  const a=state.account,e=a.episodes?.[id];if(!e)return;
  state.selectedEpisode=id;state.selectedPair=null;state.stock=e.stock;$("stock").value=e.stock;
  $("lineMode").value="episodes";$("focusEpisode").checked=true;if(e.initial)$("initialLines").checked=true;
  setCursor(e.exit_receipt_tick??manifest.rules.ticks_total,false);
  const fills=e.fill_ids.map(i=>a.fills[i]);
  focusInterval(Math.min(e.entry_tick,...fills.map(f=>f.tick)),Math.max(e.exit_tick??state.tick,...fills.map(f=>f.tick)));
  renderTable();$("selection").hidden=false;
  let held=e.initial_qty;
  const changes=new Map();for(const x of e.adjustments)changes.set(x.tick,(changes.get(x.tick)||0)+x.qty);
  const timeline=[];
  for(const f of fills)timeline.push({tick:f.receipt_tick,fill:f});
  for(const [tick,qty] of changes)timeline.push({tick,adjustment:qty});
  timeline.sort((a,b)=>a.tick-b.tick||Number(!a.fill)-Number(!b.fill));
  const rows=timeline.map(item=>{
    const f=item.fill;
    if(!f){held+=item.adjustment;return `<tr><td>${item.tick}</td><td colspan="4">库存调整 ${item.adjustment} 股（不是成交）</td><td>${held}</td></tr>`;}
    held+=f.side==="Buy"?f.qty:-f.qty;
    return `<tr data-fill="${f.id}" tabindex="0"><td>${f.tick} / ${f.receipt_tick}</td><td class="${f.side==="Buy"?"positive":"negative"}">${f.side==="Buy"?"买入":"卖出"}</td><td>${f.qty}</td><td>${money(f.price)}</td><td>${money(f.fee)}</td><td>${held}</td></tr>`;
  }).join("");
  $("selection").innerHTML=`<h3>整轮 #${e.id} · ${stockName(e.stock)} · ${episodeStatus(e)}</h3>`+
    kv([["首次入场 → 最终退出（撮合）",`${e.entry_tick} → ${e.exit_tick??"未平仓 / 无真实卖出"}`],["净交易价差（扣交易费）",money(e.price_pnl)],
      ["买入 → 卖出总股数",`${e.buy_qty} → ${e.sell_qty}`],["加权入价 → 加权出价",`${money(e.entry_price)} → ${money(e.exit_price)}`],
      ["买单 → 卖单数量",`${e.buy_orders} → ${e.sell_orders}`],["成交笔数 / 交易费",`${fills.length} / ${money(e.fees)} 元`],
      ["初始 / 最大 / 剩余股数",`${e.initial_qty} / ${e.peak_qty} / ${e.remaining}`],["收报持有 ticks",e.holding_ticks]])+
    `<p class="small muted">一轮按该账户收到的回报顺序重建。圆圈和方块连的是整轮加权均价，不代表那一刻能按均价成交；三角形保留实际买卖点。${e.initial?"开局库存以初始价格计成本。":""}分红、利息、闲置费不分配给本轮。${e.adjusted?"含非交易库存调整，完整价差留空。":""}</p>`+
    `<div class="episode-steps"><button id="episodeEntry">查看入场决策</button><button id="episodeExit">查看退出决策</button><button id="episodePairs">展开本轮 FIFO 连线</button></div>`+
    `<div class="episode-fills"><table><thead><tr><th>撮合 / 回报</th><th>方向</th><th>股数</th><th>价格 / 元</th><th>费用 / 元</th><th>逐笔持仓</th></tr></thead><tbody>${rows}</tbody></table></div>`;
  const first=fills.find(f=>f.side==="Buy"),last=fills.findLast(f=>f.side==="Sell");
  $("episodeEntry").onclick=()=>setCursor(first?.decision_tick??e.entry_receipt_tick);
  $("episodeExit").onclick=()=>setCursor(last?.decision_tick??e.exit_receipt_tick??state.tick);
  $("episodePairs").onclick=()=>{$("lineMode").value="pairs";renderPrice();};
  $("selection").onclick=event=>{const row=event.target.closest("tr[data-fill]");if(row)selectFill(+row.dataset.fill);};
  $("selection").onkeydown=event=>{if(event.key==="Enter"&&event.target.matches("tr[data-fill]"))selectFill(+event.target.dataset.fill);};
}
function selectPair(id) {
  const a=state.account,p=a.pairs[id];state.selectedEpisode=p.episode_id??null;state.selectedPair=id;$("focusEpisode").checked=true;$("lineMode").value="pairs";if(p.initial)$("initialLines").checked=true;state.stock=p.stock;$("stock").value=p.stock;setCursor(p.exit_receipt_tick,false);focusInterval(Math.min(p.entry_tick,p.exit_tick),Math.max(p.entry_tick,p.exit_tick));renderTable();
  const prices=a.frames.filter(f=>f.tick>=p.entry_receipt_tick&&f.tick<=p.exit_receipt_tick&&f.quotes).map(f=>{const q=f.quotes[p.stock];return q[0]&&q[1]?(q[0]+q[1])/2:q[5];});
  const min=prices.length?Math.min(...prices):null,max=prices.length?Math.max(...prices):null;
  $("selection").hidden=false;$("selection").innerHTML=`<h3>FIFO 配对 · ${stockName(p.stock)} · ${p.qty} 股</h3>${kv([["入场 / 出场撮合",`${p.entry_tick} → ${p.exit_tick}`],["入场 / 出场回报",`${p.entry_receipt_tick} → ${p.exit_receipt_tick}`],["入价 / 出价",`${money(p.entry_price)} → ${money(p.exit_price)}`],["净交易价差（扣两端手续费）",money(p.price_pnl)],["持有期间最大有利价格变化",max==null?"—":pct(max/p.entry_price-1)],["持有期间最大不利价格变化",min==null?"—":pct(min/p.entry_price-1)]])}<p class="small muted">${p.initial?"入场是开局或调整库存，不能当作策略买点。":"配对按回报顺序采用 FIFO，并非策略自己标记的事件仓位。"} 分红单独核算；有利 / 不利变化是事后观察中间价，不是可保证成交价。</p><p><button id="pairEntry">查看入场回报 tick</button> <button id="pairExit">查看出场回报 tick</button></p>`;
  $("pairEntry").onclick=()=>setCursor(p.entry_receipt_tick);$("pairExit").onclick=()=>setCursor(p.exit_receipt_tick);
}
async function changeCompare(value) {$("compare").value=value;const comparison=value===""?null:await loadAccount(+value);if($("compare").value!==value)return;state.compare=comparison;renderPrice();renderEquity();}
function handleRow(row) {
  const key=Number(row.dataset.key);switch(row.dataset.action) {
    case "episode":selectEpisode(key);break;case "fill":selectFill(key);break;case "pair":selectPair(key);break;case "frame":setCursor(key);break;
    case "stock":state.stock=key;state.selectedEpisode=null;state.selectedPair=null;$("focusEpisode").checked=false;$("selection").hidden=true;$("stock").value=key;renderAll();break;case "compare":if(Number.isFinite(key))changeCompare(String(key)).catch(error);break;
  }
}
function download(name,body,type) {const url=URL.createObjectURL(new Blob([body],{type}));const a=document.createElement("a");a.href=url;a.download=name;a.click();setTimeout(()=>URL.revokeObjectURL(url),1000);}
function exportCSV() {
  const cell=x=>{let v=String(x??"");if(typeof x==="string"&&/^[=+\-@]/.test(v))v="'"+v;return '"'+v.replace(/"/g,'""')+'"';};
  download(`${state.account.start.account}-${state.tab}.csv`,"\ufeff"+[state.headers,...state.rows.map(r=>r.cells)].map(r=>r.map(cell).join(",")).join("\r\n"),"text/csv;charset=utf-8");
}
function pause() {clearInterval(state.timer);state.timer=null;$("play").textContent="播放";}
function play() {if(state.timer)return pause();if(state.tick>=manifest.rules.ticks_total)setCursor(0);$("play").textContent="暂停";state.timer=setInterval(()=>{setCursor(state.tick+Number($("speed").value));if(state.tick>=manifest.rules.ticks_total)pause();},250);}
function nextFill(direction) {const fs=state.account.fills.filter(f=>f.stock===state.stock).map(f=>f.decision_tick??f.receipt_tick).sort((a,b)=>a-b);const next=direction>0?fs.find(t=>t>state.tick):fs.findLast(t=>t<state.tick);if(next!=null)setCursor(next);}
function bind() {
  $("search").oninput=renderAccounts;$("sort").onchange=renderAccounts;
  $("accounts").onclick=e=>{const b=e.target.closest("[data-index]");if(b)selectAccount(+b.dataset.index).catch(error);};
  $("stock").onchange=()=>{state.stock=+$("stock").value;state.page=0;state.selectedEpisode=null;state.selectedPair=null;$("focusEpisode").checked=false;$("selection").hidden=true;renderAll();};
  $("period").onchange=()=>{state.period=+$("period").value;renderPrice();};
  $("compare").onchange=()=>changeCompare($("compare").value).catch(error);
  for(const id of ["asof","quotes","newsTruth"]) $(id).onchange=()=>{renderPrice();renderEquity();renderTable();};
  for(const id of ["lineMode","initialLines","focusEpisode"]) $(id).onchange=renderPrice;
  $("clearEpisode").onclick=()=>{state.selectedEpisode=null;state.selectedPair=null;$("focusEpisode").checked=false;renderPrice();};
  $("resetZoom").onclick=()=>{state.zoom=[0,100];renderPrice();renderEquity();};
  $("sidebarToggle").onclick=()=>{const hidden=$("layout").classList.toggle("hide-sidebar");$("sidebarToggle").textContent=hidden?"展开账户":"收起账户";$("sidebarToggle").setAttribute("aria-expanded",!hidden);};
  $("cursor").oninput=e=>setCursor(e.target.value);$("tick").oninput=e=>setCursor(e.target.value);
  $("prevTick").onclick=()=>setCursor(state.tick-1);$("nextTick").onclick=()=>setCursor(state.tick+1);$("prevFill").onclick=()=>nextFill(-1);$("nextFill").onclick=()=>nextFill(1);$("play").onclick=play;
  for(const tab of document.querySelectorAll("[data-tab]"))tab.onclick=()=>{state.tab=tab.dataset.tab;state.page=0;document.querySelectorAll("[data-tab]").forEach(x=>x.setAttribute("aria-selected",x===tab));renderTable();};
  for(const id of ["side","stockOnly","rowSort","tradedOnly","eventKind"])$(id).onchange=()=>{state.page=0;renderTable();};
  $("pagePrev").onclick=()=>{state.page--;renderTable();};$("pageNext").onclick=()=>{state.page++;renderTable();};
  $("table").onclick=e=>{const row=e.target.closest("tr[data-action]");if(row)handleRow(row);};
  $("table").onkeydown=e=>{if(e.key==="Enter"&&e.target.matches("tr[data-action]"))handleRow(e.target);};
  $("exportRows").onclick=exportCSV;$("exportFrame").onclick=()=>download(`${state.account.start.account}-tick-${state.tick}.json`,JSON.stringify({source_trace_sha256:manifest.source_trace_sha256,account:state.account.start.account,stock:state.stock,rules:state.account.start.rules,params:state.account.start.params,frame:frameAt(state.account,state.tick)},null,2),"application/json");
  document.addEventListener("keydown",e=>{if(e.target.closest("input,select,textarea,button,summary"))return;if(e.key==="ArrowLeft"){e.preventDefault();setCursor(state.tick-1);}if(e.key==="ArrowRight"){e.preventDefault();setCursor(state.tick+1);}if(e.key===" "){e.preventDefault();play();}if(e.key==="[")nextFill(-1);if(e.key==="]")nextFill(1);});
  priceChart.on("click",e=>{if(e.data?.linkKind==="episode")selectEpisode(e.data.linkId);else if(e.data?.linkKind==="pair")selectPair(e.data.linkId);else if(e.data?.fillId!=null)selectFill(e.data.fillId,e.data.comparison);else if(e.data?.newsTick!=null)setCursor(e.data.newsTick);else if(e.seriesType!=="scatter"&&buckets[e.dataIndex])setCursor(buckets[e.dataIndex].tick);});
  priceChart.on("legendselectchanged",e=>{priceLegend=e.selected;});
  equityChart.on("legendselectchanged",e=>{equityLegend=e.selected;});
  priceChart.getZr().on("click",e=>{if(e.target)return;for(let i=0;i<4;i++)if(priceChart.containPixel({gridIndex:i},[e.offsetX,e.offsetY])){const value=priceChart.convertFromPixel({xAxisIndex:i},[e.offsetX,e.offsetY]);const b=buckets[Math.round(value[0]??value)];if(b)setCursor(b.tick);break;}});
  priceChart.on("updateAxisPointer",e=>{const info=e.axesInfo?.[0];if(!info)return;const b=buckets[Number(info.value)];if(b)$("ohlc").textContent=`tick ${b.tick}–${Math.min(b.tick+state.period-1,manifest.rules.ticks_total)}　开 ${money(b.open)}　高 ${money(b.high)}　低 ${money(b.low)}　收 ${money(b.close)}　量 ${b.volume}`;});
  priceChart.on("datazoom",()=>{const z=priceChart.getOption().dataZoom[0];state.zoom=[z.start,z.end];equityChart.dispatchAction({type:"dataZoom",start:z.start,end:z.end});});
  equityChart.getZr().on("click",e=>{if(equityChart.containPixel({gridIndex:0},[e.offsetX,e.offsetY]))setCursor(equityChart.convertFromPixel({xAxisIndex:0},e.offsetX));});
  const observer=new ResizeObserver(()=>{priceChart.resize();equityChart.resize();});observer.observe($("priceChart"));observer.observe($("equityChart"));
}
async function main() {
  setBusy(true);
  [manifest,market]=await Promise.all([fetchJSON("/data/manifest.json"),fetchJSON("/data/market.json.gz")]);
  $("runMeta").textContent=`seed ${manifest.seed_display??manifest.result.seed} · ${manifest.result.threads} 线程 · ${manifest.rules.ticks_total} ticks · ${manifest.accounts.length} 账户`;
  $("stock").innerHTML=manifest.rules.stocks.map((s,i)=>`<option value="${i}">${esc(s.name)}</option>`).join("");
  $("compare").innerHTML+=[...manifest.accounts].sort((a,b)=>(b.strategy==="swift")-(a.strategy==="swift")).map(a=>`<option value="${a.index}">${esc(a.name)}</option>`).join("");
  $("cursor").max=manifest.rules.ticks_total;$("tick").max=manifest.rules.ticks_total;
  const sort=document.createElement("label");sort.id="rowSortLabel";sort.innerHTML='排序 <select id="rowSort"><option value="time">出场顺序</option><option value="worst">亏损优先</option><option value="best">盈利优先</option></select>';$("sideLabel").after(sort);
  const traded=document.createElement("label");traded.id="tradedOnlyLabel";traded.innerHTML='<input id="tradedOnly" type="checkbox" checked>排除含初始库存的记录';sort.after(traded);
  const kind=document.createElement("label");kind.id="eventKindLabel";kind.innerHTML='事件 <select id="eventKind"><option value="news">新闻与提示</option><option value="cash">现金流</option><option value="all">全部</option></select>';traded.after(kind);
  priceChart=echarts.init($("priceChart"));equityChart=echarts.init($("equityChart"));bind();
  if(manifest.schema<2)error(Error("此录制尚未生成整轮操作数据，请先用 pack 命令重新打包。原成交数据仍可查看。"));
  const bad=manifest.accounts.filter(a=>["cash_error","fill_count_error","trade_fee_error","dividend_error","unknown_inventory","swift_mismatches"].some(k=>a.validation[k]!==0)||a.validation.frame_count!==a.validation.expected_frame_count);
  if(bad.length)error(Error(`${bad.length} 个账户的数据核对失败，不应据此下结论。请查看「核对与规则」。`));
  await selectAccount(manifest.accounts.find(a=>a.strategy==="swift")?.index??manifest.accounts[0].index);
}
main().catch(error);
