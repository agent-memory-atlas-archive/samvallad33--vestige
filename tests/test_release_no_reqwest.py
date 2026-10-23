"""The release dependency gate must fail closed on real Cargo errors on every host."""
import contextlib
import importlib.util
import io
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import Mock

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("no_reqwest", ROOT / "scripts/check-release-no-reqwest.py")
CHECK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECK)


class ReleaseDependencyTests(unittest.TestCase):
    def test_crlf_workflow_preserves_distinct_feature_arguments(self):
        text = ('target: x86_64-pc-windows-msvc\r\n'
                'cargo_flags: "--no-default-features --features proof"\r\n'
                'target: aarch64-apple-darwin\r\n'
                'cargo_flags: ""\r\n'
                'cargo build -p vestige-mcp --release\r\n'
                'cargo ndk -t arm64-v8a build -p vestige-mcp --no-default-features\r\n')
        sets = CHECK.release_feature_sets(text)
        self.assertEqual([args for _, args in sets],
                         [["--no-default-features", "--features", "proof"], [], ["--no-default-features"]])

    def test_real_workflow_covers_default_and_minimal_release_builds(self):
        sets = CHECK.release_feature_sets((ROOT / ".github/workflows/release.yml").read_text())
        self.assertIn([], [args for _, args in sets])
        self.assertIn(["--no-default-features"], [args for _, args in sets])

    def test_missing_release_builds_refused(self):
        with self.assertRaisesRegex(ValueError, "no release feature sets"):
            CHECK.release_feature_sets("jobs: {}\r\n")

    def test_absence_passes_with_exact_arguments_and_windows_line_endings(self):
        run = Mock(return_value=SimpleNamespace(returncode=101, stdout=
                   "error: package ID specification `reqwest` did not match any packages\r\n"))
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertTrue(CHECK.check_feature_set("minimal", ["--no-default-features"], run))
        command = run.call_args.args[0]
        self.assertEqual(command[-1], "--no-default-features")
        self.assertFalse(any("\r" in arg or "\n" in arg for arg in command))

    def test_dependency_or_unproven_cargo_error_refused(self):
        for status, output in [(0, "reqwest v0.12.0"), (1, "reqwest v0.12.0"),
                               (101, "error: failed to download dependency"), (0, "")]:
            with self.subTest(status=status, output=output):
                run = Mock(return_value=SimpleNamespace(returncode=status, stdout=output))
                with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                    self.assertFalse(CHECK.check_feature_set("release", [], run))


if __name__ == "__main__":
    unittest.main()
