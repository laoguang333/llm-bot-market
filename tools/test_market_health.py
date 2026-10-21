"""Checks for diagnostics that can otherwise make a broken market look healthy."""
from copy import deepcopy
import unittest
import tomllib
from market_health import rates_and_events,liquidity,quote_window,information_response,aggregate
from research_ecology_v22 import foundation_groups,assemble,validate_generated,CORE


class HealthTests(unittest.TestCase):
    def setUp(self):
        self.cfg={'ticks':1000,'world':{'dividend_scale':50},'stocks':[{'name':'A','dividend':100}]}

    def book(self,tick,bid=99,ask=101,qty=20):
        return dict(stock='A',tick=str(tick),bid=str(bid) if bid else '',ask=str(ask) if ask else '',
                    bid_shares_2pct=str(qty),ask_shares_2pct=str(qty))

    def test_rates_change_at_effective_not_disclosure(self):
        news=[dict(id='0',first_reveal='10',public_tick='70',effective='130',changes='A:-100')]
        rates,divs,_=rates_and_events(self.cfg,news)
        self.assertEqual(rates['A'][129],100)
        self.assertEqual(rates['A'][130],0)
        self.assertEqual(divs['A'][131]-divs['A'][130],0)

    def test_missing_side_counts_against_depth_and_coverage(self):
        rates,_,_=rates_and_events(self.cfg,[])
        books={('A',0):self.book(0),('A',10):self.book(10,ask=0)}
        h=liquidity(self.cfg,books,rates,{('A',0)})['normal']
        self.assertEqual(h['both'],.5)
        self.assertEqual(h['depth10'],.5)
        self.assertEqual(h['spread'],.02)
        self.assertEqual(h['active'],1)

    def test_quote_window_cannot_use_future_or_fill_missing_side(self):
        books={('A',t):self.book(t) for t in (0,10,20,30)}
        books[('A',30)]=self.book(30,999,1001)
        self.assertEqual(quote_window(books,'A',29),(10,100))
        books[('A',10)]['ask']=''
        self.assertIsNone(quote_window(books,'A',29))

    def test_signed_news_response_and_overlap(self):
        # Zero carry isolates the price-response calculation.
        cfg=deepcopy(self.cfg);cfg['stocks'][0]['dividend']=0
        books={('A',t):self.book(t,109 if t>=350 else 99,111 if t>=350 else 101) for t in range(0,1000,10)}
        ee={'A':[dict(id=1,delta=10,first=300,public=360,effective=420)]}
        out=information_response(cfg,ee,{'A':[0.]*1001},books)
        self.assertEqual(out['used'],1)
        self.assertEqual(out['hit'],1)
        self.assertAlmostEqual(out['excess'],.1)
        ee['A'][0]['delta']=-10
        self.assertEqual(information_response(cfg,ee,{'A':[0.]*1001},books)['hit'],0)
        ee['A'].append(dict(id=2,delta=10,first=310,public=370,effective=430))
        self.assertEqual(information_response(cfg,ee,{'A':[0.]*1001},books)['used'],0)

    def test_repeats_do_not_count_as_independent_news(self):
        rows=[dict(arm='a',seed=s,metrics=dict(discovery={'used':1,'event_ids':['A:1']},
                 audits=20,errors=0,disabled=0,skipped=0)) for s in (1,1,2,2)]
        a=aggregate(rows)['a']
        self.assertEqual(a['event_samples'],2)
        self.assertEqual(a['total_audits'],80)

    def test_slots_keep_assets_and_ordinary_information(self):
        src=[dict(bot='horizon',owner='horizon-incumbent',cash=20000000,shares_each=200,count=1,
                  forecast_access={'horizon':1000},params={'research':1}),
             dict(bot='hunter',cash=2000000,shares_each=20,count=4,count_max=10)]
        new=foundation_groups(src)
        for a,b in zip(src,new):
            for k in ('cash','shares_each','count','count_max'): self.assertEqual(a.get(k),b.get(k))
            self.assertIn(b['bot'],CORE)
            self.assertNotIn('forecast_access',b)
        self.assertEqual(new[0]['owner'],'slot-horizon-incumbent')
        self.assertNotIn('owner',new[1])

    def test_generator_keeps_capital_table_after_bot_array(self):
        source='ticks = 1000\n[fees]\nidle_rate = 0.006\ncredit_ratio = 0.01\n[[bots]]\nbot = "value"\ncount = 1\n[capital_redistribution]\nfrom_bot = "grid_v2"\ncash_each = 20\nshares_each = 10\nrecipients = { value = 1 }\n'
        cfg=tomllib.loads(source)
        rebuilt=assemble(source.split('[[bots]]')[0],cfg['bots'],cfg['capital_redistribution'])
        validate_generated(rebuilt,cfg,cfg['bots'],'current')
        self.assertEqual(tomllib.loads(rebuilt)['capital_redistribution'],cfg['capital_redistribution'])
        with self.assertRaises(AssertionError):
            validate_generated(source.split('[capital_redistribution]')[0],cfg,cfg['bots'],'current')


if __name__=='__main__': unittest.main()
