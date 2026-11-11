const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { join } = require('node:path');
const { test } = require('node:test');

// Execute the actual github-script body, so fixtures cannot drift from CI.
const workflow = readFileSync(join(__dirname, '../../.github/workflows/ci.yml'), 'utf8');
const script = workflow.match(/          script: \|\n((?:            .*\n|\n)+)/)?.[1];
assert.ok(script, 'the classifier must remain covered by these tests');
const classify = new (async function () {}).constructor('github', 'context', 'core', script);

const sha = (c) => c.repeat(40);

async function run({ paths, files = paths?.map(filename => ({ filename, status: 'modified' })),
  count = files?.length, event = 'pull_request', apiError = false, summaryError = false,
  labels = [], labelError = false, before = sha('a'), after = sha('b'),
  compare = { status: 'ahead', files }, compareError = false }) {
  const outputs = {};
  const calls = [];
  const summary = {
    addHeading() { return this; },
    addTable() { return this; },
    addRaw() { return this; },
    async write() { if (summaryError) throw new Error('summary unavailable'); },
  };
  await classify({
    rest: {
      pulls: {
        listFiles: 'pulls.listFiles',
        async get(args) {
          calls.push({ method: 'pulls.get', args });
          if (labelError) throw new Error('labels unavailable');
          return { data: { labels: labels.map(name => ({ name })) } };
        },
      },
      repos: {
        async compareCommitsWithBasehead(args) {
          calls.push({ method: 'repos.compareCommitsWithBasehead', args });
          if (compareError) throw new Error('compare unavailable');
          return { data: compare };
        },
      },
    },
    async paginate(method, args) {
      calls.push({ method, args });
      if (apiError) throw new Error('API unavailable');
      return files;
    },
  }, {
    eventName: event,
    repo: { owner: 'pwrdrvr', repo: 'PwrRoute' },
    issue: { number: 93 },
    payload: event === 'push' ? { before, after } : { pull_request: { changed_files: count } },
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
  ['design mirror', [
    'design/PwrRoute Send Links.dc.html', 'design/pwrroute.css', 'design/support.js', 'design/SOURCE.md',
  ]],
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
  'unknown.yaml', 'crates/pwrroute-app/design/canvas.html', 'designs/canvas.html',
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
  ['design/Old.dc.html', 'design/New.dc.html', false],
  ['design/canvas.html', 'crates/pwrroute-app/assets/canvas.html', true],
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
    const { outputs } = await run(input);
    assert.equal(outputs.code_impacting, 'true');
    assert.equal(outputs.packaging_impacting, 'true');
  });
}

test('uses the paginating API for more than one page', async () => {
  const paths = Array.from({ length: 205 }, (_, i) => `docs/${i}.md`);
  const result = await run({ paths });
  assert.equal(result.outputs.code_impacting, 'false');
  assert.deepEqual(result.calls.filter(c => c.method !== 'pulls.get'), [{ method: 'pulls.listFiles', args: {
    owner: 'pwrdrvr', repo: 'PwrRoute', pull_number: 93, per_page: 100,
  } }]);
});

test('pushes always run every job, without querying PR files', async () => {
  for (const paths of [releaseSkill, ['crates/pwrroute-engine/src/lib.rs']]) {
    const result = await run({ paths, event: 'push' });
    assert.equal(result.outputs.code_impacting, 'true');
    assert.ok(result.calls.every(c => !c.method.startsWith('pulls.')));
  }
});

// What the macOS bundle is built from, plus the dependencies, toolchain and
// CI setup.
const packagingPaths = [
  'packaging/macos/Info.plist.in', 'packaging/icon/PwrRoute.icon/icon.json',
  'packaging/dmg/background.png', 'packaging/linux/pwrroute.desktop',
  'scripts/release/macos-dmg.sh', 'xtask/src/macos.rs', 'xtask/Cargo.toml',
  'Cargo.lock', 'Cargo.toml', 'crates/pwrroute-app/Cargo.toml',
  'rust-toolchain.toml', '.cargo/config.toml', '.github/workflows/ci.yml',
  '.github/actions/select-xcode-for-actool/action.yml',
  // xtask/src/macos.rs copies these into the bundle's Resources.
  'LICENSE', 'THIRD_PARTY_LICENSES', 'CHANGELOG.md',
];
const ordinaryCode = [
  'crates/pwrroute-app/src/main.rs', 'crates/pwrroute-app/build.rs',
  'crates/pwrroute-engine/assets/finicky.js', 'e2e/CONTRACT.md.json',
  'scripts/ci/classify-changes.test.cjs', '.github/workflows/release.yml',
  'licenses/bundled.txt', 'LICENSE.md', 'docs/packaging.md',
  'packaging-notes.txt', 'other/xtask/main.rs', 'crates/Cargo.toml.bak',
];

test('packaging inputs select the bundle check on PRs, including renames', async () => {
  for (const path of packagingPaths) {
    for (const paths of [[path], [...ordinaryCode, path], [...releaseSkill, path]]) {
      const { outputs } = await run({ paths });
      assert.equal(outputs.packaging_impacting, 'true', path);
      assert.equal(outputs.code_impacting, 'true', path);
    }
    for (const [previous_filename, filename] of [[path, 'src/moved.rs'], ['src/moved.rs', path]]) {
      assert.equal((await run({ files: [{ filename, previous_filename, status: 'renamed' }] }))
        .outputs.packaging_impacting, 'true', `rename: ${previous_filename} to ${filename}`);
    }
  }
});

test('ordinary code and docs skip the bundle check on PRs', async () => {
  for (const paths of [ordinaryCode, releaseSkill, ...ordinaryCode.map(path => [path])]) {
    assert.equal((await run({ paths })).outputs.packaging_impacting, 'false', paths.join());
  }
  assert.equal((await run({ paths: ordinaryCode })).outputs.code_impacting, 'true');
});

test('the full-ci label runs everything, even for docs, read live', async () => {
  const result = await run({ paths: releaseSkill, labels: ['documentation', 'full-ci'] });
  assert.equal(result.outputs.code_impacting, 'true');
  assert.equal(result.outputs.packaging_impacting, 'true');
  assert.deepEqual(result.calls[0], { method: 'pulls.get', args: {
    owner: 'pwrdrvr', repo: 'PwrRoute', pull_number: 93 } });
  const other = await run({ paths: ordinaryCode, labels: ['build-preview', 'full-ci-later'] });
  assert.equal(other.outputs.packaging_impacting, 'false');
});

test('a failed label lookup runs full CI', async () => {
  const { outputs } = await run({ paths: releaseSkill, labelError: true });
  assert.equal(outputs.code_impacting, 'true');
  assert.equal(outputs.packaging_impacting, 'true');
});

test('main pushes bundle only when the pushed range has a packaging input', async () => {
  for (const path of packagingPaths) {
    const result = await run({ event: 'push', paths: [...ordinaryCode, path] });
    assert.equal(result.outputs.packaging_impacting, 'true', path);
  }
  const renamed = await run({ event: 'push', files: [
    { filename: 'src/moved.rs', previous_filename: 'xtask/src/old.rs', status: 'renamed' }] });
  assert.equal(renamed.outputs.packaging_impacting, 'true');
  const result = await run({ event: 'push', paths: ordinaryCode });
  assert.equal(result.outputs.packaging_impacting, 'false');
  assert.equal(result.outputs.code_impacting, 'true');
  assert.deepEqual(result.calls, [{ method: 'repos.compareCommitsWithBasehead', args: {
    owner: 'pwrdrvr', repo: 'PwrRoute', basehead: `${sha('a')}...${sha('b')}` } }]);
});

for (const [name, input] of [
  ['new branch (zero before)', { before: sha('0') }],
  ['missing before', { before: null }],
  ['malformed before', { before: 'HEAD~1' }],
  ['malformed after', { after: 'main' }],
  ['compare failure', { compareError: true }],
  ['force push', { compare: { status: 'diverged', files: [] } }],
  ['behind range', { compare: { status: 'behind', files: [{ filename: 'src/a.rs' }] } }],
  ['identical range', { compare: { status: 'identical', files: [] } }],
  ['empty file list', { compare: { status: 'ahead', files: [] } }],
  ['300-file compare cap', { compare: { status: 'ahead',
    files: Array.from({ length: 300 }, (_, i) => ({ filename: `src/${i}.rs` })) } }],
  ['malformed file', { compare: { status: 'ahead', files: [{}] } }],
  ['duplicate files', { compare: { status: 'ahead',
    files: [{ filename: 'src/a.rs' }, { filename: 'src/a.rs' }] } }],
  ['rename without old path', { compare: { status: 'ahead',
    files: [{ filename: 'src/a.rs', status: 'renamed' }] } }],
  ['missing files', { compare: { status: 'ahead' } }],
  ['missing compare body', { compare: null }],
]) {
  test(`main push with ${name} runs the bundle check`, async () => {
    const { outputs } = await run({ event: 'push', paths: ordinaryCode, ...input });
    assert.equal(outputs.packaging_impacting, 'true');
    assert.equal(outputs.code_impacting, 'true');
  });
}

test('other events run full CI', async () => {
  const { outputs, calls } = await run({ event: 'workflow_dispatch', paths: releaseSkill });
  assert.equal(outputs.code_impacting, 'true');
  assert.equal(outputs.packaging_impacting, 'true');
  assert.equal(calls.length, 0);
});

for (const field of ['code_impacting', 'packaging_impacting']) {
  test(`${field} job output fails open after script failures or missing output`, async () => {
    assert.match(workflow, /continue-on-error: true/);
    const expression = workflow.match(new RegExp(`      ${field}: \\$\\{\\{ (.+) \\}\\}`))[1];
    const output = (outcome, value) => new Function('steps', `return (${expression});`)({
      classify: { outcome, outputs: { [field]: value } },
    });
    assert.equal(output('success', 'false'), false);
    for (const [outcome, value] of [
      ['success', 'true'], ['success', undefined], ['failure', 'false'],
      ['failure', undefined], ['skipped', undefined], ['cancelled', 'false'],
    ]) assert.equal(output(outcome, value), true);
    await assert.rejects(run({ paths: releaseSkill, summaryError: true }), /summary unavailable/);
  });
}

test('expensive jobs run on classifier failure and skip only on explicit non-code success', () => {
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
    // Every job runs on a throwaway hosted runner, so fork PRs get the same
    // coverage as branches in this repository.
    assert.doesNotMatch(job, /^    runs-on: \[?self-hosted\b/m, id);
    const needs = { changes: { result: 'success', outputs: { code_impacting: 'true' } } };
    const github = { event_name: 'pull_request', repository: 'pwrdrvr/PwrRoute',
      event: { pull_request: { head: { repo: { full_name: 'someone/fork' } } } } };
    assert.equal(evaluate(needs, github, () => false), true, `${id}: fork PR runs`);
  }
});

test('the macOS job bundles only when a packaging input changed', () => {
  const job = workflow.split('\n  macos:\n')[1].split(/\n  [a-z][\w-]*:\n/)[0];
  const steps = job.split(/^      - /m);
  const gated = ['name: Select an Xcode with actool 26', 'name: Bundle'].map(name => {
    const step = steps.find(s => s.startsWith(name));
    assert.ok(step, name);
    const expression = step.match(/        if: >-\n([\s\S]*?)\n        (?:uses|env|run):/)?.[1];
    assert.ok(expression, `${name} must be gated`);
    return new Function('needs', `return (${expression});`);
  });
  assert.match(steps.find(s => s.startsWith('name: Bundle')), /run: cargo xtask bundle/);
  for (const evaluate of gated) {
    for (const [result, output, expected] of [
      ['success', 'false', false], ['success', 'true', true], ['success', '', true],
      ['failure', 'false', true], ['failure', '', true], ['skipped', '', true],
    ]) {
      assert.equal(evaluate({ changes: { result, outputs: { packaging_impacting: output } } }),
        expected, `${result}/${output}`);
    }
  }
  // Tests stay ungated: only the bundle waits for packaging inputs.
  const testStep = steps.find(s => s.startsWith('name: Test\n'));
  assert.doesNotMatch(testStep, /packaging_impacting/);
  // No job is gated on packaging: every job runs on each code change, and
  // only these two steps of the macOS job wait for a packaging input.
  for (const match of workflow.matchAll(/^    if: >-\n((?:      .*\n)+)/gm)) {
    assert.doesNotMatch(match[1], /packaging_impacting/);
  }
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
