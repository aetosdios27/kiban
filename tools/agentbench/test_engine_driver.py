import json
import pathlib
import sys
import tempfile
import unittest

import engine_report
from engine_run import Campaign, cpus, load_summary, trial_preliminary


class EngineDriverTest(unittest.TestCase):
    def test_affinity_range_parser(self):
        self.assertEqual(cpus('1-4,8'),{1,2,3,4,8})
        with self.assertRaises(ValueError):
            cpus('4-1')

    def test_load_median_ignores_max_and_excludes_warmup(self):
        with tempfile.TemporaryDirectory() as folder:
            root=pathlib.Path(folder)
            rows=[{'label':'trial','epoch':t,'load':[value,20,30]} for t,value in [(1,20),(10,1),(11,1),(12,5),(20,40)]]
            (root/'load.jsonl').write_text('\n'.join(map(json.dumps,rows)))
            summary=load_summary(root,'trial',{'measurement_start_epoch':10,'measurement_end_epoch':12.1})
            self.assertEqual(summary['load'][0],{'median':1,'min':1,'max':5})
            self.assertFalse(trial_preliminary(summary))
            summary['load'][0]['median']=3
            self.assertTrue(trial_preliminary(summary))
            summary['load'][0]['median']=1
            summary['coverage']='partial'
            self.assertTrue(trial_preliminary(summary))

    def test_supervisor_rejects_failure_and_timeout(self):
        with tempfile.TemporaryDirectory() as folder:
            campaign=Campaign(pathlib.Path(folder),0.05)
            with self.assertRaisesRegex(RuntimeError,'Command failed'):
                campaign.command([sys.executable,'-c','raise SystemExit(2)'],'failure')
            with self.assertRaisesRegex(RuntimeError,'Timeout'):
                campaign.command([sys.executable,'-c','import time;time.sleep(30)'],'timeout')

    def test_short_trial_uses_actual_boundary_samples(self):
        with tempfile.TemporaryDirectory() as folder:
            root=pathlib.Path(folder)
            (root/'load.jsonl').write_text('')
            resource={'measurement_start_epoch':1,'measurement_end_epoch':1.1,
                'boundary_load_samples':[{'epoch':t,'load':[2,3,4]} for t in (1,1.1)]}
            summary=load_summary(root,'trial',resource)
            self.assertEqual(summary['samples'],2)
            self.assertFalse(trial_preliminary(summary))
            resource['measurement_end_epoch']=10
            resource['boundary_load_samples'][1]['epoch']=10
            self.assertTrue(trial_preliminary(load_summary(root,'trial',resource)))

    def test_invalid_campaign_has_no_comparison_table(self):
        with tempfile.TemporaryDirectory() as folder:
            root=pathlib.Path(folder)
            (root/'status.json').write_text(json.dumps({'status':'invalid','errors':1,'error':'oracle mismatch'}))
            engine_report.generate(root)
            report=(root/'REPORT.md').read_text()
            self.assertIn('oracle mismatch',report)
            self.assertNotIn('| Engine |',report)


if __name__=='__main__':
    unittest.main()
