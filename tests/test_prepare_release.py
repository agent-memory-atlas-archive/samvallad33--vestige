"""Release staging tests use labeled fixtures, never product binaries or publication."""

import importlib.util
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("prepare_release", ROOT / "scripts/prepare-release.py")
RELEASE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RELEASE)


class ReleasePreparationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="vestige-release-fixture-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.build = self.root / "target"
        self.assets = self.root / "assets"

    def archive(self, target):
        output = self.build / target / "release"
        output.mkdir(parents=True)
        for name in RELEASE.expected_files(target):
            (output / name).write_bytes(f"TEST FIXTURE {target} {name}\n".encode())
        RELEASE.package_archive(target, self.build, self.assets)
        return self.assets / RELEASE.archive_name(target)

    def test_each_current_target_has_exact_archive_members_and_checksum(self):
        for target in RELEASE.TARGETS:
            self.archive(target)
            record = RELEASE.inspect_archive(target, self.assets)
            self.assertEqual(record["target"], target)
            self.assertEqual(len(record["sha256"]), 64)
        self.assertEqual(len(list(self.assets.glob("*.sha256"))), 6)

    def test_archive_refuses_missing_binaries_and_overwrite(self):
        target = "aarch64-apple-darwin"
        with self.assertRaisesRegex(ValueError, "missing"):
            RELEASE.package_archive(target, self.build, self.assets)
        self.archive(target)
        with self.assertRaisesRegex(ValueError, "overwrite"):
            RELEASE.package_archive(target, self.build, self.assets)

    def test_changed_archive_fails_checksum_before_extraction(self):
        target = "aarch64-apple-darwin"
        archive = self.archive(target)
        archive.write_bytes(archive.read_bytes() + b"changed")
        with self.assertRaisesRegex(ValueError, "checksum"):
            RELEASE.inspect_archive(target, self.assets)

    def test_archive_rejects_traversal_and_links_even_with_matching_checksum(self):
        target = "aarch64-apple-darwin"
        self.assets.mkdir()
        archive = self.assets / RELEASE.archive_name(target)
        for attack in ("traversal", "link"):
            with self.subTest(attack=attack):
                with tarfile.open(archive, "w:gz") as bundle:
                    for index, name in enumerate(RELEASE.expected_files(target)):
                        info = tarfile.TarInfo("../escape" if attack == "traversal" and index == 0 else name)
                        if attack == "link" and index == 0:
                            info.type = tarfile.SYMTYPE
                            info.linkname = "../../outside"
                            bundle.addfile(info)
                        else:
                            info.size = 7
                            bundle.addfile(info, io.BytesIO(b"fixture"))
                checksum = RELEASE.digest(archive)
                archive.with_name(archive.name + ".sha256").write_text(f"{checksum}  {archive.name}\n")
                with self.assertRaises(ValueError):
                    RELEASE.inspect_archive(target, self.assets)

    def test_npm_staging_requires_all_targets_before_writing(self):
        self.archive("aarch64-apple-darwin")
        staging = self.root / "npm"
        with self.assertRaises(FileNotFoundError):
            RELEASE.stage_npm("5.0.1", self.assets, staging)
        self.assertFalse(staging.exists())

    def test_npm_staging_exact_pins_all_companions_and_keeps_source_manifest(self):
        for target in RELEASE.TARGETS:
            self.archive(target)
        manifest = ROOT / "packages/vestige-mcp-npm/package.json"
        before = manifest.read_bytes()
        staging = self.root / "npm"
        RELEASE.stage_npm("5.0.1", self.assets, staging)
        for package in ("vestige-mcp-npm", "vestige-init"):
            spec = json.loads((staging / package / "package.json").read_text())
            self.assertEqual(len(spec["optionalDependencies"]), 5)
            self.assertEqual(set(spec["optionalDependencies"].values()), {"5.0.1"})
        for platform in RELEASE.TARGETS.values():
            if platform:
                expected = {f"{name}.exe" if platform == "win32-x64" else name for name in RELEASE.BINARIES}
                self.assertEqual({file.name for file in (staging / f"mcp-{platform}" / "bin").iterdir()}, expected)
        self.assertEqual(manifest.read_bytes(), before)
        self.assertFalse(json.loads((staging / "preparation.json").read_text())["published"])
        with self.assertRaisesRegex(ValueError, "already exists"):
            RELEASE.stage_npm("5.0.1", self.assets, staging)


if __name__ == "__main__":
    unittest.main()
