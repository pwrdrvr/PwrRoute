#!/usr/bin/env node
// Release metadata gate, reduced from PwrGit (which ported it from PwrSnap).
// Verifies that a release tag, apps/desktop/package.json, electron-builder.yml,
// and CHANGELOG.md all agree before any signing/publishing work starts, and can
// extract the changelog section as release notes for `gh release create`.
// PwrGit's version also pins release.yml's job structure; that half returns
// when the release pipeline does.

import { readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const desktopPackagePath = resolve(repoRoot, "apps/desktop/package.json");
const electronBuilderPath = resolve(repoRoot, "apps/desktop/electron-builder.yml");
const changelogPath = resolve(repoRoot, "CHANGELOG.md");

function usage() {
  console.error("Usage: RELEASE_TAG=v0.0.1-alpha.1 pnpm release:check");
  console.error("   or: pnpm release:check --tag v0.0.1-alpha.1");
  console.error("   or: pnpm release:check --tag v0.0.1-alpha.1 --notes-file /tmp/RELEASE_NOTES.md");
}

function parseTagArg(argv) {
  const tagIndex = argv.indexOf("--tag");
  if (tagIndex !== -1) {
    return argv[tagIndex + 1] || "";
  }
  const inline = argv.find((arg) => arg.startsWith("--tag="));
  if (inline) {
    return inline.slice("--tag=".length);
  }
  return process.env.RELEASE_TAG || process.env.GITHUB_REF_NAME;
}

function parseNotesFileArg(argv) {
  const notesFileIndex = argv.indexOf("--notes-file");
  if (notesFileIndex !== -1) {
    return argv[notesFileIndex + 1] || "";
  }
  const inline = argv.find((arg) => arg.startsWith("--notes-file="));
  if (inline) {
    return inline.slice("--notes-file=".length);
  }
  return undefined;
}

function fail(message) {
  console.error(`release metadata check failed: ${message}`);
  process.exitCode = 1;
}

function escapeRegex(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function extractChangelogSection(changelog, version) {
  const headingPattern = new RegExp(`^##\\s+v?${escapeRegex(version)}(?:\\s|$)`);
  const nextHeadingPattern = /^##\s+/;
  const lines = changelog.split(/\r?\n/);
  const section = [];
  let inSection = false;

  for (const line of lines) {
    if (!inSection && headingPattern.test(line)) {
      inSection = true;
      continue;
    }
    if (inSection && nextHeadingPattern.test(line)) {
      break;
    }
    if (inSection) {
      section.push(line);
    }
  }

  return section.join("\n").trim();
}

const argv = process.argv.slice(2);
const tag = parseTagArg(argv);
if (!tag) {
  usage();
  fail("no release tag was provided");
  process.exit();
}

if (!/^v\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(tag)) {
  fail(`tag "${tag}" must look like vX.Y.Z or vX.Y.Z-prerelease`);
}

const expectedVersion = tag.slice(1);
const notesFile = parseNotesFileArg(argv);
if (notesFile === "") {
  usage();
  fail("--notes-file requires a path");
}
const desktopPackage = JSON.parse(readFileSync(desktopPackagePath, "utf8"));
if (desktopPackage.version !== expectedVersion) {
  fail(
    `apps/desktop/package.json version is ${desktopPackage.version}, but release tag ${tag} requires ${expectedVersion}`,
  );
}

const electronBuilder = readFileSync(electronBuilderPath, "utf8");
if (!/^\s*releaseType:\s*prerelease\s*$/m.test(electronBuilder)) {
  fail("apps/desktop/electron-builder.yml publish.releaseType must be prerelease");
}

const electronVersion = /^electronVersion:\s*(\S+)\s*$/m.exec(electronBuilder)?.[1];
const declaredElectron = desktopPackage.devDependencies?.electron?.replace(/^[\^~]/, "");
if (electronVersion === undefined || electronVersion !== declaredElectron) {
  fail(
    `apps/desktop/electron-builder.yml electronVersion (${electronVersion}) must match apps/desktop/package.json devDependencies.electron (${declaredElectron})`,
  );
}

const notes = extractChangelogSection(readFileSync(changelogPath, "utf8"), expectedVersion);
if (notes.length === 0) {
  fail(`CHANGELOG.md must contain a non-empty "## ${tag}" section`);
}

if (process.exitCode) {
  process.exit();
}

if (notesFile !== undefined) {
  writeFileSync(notesFile, `${notes}\n`);
}
console.log(`release metadata check passed for ${tag}`);
