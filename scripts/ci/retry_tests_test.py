import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from types import SimpleNamespace
import xml.etree.ElementTree as ET

spec = importlib.util.spec_from_file_location('retry_tests', Path(__file__).with_name('retry-tests.py'))
retry = importlib.util.module_from_spec(spec)
spec.loader.exec_module(retry)


def junit(path, outcomes):
    root = ET.Element('testsuites', tests=str(len(outcomes)), errors='0', skipped='0',
                      failures=str(sum(v == 'failed' for v in outcomes.values())))
    suites = {}
    for (binary, name), outcome in outcomes.items():
        suite = suites.setdefault(binary, ET.Element('testsuite', name=binary, tests='0', errors='0', skipped='0', failures='0'))
        suite.set('tests', str(int(suite.get('tests')) + 1))
        case = ET.SubElement(suite, 'testcase', classname=binary, name=name)
        if outcome == 'failed':
            suite.set('failures', str(int(suite.get('failures')) + 1))
            ET.SubElement(case, 'failure', type='test failure with exit code 101')
            summary = 'test result: FAILED. 0 passed; 1 failed; 0 ignored;'
        else:
            summary = 'test result: ok. 1 passed; 0 failed; 0 ignored;'
        status = 'FAILED' if outcome == 'failed' else 'ok'
        ET.SubElement(case, 'system-out').text = f'running 1 test\ntest {name} ... {status}\n{summary}'
    root.extend(suites.values())
    ET.ElementTree(root).write(path, encoding='unicode')


class RetryTest(unittest.TestCase):
    def setUp(self):
        # Exercise the same run-identity checks locally and on hosted CI.
        environment = patch.dict(os.environ, {'GITHUB_RUN_ID': '1234', 'GITHUB_RUN_ATTEMPT': '2'})
        environment.start()
        self.addCleanup(environment.stop)

    def save(self, state):
        retry.save({'run_id': '1234', 'attempt': '2', **state})

    def test_inventory_reports_exactly_the_selected_cases(self):
        # Enumerate every pass/fail subset; dropping or substituting any result
        # must fail even when the count is unchanged.
        ids = {('bin', 'test'+str(i)) for i in range(4)}
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'junit.xml'
            for mask in range(1 << len(ids)):
                outcomes = {key: ('failed' if mask & (1 << i) else 'passed')
                            for i, key in enumerate(sorted(ids))}
                junit(path, outcomes)
                self.assertEqual(retry.report(path, ids), outcomes)
                for missing in ids:
                    bad = dict(outcomes)
                    del bad[missing]
                    junit(path, bad)
                    with self.assertRaises(ValueError):
                        retry.report(path, ids)
                    bad[('other', missing[1])] = 'passed'
                    junit(path, bad)
                    with self.assertRaises(ValueError):
                        retry.report(path, ids)

    def test_missing_or_incomplete_results_are_not_success(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'junit.xml'
            with self.assertRaises(OSError):
                retry.report(path, {('a', 'b')})
            junit(path, {('a', 'b'): 'passed'})
            tree = ET.parse(path)
            tree.find('.//system-out').text = 'running 1 test'
            tree.write(path)
            with self.assertRaises(ValueError):
                retry.report(path, {('a', 'b')})

    def test_filters_escape_names(self):
        self.assertEqual(retry.expression([('a)b', 'x,y\\z')]),
                         r'(binary_id(=a\)b) & test(=x\,y\\z))')

    def test_no_failures_never_invokes_a_runner(self):
        with tempfile.TemporaryDirectory() as tmp, patch.object(retry, 'OUT', Path(tmp)):
            self.save({'type': 'unit', 'initial': 'complete', 'retry': 'not-run', 'failures': []})
            with patch.object(retry, 'command', side_effect=AssertionError('runner invoked')):
                retry.retry('unit')
            self.assertEqual(retry.load()['retry'], 'not-needed')

    def test_budget_is_per_slice_and_excess_never_invokes_a_runner(self):
        with tempfile.TemporaryDirectory() as tmp, patch.object(retry, 'OUT', Path(tmp)):
            failures = [{'binary': 'a', 'name': str(i), 'retry': 'not-run'} for i in range(21)]
            self.save({'type': 'unit', 'initial': 'complete', 'failures': failures})
            with patch.object(retry, 'command', side_effect=AssertionError('runner invoked')):
                with self.assertRaises(ValueError):
                    retry.retry('unit')
            self.assertEqual(retry.load()['retry'], 'budget-exceeded')

    def test_twenty_failures_remain_eligible(self):
        with tempfile.TemporaryDirectory() as tmp, patch.object(retry, 'OUT', Path(tmp)), \
                patch.object(retry, 'JUNIT', Path(tmp) / 'junit.xml'):
            ids = {('a', str(i)) for i in range(20)}
            self.save({'type': 'unit', 'initial': 'complete', 'failures': [
                {'binary': b, 'name': n, 'retry': 'not-run'} for b, n in ids]})
            def run(*args, **kwargs):
                junit(retry.JUNIT, {identity: 'passed' for identity in ids})
                return SimpleNamespace(returncode=0)
            with patch.object(retry, 'list_tests', return_value=(ids, set())), \
                    patch.object(retry, 'command', side_effect=run):
                retry.retry('unit')
            self.assertEqual(retry.load()['retry'], 'passed')

    def test_incomplete_initial_report_blocks_retry(self):
        with tempfile.TemporaryDirectory() as tmp, patch.object(retry, 'OUT', Path(tmp)):
            self.save({'type': 'unit', 'initial': 'incomplete', 'failures': []})
            with self.assertRaises(ValueError):
                retry.retry('unit')

    def test_selection_rejects_unexpected_filters(self):
        data = {'test-count': 2, 'rust-suites': {'a': {'status': 'listed', 'binary-id': 'a',
                'binary-path': '/tmp/a', 'testcases': {
                    'x': {'kind': 'test', 'ignored': False, 'filter-match': {'status': 'matches'}},
                    'y': {'kind': 'test', 'ignored': False, 'filter-match': {'status': 'mismatch', 'reason': 'expression'}}}}}}
        with self.assertRaises(ValueError):
            retry.inventory(data)
        self.assertEqual(retry.inventory(data, filtered=True)[0], {('a', 'x')})

    def test_retry_requires_exact_selection_and_success_evidence(self):
        failed = [{'binary': 'a', 'name': 'x', 'retry': 'not-run'}]
        with tempfile.TemporaryDirectory() as tmp, patch.object(retry, 'OUT', Path(tmp)), \
                patch.object(retry, 'JUNIT', Path(tmp) / 'junit.xml'):
            for selected in ({('a', 'x'), ('a', 'y')}, set()):
                self.save({'type': 'unit', 'initial': 'complete', 'failures': failed})
                with patch.object(retry, 'list_tests', return_value=(selected, set())), \
                        patch.object(retry, 'command', side_effect=AssertionError('extra test run')):
                    with self.assertRaises(ValueError):
                        retry.retry('unit')
                self.assertEqual(retry.load()['retry'], 'incomplete')

            for outcome, code in [('failed', 100), ('passed', 0)]:
                self.save({'type': 'unit', 'initial': 'complete', 'failures': failed})
                def run(*args, **kwargs):
                    junit(retry.JUNIT, {('a', 'x'): outcome})
                    return SimpleNamespace(returncode=code)
                with patch.object(retry, 'list_tests', return_value=({('a', 'x')}, set())), \
                        patch.object(retry, 'command', side_effect=run):
                    if outcome == 'failed':
                        with self.assertRaises(ValueError):
                            retry.retry('unit')
                    else:
                        retry.retry('unit')
                self.assertEqual(retry.load()['retry'], outcome)

    def test_missing_retry_report_stays_incomplete(self):
        with tempfile.TemporaryDirectory() as tmp, patch.object(retry, 'OUT', Path(tmp)), \
                patch.object(retry, 'JUNIT', Path(tmp) / 'missing.xml'):
            self.save({'type': 'unit', 'initial': 'complete',
                        'failures': [{'binary': 'a', 'name': 'x', 'retry': 'not-run'}]})
            with patch.object(retry, 'list_tests', return_value=({('a', 'x')}, set())), \
                    patch.object(retry, 'command', return_value=SimpleNamespace(returncode=0)):
                with self.assertRaises(OSError):
                    retry.retry('unit')
            self.assertEqual(retry.load()['retry'], 'incomplete')

    def test_stale_or_missing_run_identity_never_invokes_a_runner(self):
        with tempfile.TemporaryDirectory() as tmp, patch.object(retry, 'OUT', Path(tmp)):
            for identity in ({}, {'run_id': 'old', 'attempt': '2'},
                             {'run_id': '1234', 'attempt': '1'},
                             {'run_id': '1234'}, {'attempt': '2'}):
                retry.save({'type': 'unit', 'initial': 'complete', 'failures': [], **identity})
                with self.subTest(identity=identity), \
                        patch.object(retry, 'command', side_effect=AssertionError('runner invoked')):
                    with self.assertRaisesRegex(ValueError, 'another run or attempt'):
                        retry.retry('unit')


class ConsolidationTest(unittest.TestCase):
    def test_missing_invalid_and_incomplete_slices_are_distinct(self):
        spec = importlib.util.spec_from_file_location('consolidate', Path(__file__).with_name('consolidate-retries.py'))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for kind, platform, content in [
                ('unit', 'linux', {'version': 1, 'type': 'unit', 'platform': 'linux',
                                   'slice': 'all', 'run_id': '123', 'attempt': '2',
                                   'initial': 'incomplete', 'retry': 'not-run', 'failures': []}),
                ('unit', 'macos', {'version': 1}),
            ]:
                folder = root / f'retry-report-{kind}-{platform}-2'
                folder.mkdir()
                (folder / 'report.json').write_text(json.dumps(content))
            slices = module.consolidate(root, '2', '123')['slices']
            self.assertEqual(len(slices), 6)
            self.assertEqual((slices[0]['report'], slices[0]['initial']), ('received', 'incomplete'))
            self.assertEqual(slices[1]['report'], 'invalid')
            self.assertEqual([s['report'] for s in slices[2:]], ['missing'] * 4)

    def test_clean_recovered_and_contradictory_reports(self):
        spec = importlib.util.spec_from_file_location('consolidate', Path(__file__).with_name('consolidate-retries.py'))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for platform, failures, status in [
                ('linux', [], 'not-needed'),
                ('macos', [{'binary': 'a', 'name': 'x', 'retry': 'passed'}], 'passed'),
                ('windows', [{'binary': 'a', 'name': 'x', 'retry': 'not-run'}], 'not-needed'),
            ]:
                folder = root / f'retry-report-unit-{platform}-1'
                folder.mkdir()
                (folder / 'report.json').write_text(json.dumps({
                    'version': 1, 'type': 'unit', 'platform': platform, 'slice': 'all',
                    'run_id': '123', 'attempt': '1', 'selected': 2, 'doctests': 'passed',
                    'initial': 'complete', 'retry': status, 'failures': failures}))
            slices = module.consolidate(root, '1', '123')['slices']
            self.assertEqual([s.get('result', s['report']) for s in slices[:3]],
                             ['clean', 'recovered', 'invalid'])


if __name__ == '__main__':
    unittest.main()
