import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


SKILL_DIR = Path(__file__).resolve().parents[1]
SCRIPT = SKILL_DIR / "scripts" / "inspect_shared_settings.py"
FIXTURES = Path(__file__).parent / "fixtures"

spec = importlib.util.spec_from_file_location("inspect_shared_settings", SCRIPT)
inspector = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = inspector
spec.loader.exec_module(inspector)


class SharedSettingPathTests(unittest.TestCase):
    def test_selects_dynamic_gui_tui_intersection(self):
        schema = json.loads((FIXTURES / "settings_schema.json").read_text())

        self.assertEqual(
            inspector.shared_setting_paths(schema),
            [
                ("agent", "permissions", "command_policy"),
                ("agent", "permissions", "file_allowlist"),
            ],
        )

    def test_annotated_object_setting_does_not_descend_into_value_schema(self):
        schema = {
            "properties": {
                "shared_object": {
                    "x-warp-surfaces": ["gui", "tui"],
                    "properties": {"value_field": {"x-warp-surfaces": ["gui", "tui"]}},
                }
            }
        }

        self.assertEqual(
            inspector.shared_setting_paths(schema),
            [("shared_object",)],
        )


class InspectSharedSettingsCliTests(unittest.TestCase):
    def run_script(self, *args):
        return subprocess.run(
            [sys.executable, str(SCRIPT), *args],
            check=False,
            capture_output=True,
            text=True,
        )

    def test_nested_toml_outputs_only_shared_configured_values(self):
        result = self.run_script(
            "--schema",
            str(FIXTURES / "settings_schema.json"),
            "--source",
            str(FIXTURES / "gui_settings.toml"),
            "--destination",
            str(FIXTURES / "tui_settings.toml"),
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        output = json.loads(result.stdout)
        self.assertEqual(output["eligible_configured_count"], 2)
        by_path = {item["path"]: item for item in output["settings"]}
        self.assertEqual(
            by_path["agent.permissions.command_policy"]["state"],
            "destination_conflict",
        )
        self.assertEqual(
            by_path["agent.permissions.file_allowlist"]["state"],
            "missing_in_tui",
        )
        combined = result.stdout + result.stderr
        for excluded in (
            "GUI_PRIVATE_SENTINEL",
            "TUI_ONLY_SOURCE_SENTINEL",
            "GUI_THEME_SENTINEL",
            "PRIVATE_SETTING_SENTINEL",
        ):
            self.assertNotIn(excluded, combined)

    def test_missing_destination_is_treated_as_empty(self):
        with tempfile.TemporaryDirectory() as directory:
            result = self.run_script(
                "--schema",
                str(FIXTURES / "settings_schema.json"),
                "--source",
                str(FIXTURES / "gui_settings.toml"),
                "--destination",
                str(Path(directory) / "missing.toml"),
            )

        self.assertEqual(result.returncode, 0, result.stderr)
        output = json.loads(result.stdout)
        self.assertTrue(
            all(item["state"] == "missing_in_tui" for item in output["settings"])
        )

    def test_malformed_source_has_sanitized_error(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "source.toml"
            destination = Path(directory) / "destination.toml"
            secret = "MALFORMED_PRIVATE_SENTINEL"
            source.write_text(
                "[agent.permissions]\n"
                f'command_policy = [invalid\\nprivate = "{secret}"'
            )
            destination.write_text("")
            result = self.run_script(
                "--schema",
                str(FIXTURES / "settings_schema.json"),
                "--source",
                str(source),
                "--destination",
                str(destination),
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(result.stdout, "")
        self.assertNotIn(secret, result.stderr)
        self.assertIn("source_settings_unavailable_or_invalid", result.stderr)

    def test_invalid_unrelated_section_is_ignored(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "source.toml"
            destination = Path(directory) / "destination.toml"
            source.write_text(
                '[agent.permissions]\ncommand_policy = "always-ask"\n\n'
                "[privacy]\ninvalid_gui_only_value = [\n"
            )
            destination.write_text("")
            result = self.run_script(
                "--schema",
                str(FIXTURES / "settings_schema.json"),
                "--source",
                str(source),
                "--destination",
                str(destination),
            )

        self.assertEqual(result.returncode, 0, result.stderr)
        output = json.loads(result.stdout)
        self.assertEqual(output["eligible_configured_count"], 1)
        self.assertEqual(
            output["settings"][0]["path"],
            "agent.permissions.command_policy",
        )

    def test_missing_source_is_treated_as_empty(self):
        with tempfile.TemporaryDirectory() as directory:
            result = self.run_script(
                "--schema",
                str(FIXTURES / "settings_schema.json"),
                "--source",
                str(Path(directory) / "missing-source.toml"),
                "--destination",
                str(FIXTURES / "tui_settings.toml"),
            )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            json.loads(result.stdout),
            {"eligible_configured_count": 0, "settings": []},
        )

    def test_empty_source_path_fails_before_filesystem_access(self):
        with tempfile.TemporaryDirectory() as directory:
            destination = Path(directory) / "destination.toml"
            original = "canary = true\n"
            destination.write_text(original)
            result = self.run_script(
                "--schema",
                str(Path(directory) / "missing-schema.json"),
                "--source",
                "",
                "--destination",
                str(destination),
            )

            self.assertEqual(destination.read_text(), original)
        self.assertEqual(result.returncode, 2)
        self.assertEqual(result.stdout, "")
        self.assertIn("source_path_unavailable", result.stderr)

    def test_symlink_source_is_rejected(self):
        if sys.platform == "win32":
            self.skipTest("symlink creation is not generally available on Windows")
        with tempfile.TemporaryDirectory() as directory:
            link = Path(directory) / "source.toml"
            link.symlink_to(FIXTURES / "gui_settings.toml")
            result = self.run_script(
                "--schema",
                str(FIXTURES / "settings_schema.json"),
                "--source",
                str(link),
                "--destination",
                str(FIXTURES / "tui_settings.toml"),
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("settings_symlink_rejected", result.stderr)

    def test_dangling_destination_symlink_is_rejected(self):
        if sys.platform == "win32":
            self.skipTest("symlink creation is not generally available on Windows")
        with tempfile.TemporaryDirectory() as directory:
            destination = Path(directory) / "destination.toml"
            destination.symlink_to(Path(directory) / "missing-target.toml")
            result = self.run_script(
                "--schema",
                str(FIXTURES / "settings_schema.json"),
                "--source",
                str(FIXTURES / "gui_settings.toml"),
                "--destination",
                str(destination),
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("settings_symlink_rejected", result.stderr)


if __name__ == "__main__":
    unittest.main()
