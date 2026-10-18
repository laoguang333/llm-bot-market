/* One custom series keeps thousands of pairings zoomable without one series per trade. */
"use strict";
function tradeLinks(account, {stock, period, limit, mode, includeInitial, selectedEpisode, selectedPair, onlySelected}) {
  const whole=mode==="episodes";
  const source=whole?(account.episodes??[]):account.pairs;
  const data=source.map((item,id)=>({item,id})).filter(({item:e})=>
    e.stock===stock && (includeInitial||!e.initial) &&
    (!whole||e.closed) && e.price_pnl!=null && e.entry_price!=null && e.exit_price!=null && e.exit_tick!=null &&
    e.entry_receipt_tick<=limit && e.exit_receipt_tick<=limit &&
    (!onlySelected||selectedEpisode==null||(whole?e.id:e.episode_id)===selectedEpisode)
  ).map(({item:e,id})=>({
    value:[Math.floor(e.entry_tick/period),e.entry_price/100,Math.floor(e.exit_tick/period),e.exit_price/100],
    linkKind:whole?"episode":"pair",linkId:id,
    selected:whole?e.id===selectedEpisode:id===selectedPair,
    pnl:e.price_pnl,initial:e.initial,
    details:`${whole?"整轮 #"+e.id:"FIFO 配对"} · ${e.entry_tick} → ${e.exit_tick}<br>`+
      `${whole?"加权入 / 出均价":"入 / 出成交价"} ${money(e.entry_price)} → ${money(e.exit_price)}<br>`+
      `${whole?"买入 / 卖出 "+e.buy_qty+" / "+e.sell_qty:e.qty} 股 · 净价差 ${money(e.price_pnl)} 元<br>`+
      `已扣交易费，不含分红${e.initial?" · 含开局库存成本":""} · 点击查看明细`
  }));
  return {name:whole?"整轮进出场":"FIFO 进出场",type:"custom",coordinateSystem:"cartesian2d",
    xAxisIndex:0,yAxisIndex:0,clip:true,z:5,encode:{x:[0,2],y:[1,3]},data,
    tooltip:{trigger:"item",formatter:v=>v.data.details},
    renderItem(params,api) {
      const item=data[params.dataIndex], start=api.coord([api.value(0),api.value(1)]), end=api.coord([api.value(2),api.value(3)]);
      const stroke=item.pnl==null?C.muted:item.pnl>0?C.up:item.pnl<0?C.down:C.muted;
      const width=item.selected?3:whole?2:1.2, opacity=item.selected?1:.75;
      const shape={x1:start[0],y1:start[1],x2:end[0],y2:end[1]};
      const children=[
        {type:"line",shape,style:{stroke,lineWidth:Math.max(10,width),opacity:.01}},
        {type:"line",shape,style:{stroke,lineWidth:width,opacity,lineDash:item.initial?[5,4]:null}},
        {type:"circle",shape:{cx:start[0],cy:start[1],r:item.selected?4:2.5},style:{stroke,fill:"#111b29",lineWidth:1.5}},
        {type:"rect",shape:{x:end[0]-3,y:end[1]-3,width:6,height:6},style:{fill:stroke}}
      ];
      if(item.selected) children.push({type:"text",silent:true,x:(start[0]+end[0])/2,y:(start[1]+end[1])/2-24,
        style:{text:`${whole?"整轮":"配对"} ${money(item.pnl)} 元`,fill:stroke,fontSize:12,align:"center",backgroundColor:"#111b29e6",padding:[3,5]}});
      return {type:"group",children};
    }};
}
