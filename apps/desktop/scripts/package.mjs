#!/usr/bin/env node
/**
 * Package the desktop app with electron-builder, signed when the credentials
 * are present and unsigned when they are not.
 *
 *   node scripts/package.mjs --mac|--win|--linux [--require-signing] [--arch=<arch>]
 *
 * Build first (`electron-vite build`); the package:* scripts do that.
 *
 * A reduced stand-in for PwrGit's apps/desktop/scripts/release.mjs. The stub
 * ships nothing outside out/, so it packages straight from the workspace and
 * skips PwrGit's pnpm-deploy stage, native staging and asar audits. What it
 * keeps is the signing decision, so release.yml lights up the moment the
 * secrets are added:
 *
 * - macOS signs with CSC_LINK / CSC_KEY_PASSWORD and notarizes with
 *   APPLE_API_KEY_BASE64 / APPLE_API_KEY_ID / APPLE_API_ISSUER. Without a
 *   certificate it signs ad hoc (identity "-"): electron-builder rewrites the
 *   Electron binary to set fuses, which breaks its original signature, and an
 *   unsigned result is killed by macOS on launch.
 * - Windows signs through Azure Artifact Signing when every WIN_AZURE_SIGN_*
 *   variable is set (AZURE_* service-principal secrets authenticate it).
 * - --require-signing turns a missing credential into a failure instead of an
 *   unsigned build. release.yml's signed lanes pass it.
 */

import { spawnSync } from "node:child_process";
import { chmodSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const desktopRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const platform = ["--mac", "--win", "--linux"].find((flag) => args.includes(flag));
const requireSigning = args.includes("--require-signing");
const arch = args.find((arg) => arg.startsWith("--arch="))?.slice("--arch=".length);

if (platform === undefined) {
  console.error("usage: package.mjs --mac|--win|--linux [--require-signing] [--arch=<arch>]");
  process.exit(2);
}

const env = { ...process.env };
const builderArgs = ["--config", "electron-builder.yml", platform, "--publish", "never"];
if (arch !== undefined) builderArgs.push(`--${arch}`);

function present(name) {
  return typeof env[name] === "string" && env[name].length > 0;
}

function fail(message) {
  console.error(`[package] ${message}`);
  process.exit(1);
}

if (platform === "--mac") {
  const canSign = present("CSC_LINK");
  const canNotarize = ["APPLE_API_KEY_BASE64", "APPLE_API_KEY_ID", "APPLE_API_ISSUER"].every(present);
  if (requireSigning && !(canSign && canNotarize)) {
    fail("--require-signing: CSC_LINK and APPLE_API_KEY_BASE64/APPLE_API_KEY_ID/APPLE_API_ISSUER are required.");
  }
  if (canSign) {
    env.CSC_IDENTITY_AUTO_DISCOVERY = "true";
  } else {
    // Ad hoc, not none: see the header. Hardened-runtime library validation
    // rejects an ad hoc Electron Framework, so drop it for this build only.
    builderArgs.push("--config.mac.identity=-", "--config.mac.hardenedRuntime=false");
  }
  if (canSign && canNotarize) {
    // electron-builder's notarize step wants a key file, not base64.
    const keyPath = join(tmpdir(), `AuthKey_${env.APPLE_API_KEY_ID}.p8`);
    writeFileSync(keyPath, Buffer.from(env.APPLE_API_KEY_BASE64, "base64"));
    chmodSync(keyPath, 0o600);
    env.APPLE_API_KEY = keyPath;
  } else {
    builderArgs.push("--config.mac.notarize=false");
  }
  console.log(`[package] macOS: ${canSign ? "Developer ID signed" : "ad hoc signed"}, ${canSign && canNotarize ? "notarized" : "not notarized"}`);
}

if (platform === "--win") {
  const config = ["WIN_AZURE_SIGN_PUBLISHER_NAME", "WIN_AZURE_SIGN_ENDPOINT", "WIN_AZURE_SIGN_ACCOUNT", "WIN_AZURE_SIGN_PROFILE"];
  const credentials = ["AZURE_TENANT_ID", "AZURE_CLIENT_ID", "AZURE_CLIENT_SECRET"];
  const canSign = config.every(present) && credentials.every(present);
  if (config.some(present) && !canSign) {
    const missing = [...config, ...credentials].filter((name) => !present(name));
    fail(`Windows signing is partly configured; missing ${missing.join(", ")}. Unset WIN_AZURE_SIGN_* to build unsigned.`);
  }
  if (requireSigning && !canSign) {
    fail("--require-signing: WIN_AZURE_SIGN_* variables and AZURE_* secrets are required.");
  }
  if (canSign) {
    builderArgs.push(
      `--config.win.azureSignOptions.publisherName=${env.WIN_AZURE_SIGN_PUBLISHER_NAME}`,
      `--config.win.azureSignOptions.endpoint=${env.WIN_AZURE_SIGN_ENDPOINT}`,
      `--config.win.azureSignOptions.codeSigningAccountName=${env.WIN_AZURE_SIGN_ACCOUNT}`,
      `--config.win.azureSignOptions.certificateProfileName=${env.WIN_AZURE_SIGN_PROFILE}`,
    );
  }
  console.log(`[package] Windows: ${canSign ? "Azure Artifact Signing" : "UNSIGNED"}`);
}

const require = createRequire(import.meta.url);
const cli = join(dirname(require.resolve("electron-builder/package.json", { paths: [desktopRoot] })), "cli.js");
const result = spawnSync(process.execPath, [cli, ...builderArgs], {
  cwd: desktopRoot,
  env,
  stdio: "inherit",
});
if (result.error) throw result.error;
process.exitCode = result.status ?? 1;
