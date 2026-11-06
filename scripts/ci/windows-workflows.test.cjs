const assert = require('node:assert/strict');
const { readFileSync, readdirSync } = require('node:fs');
const { join } = require('node:path');
const { test } = require('node:test');

test('Windows build, test and packaging steps use runner temp', () => {
  const expected = {
    'ci.yml/windows': ['Test', 'Retry failed tests'],
    'ci.yml/e2e-windows': ['Desktop E2E', 'Retry failed Desktop E2E tests'],
    'preview-build.yml/preview-windows': ['Build (unsigned)', 'Package zip', 'Install vpk', 'Pack unsigned Setup.exe and MSI'],
    'release.yml/windows-prepare': ['Build', 'Stage signing input'],
    'release.yml/windows-sign': ['Install vpk', 'Fetch the previous release for deltas', 'Pack with vpk'],
  };
  const directory = join(__dirname, '../../.github/workflows');
  const audited = [];
  for (const file of readdirSync(directory).filter(name => /\.ya?ml$/.test(name)).sort()) {
    const source = readFileSync(join(directory, file), 'utf8');
    const jobs = source.split('\njobs:\n')[1];
    assert.ok(jobs, `${file}: jobs section`);
    for (const [, id, body] of jobs.matchAll(/^  ([\w-]+):\n((?:(?: {4,}.*)?\n)+)/gm)) {
      if (!/^    runs-on: windows-/m.test(body)) continue;
      const name = `${file}/${id}`;
      assert.ok(expected[name], `${name}: audit new Windows job`);
      for (const stepName of expected[name]) {
        const step = body.split(`      - name: ${stepName}\n`)[1]?.split(/\n      - /)[0];
        assert.ok(step, `${name}: ${stepName}`);
        for (const variable of ['TEMP', 'TMP']) {
          assert.match(step, new RegExp(`^          ${variable}: \\$\\{\\{ runner\\.temp \\}\\}$`, 'm'),
            `${name}: ${stepName} ${variable}`);
        }
      }
      audited.push(name);
    }
  }
  assert.deepEqual(audited.sort(), Object.keys(expected).sort());
});
