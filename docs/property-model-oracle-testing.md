# Property, model and oracle tests

Most PwrRoute tests are **example tests**: pick one input, write down the
output you expect. They check only the cases someone thought of, and they
check what the author *believed*. In this repo's history the expected output
was sometimes the bug itself, and the test kept it in place (table below).

The three ideas here let the computer choose the inputs. We use
[proptest](https://docs.rs/proptest) in ordinary `cargo test` tests.

## The three words

**Property.** A rule that must hold for *every* input, written as code.
proptest generates hundreds of inputs. When one breaks the rule, it *shrinks*
that input to the smallest one that still fails, and prints it.

> *"A rule suggested for a link with a query matches the same page with
> another query."* Against the code before #117, this shrank to
> `https://site.com/a?a=` within a median of 7 inputs. Three assertions in
> `suggest.rs` had pinned the exact-link suggestion as correct.

**Model.** For code with state or concurrency, a small stand-in that is
obviously correct. The test generates a random list of commands (or a random
order for concurrent work), runs it on the real code and on the model, and
checks that they agree after every step.

> Two real `Syncer`s share one temp folder as the cloud. Commands: edit,
> sync, the other machine edits or pushes in the middle of a sync, the cloud
> file is half written or deleted. Rule: sync replaces a revision only with
> one that descends from it. Before #174 it failed on 100 of 100 seeds and
> shrank to `edit A; sync A while B pushes`.

**Oracle.** Whatever tells the test what the right answer is. It can be one
of three things:

- **A real outside source of truth.** Finicky itself (its config API's
  JavaScript, or the Go host's answers that `tests/oracle/record.py`
  recorded), the OS, or a real directory on disk.
- **A slow, obviously correct version** of the function.
- **A second code path** that must give the same answer. Comparing two paths
  is called a *differential* test.

> *"Each browser's own reader decodes exactly the arguments PwrRoute
> meant."* The readers come from Chromium's source and from Edge's captured
> sender. Against #92 this shrank to Edge with arguments `[program, ""]`:
> Edge read the program as a number and opened the wrong profile.

The three work together. A property needs an oracle to say what "correct"
means, and a model *is* an oracle for stateful code.

## What the history showed

Each test below ran against the code from just before the fix, extracted with
`git show <fix>^:<path>`, and passed on the fixed code. "Found in" is the
median number of generated inputs before the first failure, over 100 seeds.

| Fix | Bug | Test | Found in |
| --- | --- | --- | --- |
| #59, #168 | Approving PwrRoute as default browser went unnoticed when the OS answered late (#59) or one read failed (#168). A test asserted the refusal. Shipped in 20 releases. | Model: generated OS lag and failed reads | 1–2 inputs, 100/100 seeds |
| #166 | The updater dropped a slot change or a Check now made during a check. Two tests asserted the stale answer. Shipped in 16 releases. | Model with a generated thread schedule | 2 inputs, 100/100 seeds |
| #167 | Release discovery had no total deadline. Shipped in 16 releases. | Property, plus a differential test against the old serial code | 2 inputs, 100/100 seeds |
| #117 | Add a rule suggested the exact link with its query, which never matched again. Three assertions pinned it. Shipped in 10 releases. | Property, Finicky's `isMatch` as oracle | 7 inputs, 100/100 seeds |
| #116 | The tray dropdown opened at the top-left corner on macOS 27. Shipped in 10 releases. | Property over generated display layouts | 1 input, 100/100 seeds |
| #169, #174 | Sync lost an edit, stopped converging, or overwrote another machine's revision. With #43, four fixes to one rule. Shipped in 6 releases. | Model: two real `Syncer`s on a temp folder | 10–46 inputs |
| #92 → #99 → #119 | Each hand-off fix caused the next bug: Edge misread Chrome's format, then got a 2 s second copy. | Oracle: each browser's own reader | 4 inputs, 100/100 seeds |
| #46 | macOS `open` dropped a config's `args` for a running browser. Four assertions pinned the missing `-n`. | Property | 2 inputs, 100/100 seeds |
| #72 | A Finicky config that never loaded took `rules.json` down with it. | Model of config and `rules.json` loads | 4 inputs, 100/100 seeds |
| #49 | An expired approval window locked an agent out for good. | Model of the approval door | 4 inputs, 100/100 seeds |

Run against main at v0.17.0, the same rules still fail for three bugs: a
suggested rule for a path containing `^` never matches its own link, the
tray dropdown opens on the wrong display when screens have different scales,
and a deleted cloud `rules.json` lets a stale machine revert newer edits.

## When to write one

- **Fixing a bug.** Write the rule the bug broke as a property or model
  invariant, and confirm it fails on the pre-fix code before you fix it. Also
  keep the shrunk counterexample as a plain example test, so the regression
  has a name.
- **Pure logic with rules:** URL matching, wildcard and pattern
  suggestions, rewrites, release-slot choice, launch argument building, wire
  formats, text and offset arithmetic, window placement. Write a property.
- **State and concurrency:** sync between machines, `rules.json` drafts and
  reloads, approvals, first run, the updater, router queues and workers.
  Write a model.
- **Anything Finicky also answers.** Finicky is the oracle. Compare with its
  config API in the engine's QuickJS (`Engine::collisions` asks Finicky's
  `isMatch`), or with the Go host's recorded answers. For example,
  `every_candidate_matches_its_own_link_by_finickys_is_match` in
  `crates/pwrroute-engine/tests/rules_tests.rs` checks 23 hand-picked links;
  generate the links instead.
- **Two places answer the same question,** such as the router and Test a
  link, or the Rust `url` crate that builds rule suggestions and the URL
  parser inside Finicky's config API that matches them. Write a
  differential test that they agree.
- **Before an `assert_eq!` pins a refusal, an empty result or an order,** say
  the rule in words. If the rule and the output disagree, the output is the
  bug.

## How to write them

- **proptest is a dev-dependency** of the crate that uses it:
  `proptest = "1"` under `[dev-dependencies]`. Cargo.lock already has 1.12.0
  through GPUI, so it adds no crate to the lock. Dev-dependencies do not
  ship, so `THIRD_PARTY_LICENSES` does not change, but `cargo deny` still
  judges them, and Dependabot's seven-day cooldown applies to upgrades.
  Adding it is a dependency change: run `cargo xtask lint` (in a sandbox when
  the thread has Sandboxer; see the root `AGENTS.md`).
- **Put the test where the code's tests already live.** A property over a
  private function goes in that module's `#[cfg(test)]` tests; one over a
  crate's public API goes in `crates/<crate>/tests/<area>_props.rs`.
  `cargo xtask test` runs both, and CI runs them on Linux, macOS and Windows.
- **Build inputs the way users do.** Generate a list of operations (edit a
  rule, save, write `rules.json` from outside, sync, click a link) and replay
  it, not random structs. Don't generate states the app can't reach.
  Impossible states give false failures.
- **Take the property from a promise:** a doc comment, the design in
  `design/`, an `AGENTS.md`, Finicky's behaviour, or what the user wants.
  Never take it from today's output.
- **Narrow, don't weaken.** A counterexample may be a case the code handles
  on purpose. If so, restrict the strategy or add `prop_assume!(...)`, with a
  comment saying why. Don't loosen the assertion until it passes.
- **Models need no extra crate.** Generate commands with
  `prop::collection::vec(command(), 1..40)`, apply each to the real code and
  to the model, and assert they agree after every step. proptest shrinks the
  list to the few commands that matter. For concurrency, generate the order
  in which workers run and drive them step by step; never sleep.
- **Budget the runs.** proptest's default is 256 cases. Workspace crates
  build at `opt-level = 1`, and Windows CI is the slowest job, so keep a pure
  property under about a second and a model on real temp directories under
  about 10 s. Lower `cases` with `ProptestConfig::with_cases(n)` rather than
  shrinking the inputs until they stop finding bugs.
- **Reproduce a failure from what proptest prints:** the minimal failing
  input. Locally, proptest also saves that case's seed as a `cc …` line in
  `proptest-regressions/` beside the test and runs it first next time.
  `PROPTEST_RNG_SEED=<n>` fixes the seed for a run, and
  `PROPTEST_CASES=10000` runs a property harder.
- **Don't commit `proptest-regressions/` files.** Turn the shrunk case into
  a named example test instead, so the regression reads as a sentence.
- **A property failure on an unrelated PR is a real bug.** Write the
  printed input as an example test and fix it or file it. Never re-run CI
  to get green.
