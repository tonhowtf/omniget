import importlib.util
import json
import os
import pathlib
import tempfile
import unittest
from unittest.mock import patch
import prepared

class PreparedContracts(unittest.TestCase):
    def test_edit_and_revert_during_build_is_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            result=prepared.subprocess.CompletedProcess([],0,'','')
            with patch.object(prepared,'sources',return_value='same bytes'), patch.object(prepared,'source_stamp',side_effect=[['old mtime'],['new mtime']]), patch.object(prepared.subprocess,'run',return_value=result):
                with self.assertRaisesRegex(RuntimeError,'SOURCES_CHANGED_DURING_BUILD'):
                    prepared.prepare(d)
    def test_stale_manifest_fails_before_any_process(self):
        with tempfile.TemporaryDirectory(prefix='prepared path ') as d:
            p=pathlib.Path(d)/'manifest.json'
            p.write_text(json.dumps({'root':str(prepared.ROOT),'sourceDigest':'old'}))
            with patch.object(prepared,'sources',return_value='new'), patch.object(prepared.subprocess,'run') as run:
                with self.assertRaisesRegex(RuntimeError,'STALE_PREPARATION'):
                    prepared.run_tests('omniget','case',{'OMNIGET_PREPARED_TESTS':str(p)})
                run.assert_not_called()
    def test_prepared_execution_has_no_cargo_and_rejects_zero_tests(self):
        with tempfile.TemporaryDirectory(prefix='prepared path ') as d:
            root=pathlib.Path(d);exe=root/'test binary';exe.write_bytes(b'binary')
            manifest={'root':str(prepared.ROOT),'sourceDigest':'current','configuration':{},'rustLib':d,
                      'artifacts':{'omniget':{'path':str(exe),'digest':prepared.digest(exe),'deps':d,'cwd':d}}}
            p=root/'manifest.json';p.write_text(json.dumps(manifest));env={'OMNIGET_PREPARED_TESTS':str(p)}
            result=prepared.subprocess.CompletedProcess([],0,'test result: ok. 1 passed; 0 failed;','')
            with patch.object(prepared,'sources',return_value='current'),patch.object(prepared.subprocess,'run',return_value=result) as run:
                cmd,_=prepared.run_tests('omniget','case',env)
                self.assertEqual(cmd,[str(exe),'case','--test-threads=1']);self.assertEqual(run.call_args.kwargs['cwd'],d)
                result.stdout='test result: ok. 0 passed; 0 failed;'
                with self.assertRaisesRegex(RuntimeError,'ZERO_TESTS_EXECUTED'):prepared.run_tests('omniget','missing',env)
                exe.write_bytes(b'changed')
                with self.assertRaisesRegex(RuntimeError,'ARTIFACT_CHANGED'):prepared.run_tests('omniget','case',env)
if __name__=='__main__':unittest.main()
