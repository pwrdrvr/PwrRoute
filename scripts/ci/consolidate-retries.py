#!/usr/bin/env python3
"""Produce one small report, including every absent or malformed CI slice."""
import json
from pathlib import Path
import sys

EXPECTED = [(kind, platform) for kind in ('unit', 'e2e')
            for platform in ('linux', 'macos', 'windows')]


def consolidate(directory, attempt, run_id='local'):
    slices = []
    for kind, platform in EXPECTED:
        path = directory / f'retry-report-{kind}-{platform}-{attempt}' / 'report.json'
        try:
            data = json.loads(path.read_text(encoding='utf-8'))
            if (data['version'], data['type'], data['platform'], data['slice']) != (1, kind, platform, 'all'):
                raise ValueError('slice identity mismatch')
            if data['run_id'] != run_id or data['attempt'] != attempt:
                raise ValueError('stale report')
            if data['initial'] not in ('complete', 'incomplete') or not isinstance(data['failures'], list):
                raise ValueError('invalid report')
            if data['retry'] not in ('not-run', 'not-needed', 'incomplete', 'budget-exceeded', 'invalid', 'passed', 'failed'):
                raise ValueError('invalid retry state')
            failures = data['failures']
            ids = {(item['binary'], item['name']) for item in failures}
            if len(ids) != len(failures) or any(item['retry'] not in ('not-run', 'passed', 'failed') for item in failures):
                raise ValueError('invalid failed test identities')
            if data['initial'] == 'incomplete':
                result = 'incomplete'
            elif data.get('doctests') != 'passed' or not isinstance(data.get('selected'), int) or data['selected'] < len(failures) or data['selected'] < 1:
                raise ValueError('invalid completed report')
            elif not failures and data['retry'] == 'not-needed':
                result = 'clean'
            elif failures and data['retry'] == 'passed' and all(item['retry'] == 'passed' for item in failures):
                result = 'recovered'
            elif data['retry'] in ('not-run', 'incomplete'):
                result = 'incomplete'
            elif data['retry'] in ('failed', 'budget-exceeded', 'invalid'):
                result = 'failed'
            else:
                raise ValueError('contradictory retry state')
            data['report'] = 'received'
            data['result'] = result
        except FileNotFoundError:
            data = {'type': kind, 'platform': platform, 'slice': 'all', 'report': 'missing'}
        except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError):
            data = {'type': kind, 'platform': platform, 'slice': 'all', 'report': 'invalid'}
        slices.append(data)
    return {'version': 1, 'run_id': run_id, 'attempt': attempt, 'slices': slices}


if __name__ == '__main__':
    report = consolidate(Path(sys.argv[1]), sys.argv[2], sys.argv[4])
    Path(sys.argv[3]).write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
