const assert=require('node:assert/strict');
const {researchRows}=require('./debugger/research-links.js');
const news=(low,high)=>({id:1,changes:[{stock:0,low,high}],public_tick:170,effective_tick:230,forecast:true});
const a={frames:[{tick:10,news:[news(1,5)]},{tick:100,news:[news(2,3)]}],fills:[{stock:0,receipt_tick:20,side:'Buy',qty:4},{stock:0,receipt_tick:250,side:'Sell',qty:4}],result:{liquidation_events:[{tick:250}]}};
assert.equal(researchRows(a,0,5).length,0);
let rows=researchRows(a,0,50);assert.equal(rows[0].latest.low,1);assert.equal(rows[0].buy,4);assert.equal(rows[0].sell,0);assert.equal(rows[0].risks.length,0);
rows=researchRows(a,0);assert.equal(rows.length,1);assert.equal(rows[0].updates.length,2);assert.equal(rows[0].sell,4);assert.equal(rows[0].risks.length,1);
assert.equal(researchRows(a,1).length,0);
console.log('Research receipt/as-of/deduplication checks passed');
