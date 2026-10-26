const assert = require('node:assert/strict');
const { mkdtempSync, mkdirSync, copyFileSync, writeFileSync, readFileSync, rmSync } = require('node:fs');
const { tmpdir } = require('node:os');
const { join } = require('node:path');
const { spawnSync } = require('node:child_process');
const { test } = require('node:test');

function start(t, { buildFails = false, quitFails = false } = {}) {
  const root = mkdtempSync(join(tmpdir(), 'pwrroute start '));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const bin = join(root, 'bin');
  mkdirSync(bin);
  mkdirSync(join(root, 'scripts/dev'), { recursive: true });
  copyFileSync(join(__dirname, 'start.sh'), join(root, 'scripts/dev/start.sh'));
  const mock = (name, body) => writeFileSync(join(bin, name), `#!/bin/bash\n${body}\n`, { mode: 0o755 });
  mock('uname', 'echo Darwin');
  mock('cargo', `
    echo "cargo $*" >> "$START_LOG"
    if [[ "$1" == build ]]; then exit ${buildFails ? 42 : 0}; fi
    mkdir -p target/dev/PwrRoute.app/Contents/MacOS
    cp "$MOCK_APP" target/dev/PwrRoute.app/Contents/MacOS/pwrroute
  `);
  mock('swift', `echo "quit" >> "$START_LOG"; exit ${quitFails ? 43 : 0}`);
  mock('app', 'echo "app $*" >> "$START_LOG"; exit 17');
  const result = spawnSync('bash', [join(root, 'scripts/dev/start.sh')], {
    cwd: tmpdir(), // The action must find its checkout from the script path.
    env: { ...process.env, PATH: `${bin}:${process.env.PATH}`, START_LOG: join(root, 'run.log'), MOCK_APP: join(bin, 'app') },
    encoding: 'utf8',
  });
  return { result, log: readFileSync(join(root, 'run.log'), 'utf8').trim().split('\n') };
}

test('Start launches the worktree bundle with explicit ownership and preserves its exit status', t => {
  const { result, log } = start(t);
  assert.equal(result.status, 17, result.stderr);
  assert.deepEqual(log, [
    'cargo build --locked -p pwrroute-app',
    'quit',
    'cargo xtask bundle --debug --app-only',
    'app --make-default',
  ]);
});

test('a build failure leaves the running router alone', t => {
  const { result, log } = start(t, { buildFails: true });
  assert.equal(result.status, 42, result.stderr);
  assert.deepEqual(log, ['cargo build --locked -p pwrroute-app']);
});

test('a refused quit never replaces or launches the bundle', t => {
  const { result, log } = start(t, { quitFails: true });
  assert.equal(result.status, 43, result.stderr);
  assert.deepEqual(log, ['cargo build --locked -p pwrroute-app', 'quit']);
});
