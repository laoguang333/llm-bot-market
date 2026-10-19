/* Received information only. Temporal association is not strategy attribution. */
(function(root){
  function researchRows(account, stock, limit=Infinity) {
    const map=new Map();
    for(const f of account.frames) {
      if(f.tick>limit)continue;
      for(const n of f.news??[]) {
        const c=n.changes.find(c=>c.stock===stock);if(!c)continue;
        let row=map.get(n.id);
        if(!row){row={id:n.id,stock,received:f.tick,publicTick:n.public_tick,effective:n.effective_tick,forecast:!!n.forecast,updates:[]};map.set(n.id,row);}
        row.updates.push({tick:f.tick,low:c.low,high:c.high,public:n.public,forecast:!!n.forecast});
      }
    }
    return [...map.values()].map(row=>{
      const end=Math.min(limit,row.effective+100);
      const fills=account.fills.filter(f=>f.stock===stock&&f.receipt_tick>=row.received&&f.receipt_tick<=end);
      return {...row,latest:row.updates.at(-1),fills,buy:fills.filter(f=>f.side==='Buy').reduce((n,f)=>n+f.qty,0),
        sell:fills.filter(f=>f.side==='Sell').reduce((n,f)=>n+f.qty,0),
        risks:(account.result.liquidation_events??[]).filter(e=>e.tick>=row.received&&e.tick<=end)};
    });
  }
  root.researchRows=researchRows;
  if(typeof module!=='undefined')module.exports={researchRows};
})(globalThis);
