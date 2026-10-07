import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock
import time

root = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('campaign', root / 'run.py')
campaign = importlib.util.module_from_spec(spec)
spec.loader.exec_module(campaign)

class CampaignTests(unittest.TestCase):
    def test_orders_are_reproducible_and_paired(self):
        plan = campaign.validate(json.loads((root/'plans/smoke.json').read_text()))
        a = [(c['name'],r,e) for c,r,e in campaign.jobs(plan)]
        b = [(c['name'],r,e) for c,r,e in campaign.jobs(plan)]
        self.assertEqual(a,b)
        self.assertEqual(len(a),42)
        for i in range(0,len(a),2):
            self.assertEqual(a[i][:2],a[i+1][:2])
            self.assertEqual({a[i][2],a[i+1][2]},{'kiban','rocksdb'})
    def test_missing_parameter_is_not_defaulted(self):
        plan = json.loads((root/'plans/smoke.json').read_text())
        del plan['cases'][0]['args']['seed']
        with self.assertRaises(ValueError):campaign.validate(plan)
    def test_timeout_is_invalid_and_reaps_child(self):
        with tempfile.TemporaryFile(mode='w+') as log:
            result=campaign.supervise(['python3','-c','import time; time.sleep(5)'],log,root,0.1)
        self.assertNotEqual(result['exit_code'],0)
        self.assertEqual(result['invalid_reason'],'wall-timeout')
    def test_clock_gap_invalidates_and_reaps_child(self):
        real_time = time.time
        calls = 0
        def clock():
            nonlocal calls
            calls += 1
            return real_time() + (10 if calls > 1 else 0)
        with tempfile.TemporaryFile(mode='w+') as log, mock.patch.object(campaign.time, 'time', clock):
            result=campaign.supervise(['python3','-c','import time; time.sleep(5)'],log,root,20)
        self.assertNotEqual(result['exit_code'],0)
        self.assertEqual(result['invalid_reason'],'suspend-or-wall-clock-jump')
    def test_nonzero_exit_is_preserved(self):
        with tempfile.TemporaryFile(mode='w+') as log:
            result=campaign.supervise(['python3','-c','raise SystemExit(7)'],log,root,5)
        self.assertEqual(result['exit_code'],7)
        self.assertIsNone(result['invalid_reason'])
    def test_all_prepared_plans_validate(self):
        for p in (root/'plans').glob('*.json'):
            campaign.validate(json.loads(p.read_text()))

if __name__=='__main__':unittest.main()
