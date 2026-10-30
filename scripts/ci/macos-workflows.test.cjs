const assert = require('node:assert/strict');
const { readFileSync, readdirSync, mkdtempSync, mkdirSync, writeFileSync, existsSync, rmSync } = require('node:fs');
const { join } = require('node:path');
const { tmpdir } = require('node:os');
const { spawnSync } = require('node:child_process');
const { test } = require('node:test');

const root = join(__dirname, '../..');
const read = path => readFileSync(join(root, path), 'utf8');
const workflows = readdirSync(join(root, '.github/workflows')).filter(name => /\.ya?ml$/.test(name));
const jobs = file => [...read(`.github/workflows/${file}`).split('\njobs:\n')[1].matchAll(/^  ([\w-]+):\n((?:(?: {4,}.*)?\n)+)/gm)]
  .map(([, id, body]) => ({ id, body }));
const release = jobs('release.yml');
const signing = release.find(job => job.id === 'macos-sign').body;

test('every macOS job runs on the standard hosted macos-26 runner', () => {
  // The standard hosted runner is free for a public repository; the -large
  // and -xlarge sizes are billed, and no job may need a self-hosted machine.
  const audited = [];
  for (const file of workflows) {
    const source = read(`.github/workflows/${file}`).replace(/^ *#.*$/gm, '');
    assert.doesNotMatch(source, /self-hosted/, file);
    assert.doesNotMatch(source, /\bmacos-[\w.-]*-(?:large|xlarge)\b/i, file);
    assert.doesNotMatch(source, /\bmacos-latest\b/i, file);
    for (const { id, body } of jobs(file)) {
      if (!/runs-on:.*macos/i.test(body)) continue;
      assert.match(body, /^    runs-on: macos-26$/m, id);
      // Jobs that compile keep rust-cache's key to the pinned toolchain.
      if (/rust-cache/.test(body)) {
        assert.ok(body.indexOf('prune-rust-toolchains.sh') < body.indexOf('Swatinem/rust-cache'), `${file}/${id}`);
        assert.ok(body.includes('prune-rust-toolchains.sh'), `${file}/${id}`);
      }
      audited.push(`${file}/${id}`);
    }
  }
  assert.deepEqual(audited, ['ci.yml/macos', 'ci.yml/e2e-macos', 'preview-build.yml/preview-macos',
    'release.yml/macos-prepare', 'release.yml/macos-sign']);
});

test('macOS preview builds on the build-preview label or dispatch, forks included', () => {
  const body = jobs('preview-build.yml').find(job => job.id === 'preview-macos').body;
  const expression = body.match(/    if: \|\n((?:      .*\n)+)/)[1]
    .replace(/github\.event\.pull_request\.labels\.\*\.name/g, 'github.event.pull_request.labels.map(label => label.name)');
  const evaluate = new Function('github', 'contains', `return (${expression});`);
  const contains = (array, item) => array.includes(item);
  for (const head of ['someone/fork', 'pwrdrvr/PwrRoute']) {
    const pr = { head: { repo: { full_name: head } }, labels: [{ name: 'build-preview' }] };
    const github = { event_name: 'pull_request', repository: 'pwrdrvr/PwrRoute', event: {
      action: 'labeled', label: { name: 'build-preview' }, pull_request: pr } };
    assert.equal(evaluate(github, contains), true, `labeled ${head}`);
    github.event.action = 'synchronize';
    assert.equal(evaluate(github, contains), true, `synchronize ${head}`);
    github.event = { action: 'labeled', label: { name: 'other' }, pull_request: { ...pr, labels: [{ name: 'other' }] } };
    assert.equal(evaluate(github, contains), false, `other label ${head}`);
  }
  assert.equal(evaluate({ event_name: 'workflow_dispatch', event: {} }, contains), true);
});

test('credentialed signing retains environment and digest boundaries without checkout or compilation', () => {
  assert.match(signing, /environment: apple-signing/);
  assert.doesNotMatch(signing, /uses: actions\/checkout|run: cargo /);
  assert.match(signing, /needs\.macos-prepare\.outputs\.signing-input-sha256/);
  assert.match(signing, /shasum -a 256 --check/);
  assert.ok(signing.indexOf('name: Verify signing input') < signing.indexOf('name: Expand signing input'));
  assert.ok(signing.indexOf('name: Expand signing input') < signing.indexOf('name: Import Developer ID'));
  assert.match(signing, /rm -rf -- release-signing/);
  assert.match(signing, /mktemp -d "\$RUNNER_TEMP\/pwrroute-signing\.XXXXXX"/);
  assert.match(signing, /path: release-signing\/dist\/\*/);
  const prepare = release.find(job => job.id === 'macos-prepare').body;
  assert.doesNotMatch(prepare, /secrets\.|environment:/);
  for (const script of ['dmg-layout.py', 'macos-signing.py', 'macos-dmg.sh']) assert.ok(prepare.includes(script));
  for (const job of [prepare, jobs('preview-build.yml')[0].body]) {
    assert.match(job, /targets: aarch64-apple-darwin, x86_64-apple-darwin/);
    assert.match(job, /bundle --universal .*--require-icon-catalog/);
    assert.match(job, /DEVELOPER_DIR: \$\{\{ steps.xcode.outputs.developer-dir \}\}/);
  }
  assert.match(signing, /input\/macos-dmg.sh/);
  assert.match(signing, /xcrun stapler validate/);
});

test('DMG packaging writes complete layout without Finder Automation', () => {
  const script = read('scripts/release/macos-dmg.sh');
  assert.doesNotMatch(script, /\bosascript\b/);
  assert.match(script, /python3 "\$here\/dmg-layout.py" "\$mount" "\$volname"/);
  assert.match(script, /-nobrowse -mountpoint "\$work\/mount"/);
  assert.match(script, /ln -s \/Applications/);
  assert.match(script, /\.VolumeIcon.icns/);
});

test('always cleanup removes only the private credential directory even when keychain restoration fails', {
  skip: process.platform === 'win32',
}, () => {
  const cleanup = signing.split('      - name: Remove temporary signing credentials\n')[1]
    .split('\n      # This version')[0];
  assert.match(cleanup, /if: always\(\)/);
  const script = cleanup.split('        run: |\n')[1].split('\n').map(line => line.slice(10)).join('\n');
  for (const status of [0, 17]) {
    const directory = mkdtempSync(join(tmpdir(), 'pwrroute-cleanup-test-'));
    try {
      const privateDir = join(directory, 'private');
      mkdirSync(privateDir);
      mkdirSync(join(directory, 'input'));
      writeFileSync(join(directory, 'input/macos-signing.py'), `raise SystemExit(${status})\n`);
      for (const file of ['AuthKey.p8', 'cert.p12', 'signing.keychain-db']) writeFileSync(join(privateDir, file), 'fixture');
      const preserved = join(directory, 'unrelated.keychain-db');
      writeFileSync(preserved, 'fixture');
      const result = spawnSync('bash', ['-c', script], {
        cwd: directory, env: { ...process.env, PWRROUTE_SIGNING_TEMP_DIR: privateDir }, encoding: 'utf8',
      });
      assert.equal(result.status, status, result.stderr);
      assert.equal(existsSync(privateDir), false);
      assert.equal(existsSync(preserved), true);
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  }
});
