#!/usr/bin/env python3
"""Job-scoped keychain lifecycle, adapted from PwrAgnt release.mjs.

The workflow supplies a private PWRROUTE_SIGNING_TEMP_DIR and runs cleanup with
always(), including cancellation. Snapshot state before creating the keychain;
restore the exact search list (including paths with spaces) before deleting it.
Never change the default keychain for signing. No credentials or subprocess
arguments containing credentials are printed, including on command failure.
"""

import base64
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import uuid


def run(*args):
    result = subprocess.run(args, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    if result.returncode:
        raise RuntimeError(f"{args[0]} {args[1]} failed (exit {result.returncode})")
    return result.stdout


def keychains(output):
    # security prints one quoted path per line. Do not split on whitespace.
    return [line.strip().removeprefix('"').removesuffix('"')
            for line in output.splitlines() if line.strip()]


def cleanup(directory):
    state_file = directory / "keychain-state.json"
    failures = []
    try:
        if state_file.exists():
            state = json.loads(state_file.read_text())
            try:
                run("security", "list-keychains", "-d", "user", "-s", *state["search"])
            except RuntimeError as error:
                failures.append(str(error))
            try:
                current = keychains(run("security", "default-keychain", "-d", "user"))
                if current != state["default"]:
                    run("security", "default-keychain", "-d", "user", "-s", *state["default"])
            except RuntimeError as error:
                failures.append(str(error))
            keychain = directory / "signing.keychain-db"
            if keychain.exists():
                try:
                    run("security", "delete-keychain", str(keychain))
                except RuntimeError as error:
                    failures.append(str(error))
            if not failures:
                state_file.unlink()
    finally:
        for name in ("cert.p12", "AuthKey.p8"):
            (directory / name).unlink(missing_ok=True)
    if failures:
        raise RuntimeError("; ".join(failures))


def prepare(directory):
    os.umask(0o077)
    state = {
        "search": keychains(run("security", "list-keychains", "-d", "user")),
        "default": keychains(run("security", "default-keychain", "-d", "user")),
    }
    (directory / "keychain-state.json").write_text(json.dumps(state))

    # Actions first sends SIGINT, then SIGTERM when cancelling a running step.
    # Cleanup here covers import failures and signals; always() retries cleanup
    # after a cancelled/failed later packaging step on the persistent guest.
    def interrupted(signum, _frame):
        raise SystemExit(128 + signum)

    signal.signal(signal.SIGINT, interrupted)
    signal.signal(signal.SIGTERM, interrupted)
    try:
        keychain = str(directory / "signing.keychain-db")
        password = str(uuid.uuid4())
        certificate = directory / "cert.p12"
        certificate.write_bytes(base64.b64decode(os.environ["CSC_LINK"]))
        run("security", "create-keychain", "-p", password, keychain)
        run("security", "set-keychain-settings", "-lut", "21600", keychain)
        run("security", "unlock-keychain", "-p", password, keychain)
        run("security", "import", str(certificate), "-k", keychain,
            "-P", os.environ["CSC_KEY_PASSWORD"], "-T", "/usr/bin/codesign")
        run("security", "set-key-partition-list", "-S", "apple-tool:,apple:,codesign:",
            "-s", "-k", password, keychain)
        run("security", "list-keychains", "-d", "user", "-s", keychain, *state["search"])
        certificate.unlink()
        identities = run("security", "find-identity", "-v", "-p", "codesigning", keychain)
        team = re.escape(os.environ["APPLE_TEAM_ID"])
        match = re.search(r'"(Developer ID Application: [^"\r\n]+\(' + team + r'\))"', identities)
        if not match:
            raise RuntimeError("No Developer ID Application identity for the expected team")
        api_key = directory / "AuthKey.p8"
        api_key.write_bytes(base64.b64decode(os.environ["APPLE_API_KEY_BASE64"]))
        run("xcrun", "notarytool", "store-credentials", "pwrroute-notary", "--key", str(api_key),
            "--key-id", os.environ["APPLE_API_KEY_ID"], "--issuer", os.environ["APPLE_API_ISSUER"],
            "--keychain", keychain)
        api_key.unlink()
        with open(os.environ["GITHUB_OUTPUT"], "a") as output:
            output.write(f"path={keychain}\nidentity={match[1]}\n")
    except BaseException:
        cleanup(directory)
        raise


def main():
    directory = Path(os.environ["PWRROUTE_SIGNING_TEMP_DIR"])
    if sys.argv[1:] == ["prepare"]:
        prepare(directory)
    elif sys.argv[1:] == ["cleanup"]:
        cleanup(directory)
    else:
        raise RuntimeError("usage: macos-signing.py prepare|cleanup")


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        # No traceback: CalledProcessError/arguments must never reveal secrets.
        print(f"::error::Signing keychain operation failed: {error}", file=sys.stderr)
        sys.exit(1)
