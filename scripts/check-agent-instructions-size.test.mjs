import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

import {
  ROOT_AGENTS_MD_MAX_BYTES,
  checkAgentInstructionsSize,
} from "./check-agent-instructions-size.mjs";

const CODEX_PROJECT_DOC_DEFAULT_MAX_BYTES = 32 * 1024;

test("the limit is 24 KiB, under Codex's default project-doc budget", () => {
  assert.equal(ROOT_AGENTS_MD_MAX_BYTES, 24_576);
  assert.ok(ROOT_AGENTS_MD_MAX_BYTES < CODEX_PROJECT_DOC_DEFAULT_MAX_BYTES);
});

test("passes exactly the sizes at or under the limit", () => {
  for (const limit of [0, 1, 100, ROOT_AGENTS_MD_MAX_BYTES]) {
    for (let bytes = Math.max(0, limit - 200); bytes <= limit + 200; bytes += 1) {
      const result = checkAgentInstructionsSize(bytes, limit);
      assert.equal(result.ok, bytes <= limit, `${bytes} of ${limit}`);
      if (!result.ok) {
        assert.match(result.message, new RegExp(`\\(${bytes - limit} over\\)`));
        assert.match(result.message, /when-to-read/);
      }
    }
  }
});

test("rejects a file Codex would truncate", () => {
  assert.equal(
    checkAgentInstructionsSize(CODEX_PROJECT_DOC_DEFAULT_MAX_BYTES + 1).ok,
    false,
  );
});

// Docs-only PRs skip Typecheck, so the classifier must run the check itself,
// and a failure there must fail its job rather than be swallowed.
test("CI's classifier runs the check before classifying", () => {
  const workflow = readFileSync(
    new URL("../.github/workflows/ci.yml", import.meta.url), "utf8");
  const job = workflow.split("\n  changes:\n")[1].split(/\n  [a-z][\w-]*:\n/)[0];
  const steps = job.split(/^      - /m);
  const check = steps.findIndex((s) =>
    s.includes("run: node ./scripts/check-agent-instructions-size.mjs"));
  const classify = steps.findIndex((s) => s.startsWith("name: Classify changed files"));
  assert.ok(check > 0 && check < classify);
  assert.doesNotMatch(steps[check], /continue-on-error/);
  const checkout = steps.find((s) => s.includes("uses: actions/checkout"));
  assert.match(checkout, /AGENTS\.md/);
  assert.match(checkout, /scripts\/check-agent-instructions-size\.mjs/);
});
