const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { join } = require('node:path');
const { test } = require('node:test');

// Execute the actual github-script body, so fixtures cannot drift from CI.
const workflow = readFileSync(join(__dirname, '../../.github/workflows/ci.yml'), 'utf8');
const script = workflow.match(/          script: \|\n((?:            .*\n|\n)+)/)?.[1];
assert.ok(script, 'the classifier must remain covered by these tests');
const classify = new (async function () {}).constructor('github', 'context', 'core', script);

async function run({ paths, files = paths?.map(filename => ({ filename, status: 'modified' })),
  count = files?.length, event = 'pull_request', apiError = false, summaryError = false }) {
  const outputs = {};
  const calls = [];
  const summary = {
    addHeading() { return this; },
    addTable() { return this; },
    addRaw() { return this; },
    async write() { if (summaryError) throw new Error('summary unavailable'); },
  };
  await classify({
    rest: { pulls: { listFiles: 'pulls.listFiles' } },
    async paginate(method, args) {
      calls.push({ method, args });
      if (apiError) throw new Error('API unavailable');
      return files;
    },
  }, {
    eventName: event,
    repo: { owner: 'pwrdrvr', repo: 'PwrRoute' },
    issue: { number: 93 },
    payload: { pull_request: { changed_files: count } },
  }, {
    setOutput(name, value) { outputs[name] = value; },
    notice() {},
    summary,
  });
  return { outputs, calls };
}

const releaseSkill = [
  '.agents/skills/release/SKILL.md',
  '.agents/skills/release/agents/openai.yaml',
  '.agents/skills/release/references/publishing.md',
];

for (const [name, paths] of [
  ['PR #93: all three release-skill paths', releaseSkill],
  ...releaseSkill.map(path => [`PR #93: ${path}`, [path]]),
  ['skill scripts, references and images', [
    '.agents/skills/example/scripts/check.sh',
    '.agents/skills/example/references/schema.json',
    '.agents/skills/example/assets/icon.png',
  ]],
  ['documentation and diagrams', ['README.md', 'crates/pwrroute-app/README.MD', 'docs/routing.svg']],
  ['Dependabot configuration', ['.github/dependabot.yml']],
  ['issue and PR templates', ['.github/ISSUE_TEMPLATE/bug_report.yml', '.github/PULL_REQUEST_TEMPLATE/custom.yml']],
  ['mixed non-code categories', [...releaseSkill, 'docs/example.json', '.github/dependabot.yml']],
]) {
  test(`${name} skips expensive CI`, async () => {
    assert.equal((await run({ paths })).outputs.code_impacting, 'false');
  });
}

for (const path of [
  'crates/pwrroute-app/src/main.rs', 'crates/pwrroute-app/assets/icons/mark.svg',
  'Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', '.cargo/config.toml',
  'scripts/release/install-vpk.sh', 'xtask/src/main.rs', 'e2e/CONTRACT.md.json',
  '.github/workflows/ci.yml', '.github/actions/test/action.yml', '.github/actionlint.yaml',
  'scripts/ci/classify-changes.test.cjs', 'packaging/macos/Info.plist.in',
  'licenses/bundled.txt', 'LICENSE', 'THIRD_PARTY_LICENSES', 'CHANGELOG.md',
  '.gitattributes', '.gitignore', '.agents/config.toml', '.agents/skills-other/script.sh',
  'unknown.yaml',
]) {
  test(`${path} alone or mixed with skills runs full CI`, async () => {
    for (const paths of [[path], [...releaseSkill, path]]) {
      assert.equal((await run({ paths })).outputs.code_impacting, 'true');
    }
  });
}

for (const [previous_filename, filename, full] of [
  ['.agents/skills/old/openai.yaml', '.agents/skills/new/agents/openai.yaml', false],
  ['docs/old.svg', 'docs/new.svg', false],
  ['scripts/release/check.sh', '.agents/skills/release/scripts/check.sh', true],
  ['.agents/skills/release/scripts/check.sh', 'scripts/release/check.sh', true],
  ['Cargo.toml', 'docs/old-manifest.md', true],
  ['README.md', 'Cargo.toml', true],
]) {
  test(`rename ${previous_filename} to ${filename}`, async () => {
    const result = await run({ files: [{ status: 'renamed', filename, previous_filename }] });
    assert.equal(result.outputs.code_impacting, String(full));
  });
}

for (const status of ['added', 'modified', 'removed']) {
  test(`${status} skill asset skips CI`, async () => {
    const files = [{ filename: releaseSkill[1], status }];
    assert.equal((await run({ files })).outputs.code_impacting, 'false');
  });
}

for (const [name, input] of [
  ['empty list', { files: [] }],
  ['incomplete pagination', { paths: releaseSkill, count: 101 }],
  ['API 3,000-file cap', { paths: Array.from({ length: 3000 }, (_, i) => `docs/${i}.md`), count: 3001 }],
  ['missing count', { paths: releaseSkill, count: null }],
  ['noninteger count', { paths: releaseSkill, count: 3.5 }],
  ['wrong count type', { paths: releaseSkill, count: '3' }],
  ['duplicate pages', { paths: [releaseSkill[0], releaseSkill[0]] }],
  ['malformed file', { files: [{}] }],
  ['null file', { files: [null] }],
  ['empty filename', { files: [{ filename: '' }] }],
  ['invalid filename', { files: [{ filename: 42 }] }],
  ['rename without old path', { files: [{ filename: releaseSkill[1], status: 'renamed' }] }],
  ['invalid old path', { files: [{ filename: releaseSkill[1], previous_filename: 42 }] }],
  ['empty old path', { files: [{ filename: releaseSkill[1], previous_filename: '' }] }],
  ['API failure', { paths: releaseSkill, apiError: true }],
]) {
  test(`${name} runs full CI`, async () => {
    assert.equal((await run(input)).outputs.code_impacting, 'true');
  });
}

test('uses the paginating API for more than one page', async () => {
  const paths = Array.from({ length: 205 }, (_, i) => `docs/${i}.md`);
  const result = await run({ paths });
  assert.equal(result.outputs.code_impacting, 'false');
  assert.deepEqual(result.calls, [{ method: 'pulls.listFiles', args: {
    owner: 'pwrdrvr', repo: 'PwrRoute', pull_number: 93, per_page: 100,
  } }]);
});

test('pushes always run full CI without querying PR files', async () => {
  const result = await run({ paths: releaseSkill, event: 'push' });
  assert.equal(result.outputs.code_impacting, 'true');
  assert.equal(result.calls.length, 0);
});

test('classifier job output fails open to full CI after script failures or missing output', async () => {
  assert.match(workflow, /continue-on-error: true/);
  const expression = workflow.match(/code_impacting: \$\{\{ (.+) \}\}/)[1];
  const output = (outcome, value) => new Function('steps', `return (${expression});`)({
    classify: { outcome, outputs: { code_impacting: value } },
  });
  assert.equal(output('success', 'false'), false);
  for (const [outcome, value] of [
    ['success', 'true'], ['success', undefined], ['failure', 'false'],
    ['failure', undefined], ['skipped', undefined], ['cancelled', 'false'],
  ]) assert.equal(output(outcome, value), true);
  await assert.rejects(run({ paths: releaseSkill, summaryError: true }), /summary unavailable/);
});

test('expensive jobs run on classifier failure and skip only on explicit non-code success', () => {
  const selfHosted = [];
  for (const id of ['typecheck', 'build', 'test', 'macos', 'windows', 'e2e-linux', 'e2e-macos', 'e2e-windows']) {
    const job = workflow.match(new RegExp(`^  ${id}:\\n([\\s\\S]*?)(?=^  [a-z].*:|$(?![\\s\\S]))`, 'm'))?.[1];
    assert.ok(job, id);
    const expression = job.match(/    if: >-\n([\s\S]*?)(?=^    \w)/m)?.[1];
    assert.ok(expression, `${id} must have an explicit fail-safe job condition`);
    const evaluate = new Function('needs', 'github', 'cancelled', `return (${expression});`);
    for (const [result, output, expected] of [
      ['success', 'false', false], ['success', 'true', true],
      ['success', '', true], ['failure', 'false', true], ['failure', '', true],
      ['skipped', '', true],
    ]) {
      const needs = { changes: { result, outputs: { code_impacting: output } } };
      const github = { event_name: 'push' };
      assert.equal(evaluate(needs, github, () => false), expected, `${id}: ${result}/${output}`);
      assert.equal(evaluate(needs, github, () => true), false, `${id}: cancelled`);
    }
    // Every job on the self-hosted pool keeps the fork guard: the guests
    // persist and must never run fork code.
    if (/^    runs-on: \[self-hosted\b/m.test(job)) {
      const needs = { changes: { result: 'success', outputs: { code_impacting: 'true' } } };
      const github = { event_name: 'pull_request', repository: 'pwrdrvr/PwrRoute',
        event: { pull_request: { head: { repo: { full_name: 'someone/fork' } } } } };
      assert.equal(evaluate(needs, github, () => false), false, `${id}: fork guard is retained`);
      github.event.pull_request.head.repo.full_name = github.repository;
      assert.equal(evaluate(needs, github, () => false), true, `${id}: same-repository PR runs`);
      selfHosted.push(id);
    }
  }
  assert.deepEqual(selfHosted, ['macos', 'e2e-macos'], 'the self-hosted jobs were checked');
});

test('required check names stay present and the workflow trigger has no path filters', () => {
  for (const [id, name] of [['typecheck', 'Typecheck'], ['build', 'Build'], ['test', 'Test']]) {
    assert.match(workflow, new RegExp(`  ${id}:\\n    name: ${name}\\n`));
  }
  assert.doesNotMatch(workflow, /^\s+paths(?:-ignore)?:/m);
  const aggregate = workflow.split('  desktop-e2e-result:')[1];
  assert.match(aggregate, /always\(\)/);
  assert.match(aggregate, /needs\.changes\.result != 'success'/);
  assert.match(aggregate, /needs\.changes\.outputs\.code_impacting != 'false'/);
});
