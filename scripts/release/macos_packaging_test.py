"""No live credentials or keychain operations: security and xcrun are mocked."""

import base64
import importlib.util
import json
import os
from pathlib import Path
import plistlib
import signal
import struct
import tempfile
import unittest
from unittest.mock import patch


def load(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


signing = load("macos-signing")
layout = load("dmg-layout")


class SigningTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        self.original = ["/Users/fixture/Library/Keychains/login.keychain-db", "/tmp/Keychain With Spaces.keychain-db"]
        self.search = self.original[:]
        self.default = [self.original[0]]
        self.calls = []
        self.failure = None
        self.cancel = None
        self.addCleanup(signal.signal, signal.SIGINT, signal.getsignal(signal.SIGINT))
        self.addCleanup(signal.signal, signal.SIGTERM, signal.getsignal(signal.SIGTERM))
        self.env = patch.dict(os.environ, {
            "CSC_LINK": base64.b64encode(b"fixture-certificate").decode(),
            "CSC_KEY_PASSWORD": "fixture-password",
            "APPLE_API_KEY_BASE64": base64.b64encode(b"fixture-api-key").decode(),
            "APPLE_TEAM_ID": "FIXTURETEAM", "APPLE_API_KEY_ID": "fixture-id",
            "APPLE_API_ISSUER": "fixture-issuer", "GITHUB_OUTPUT": str(self.directory / "output"),
        })
        self.env.start()
        self.addCleanup(self.env.stop)

    def command(self, *args):
        self.calls.append(args)
        if args[1] == self.failure:
            raise RuntimeError("fixture command failed")
        if args[1] == self.cancel:
            self.cancel = None
            signal.raise_signal(signal.SIGTERM)
        if args[1] == "create-keychain":
            Path(args[-1]).touch()
        if args[1] == "delete-keychain":
            Path(args[-1]).unlink()
        if args[1] in ("list-keychains", "default-keychain"):
            attr = "search" if args[1] == "list-keychains" else "default"
            if "-s" in args:
                setattr(self, attr, list(args[args.index("-s") + 1:]))
            return "\n".join(f'    "{name}"' for name in getattr(self, attr))
        if args[1] in ("import", "notarytool"):
            filename = "cert.p12" if args[1] == "import" else "AuthKey.p8"
            self.assertEqual((self.directory / filename).stat().st_mode & 0o777, 0o600)
        if args[1] == "find-identity":
            return '1) FIXTURE "Developer ID Application: Fixture (FIXTURETEAM)"'
        return ""

    def assert_clean(self):
        self.assertEqual(self.search, self.original)
        for name in ("cert.p12", "AuthKey.p8", "signing.keychain-db", "keychain-state.json"):
            self.assertFalse((self.directory / name).exists(), name)
        for call in self.calls:
            if call[1] == "delete-keychain":
                self.assertEqual(call[2], str(self.directory / "signing.keychain-db"))

    def test_success_restores_exact_search_list_and_preserves_default(self):
        with patch.object(signing, "run", self.command):
            signing.prepare(self.directory)
            self.assertEqual(self.search[1:], self.original)
            self.assertFalse((self.directory / "cert.p12").exists())
            signing.cleanup(self.directory)
            signing.cleanup(self.directory)  # workflow retry is idempotent
        self.assert_clean()
        self.assertFalse(any(call[1] == "default-keychain" and "-s" in call for call in self.calls))

    def test_partial_import_failure_and_cancellation_clean_up(self):
        for operation in ("create-keychain", "unlock-keychain", "import", "set-key-partition-list", "find-identity", "notarytool"):
            with self.subTest(operation=operation):
                self.failure = operation
                with patch.object(signing, "run", self.command), self.assertRaises(RuntimeError):
                    signing.prepare(self.directory)
                self.assert_clean()
        self.failure = None
        self.cancel = "notarytool"
        with patch.object(signing, "run", self.command), self.assertRaises(SystemExit) as caught:
            signing.prepare(self.directory)
        self.assertEqual(caught.exception.code, 128 + signal.SIGTERM)
        self.assert_clean()

    def test_later_packaging_failure_still_restores_default_and_search(self):
        with patch.object(signing, "run", self.command):
            signing.prepare(self.directory)
            self.default = [str(self.directory / "signing.keychain-db")]
            signing.cleanup(self.directory)  # always() after a failed/cancelled pack step
        self.assertEqual(self.default, [self.original[0]])
        self.assert_clean()

    def test_restore_failure_still_deletes_keychain_and_decoded_files(self):
        with patch.object(signing, "run", self.command):
            signing.prepare(self.directory)
            (self.directory / "AuthKey.p8").write_text("fixture")
            self.failure = "list-keychains"
            with self.assertRaises(RuntimeError):
                signing.cleanup(self.directory)
            self.assertFalse((self.directory / "signing.keychain-db").exists())
            self.assertFalse((self.directory / "AuthKey.p8").exists())
            self.failure = None
            signing.cleanup(self.directory)
        self.assert_clean()

    def test_command_failure_does_not_print_password_or_subprocess_output(self):
        with patch.object(signing.subprocess, "run") as run:
            run.return_value.returncode = 1
            run.return_value.stderr = "fixture-secret"
            with self.assertRaisesRegex(RuntimeError, r"^security import failed \(exit 1\)$"):
                signing.run("security", "import", "-P", "fixture-secret")


class LayoutTests(unittest.TestCase):
    def test_layout_records_round_trip_and_alias_has_no_staging_paths(self):
        # Independent decoder of the leaf node, including the plist values.
        alias = b"fixture-volume-relative-alias"
        records = [(".", "bwsp", plistlib.dumps(layout.window_settings(), fmt=plistlib.FMT_BINARY)),
                   (".", "icvp", plistlib.dumps(layout.icon_view_settings(alias), fmt=plistlib.FMT_BINARY))]
        records += [(name, "Iloc", layout.icon_location(*xy)) for name, xy in layout.ITEM_POSITIONS.items()]
        data = layout.encode_ds_store(layout.encode_records(records), len(records))
        self.assertEqual(data[4:8], b"Bud1")
        # Follow the allocator table to DSDB, then the root leaf.
        allocator = struct.unpack_from(">I", data, 8)[0] + 4
        addresses = struct.unpack_from(">III", data, allocator + 8)
        tree = (addresses[1] & ~31) + 4
        root, levels, count, nodes, page_size = struct.unpack_from(">IIIII", data, tree)
        offset = (addresses[root] & ~31) + 4
        self.assertEqual((levels, count, nodes, page_size), (0, 4, 1, 4096))
        self.assertEqual(struct.unpack_from(">II", data, offset), (0, 4))
        offset += 8
        decoded = {}
        for _ in range(count):
            chars = struct.unpack_from(">I", data, offset)[0]
            offset += 4
            name = data[offset:offset + 2 * chars].decode("utf-16-be")
            offset += chars * 2
            kind, value_type, length = struct.unpack_from(">4s4sI", data, offset)
            self.assertEqual(value_type, b"blob")
            offset += 12
            decoded[(name, kind)] = data[offset:offset + length]
            offset += length
        self.assertEqual(plistlib.loads(decoded[(".", b"bwsp")])["WindowBounds"], "{{200, 120}, {660, 400}}")
        options = plistlib.loads(decoded[(".", b"icvp")])
        self.assertEqual(options["backgroundImageAlias"], alias)
        self.assertEqual((options["backgroundType"], options["iconSize"]), (2, 112))
        for name, xy in (("PwrRoute.app", (170, 230)), ("Applications", (500, 230))):
            self.assertEqual(struct.unpack_from(">II", decoded[(name, b"Iloc")]), xy)
        fake_stat = type("Stat", (), {"st_ino": 123, "st_birthtime": 1000})()
        with patch.object(layout.os, "stat", return_value=fake_stat):
            alias = layout.background_alias("/Users/private/staging/mount", "PwrRoute 1.2.3-universal", layout.BACKGROUND)
        self.assertNotIn(b"/Users/", alias)
        self.assertIn(b"/Volumes/PwrRoute 1.2.3-universal", alias)
        self.assertIn(b"/.background/background.png", alias)


if __name__ == "__main__":
    unittest.main()
