"""The repair matrix must change only the two advertised strategy parameters."""
from copy import deepcopy
from pathlib import Path
import tomllib
import unittest

ROOT=Path(__file__).resolve().parents[1]


class RepairTests(unittest.TestCase):
    def test_factorial_preserves_world_and_assets(self):
        base=tomllib.loads((ROOT/'scenarios/ecology-v22-base-none.toml').read_text())
        for arm in ('control','noise','coral','both'):
            expected=deepcopy(base)
            for b in expected['bots']:
                if b['bot']=='noise' and arm in ('noise','both'): b.setdefault('params',{})['max_qty']=4.0
                if b['bot']=='coral' and arm in ('coral','both'): b.setdefault('params',{})['leverage']=1.0
            actual=tomllib.loads((ROOT/f'scenarios/foundation-repair-v22-base-{arm}.toml').read_text())
            self.assertEqual(actual,expected)

    def test_competitors_replace_only_named_slots(self):
        for arm in ('control','noise','coral','both'):
            base=tomllib.loads((ROOT/f'scenarios/foundation-repair-v22-base-{arm}.toml').read_text())
            for kind,n in (('swift',1),('tide',1),('almanac',1),('all',3)):
                candidate=tomllib.loads((ROOT/f'scenarios/foundation-repair-v22-{kind}-{arm}.toml').read_text())
                changed=0
                for a,b in zip(base['bots'],candidate['bots']):
                    if a==b: continue
                    changed+=1
                    self.assertTrue(a['owner'].startswith('slot-'))
                    self.assertEqual({k:v for k,v in a.items() if k not in ('bot','params')},
                                     {k:v for k,v in b.items() if k not in ('bot','params')})
                self.assertEqual(changed,n)
                self.assertEqual({k:v for k,v in base.items() if k!='bots'},
                                 {k:v for k,v in candidate.items() if k!='bots'})


if __name__=='__main__':unittest.main()
