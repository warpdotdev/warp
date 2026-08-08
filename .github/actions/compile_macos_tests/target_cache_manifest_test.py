import tempfile
import unittest
from pathlib import Path

from target_cache_manifest import build_manifest


class BuildManifestTest(unittest.TestCase):
    def test_inventories_target_files_deterministically(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            target_dir = Path(directory)
            fingerprint_dir = target_dir / "debug" / ".fingerprint" / "warp-core"
            fingerprint_dir.mkdir(parents=True)
            (fingerprint_dir / "lib-warp-core").write_text("first")
            object_file = target_dir / "debug" / "warp"
            object_file.write_text("first object")

            first_manifest = build_manifest(target_dir)
            second_manifest = build_manifest(target_dir)

            self.assertEqual(first_manifest, second_manifest)
            self.assertEqual(first_manifest[1], 2)

            object_file.write_text("larger second object")

            self.assertNotEqual(first_manifest, build_manifest(target_dir))

    def test_changes_when_fingerprint_content_changes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            target_dir = Path(directory)
            fingerprint_dir = target_dir / "debug" / ".fingerprint" / "warp-core"
            fingerprint_dir.mkdir(parents=True)
            fingerprint = fingerprint_dir / "lib-warp-core"
            fingerprint.write_text("first")
            first_manifest = build_manifest(target_dir)

            fingerprint.write_text("second")

            self.assertNotEqual(first_manifest, build_manifest(target_dir))

    def test_hashes_symlink_targets(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            target_dir = Path(directory)
            first_target = target_dir / "first"
            second_target = target_dir / "other"
            first_target.mkdir()
            second_target.mkdir()
            link = target_dir / "linked-output"
            link.symlink_to(first_target, target_is_directory=True)
            first_manifest = build_manifest(target_dir)

            link.unlink()
            link.symlink_to(second_target, target_is_directory=True)

            self.assertNotEqual(first_manifest, build_manifest(target_dir))

    def test_ignores_seed_marker(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            target_dir = Path(directory)
            marker = target_dir / ".warp-cache-seed.env"
            marker.write_text("seed_attempt=1")
            first_manifest = build_manifest(target_dir)

            marker.write_text("seed_attempt=3")

            self.assertEqual(first_manifest, build_manifest(target_dir))


if __name__ == "__main__":
    unittest.main()
