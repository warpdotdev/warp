import importlib.util
import json
import os
import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock


SKILL_DIR = Path(__file__).resolve().parents[1]
SCRIPT = SKILL_DIR / "scripts" / "merge_mcp_config.py"
FIXTURES = Path(__file__).parent / "fixtures"

spec = importlib.util.spec_from_file_location("merge_mcp_config", SCRIPT)
merger = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = merger
spec.loader.exec_module(merger)


class MergeMcpConfigTests(unittest.TestCase):
    def run_script(self, source, destination, *mode):
        return subprocess.run(
            [
                sys.executable,
                str(SCRIPT),
                "--source",
                str(source),
                "--destination",
                str(destination),
                *mode,
            ],
            check=False,
            capture_output=True,
            text=True,
        )

    def copy_fixtures(self, directory):
        source = Path(directory) / "source.json"
        destination = Path(directory) / "destination.json"
        source.write_bytes((FIXTURES / "source_mcp.json").read_bytes())
        destination.write_bytes((FIXTURES / "destination_mcp.json").read_bytes())
        os.chmod(source, 0o600)
        os.chmod(destination, 0o600)
        return source, destination

    def dry_run(self, source, destination):
        result = self.run_script(source, destination, "--dry-run")
        self.assertEqual(result.returncode, 0, result.stderr)
        return result, json.loads(result.stdout)

    def apply(self, source, destination, fingerprint):
        return self.run_script(
            source,
            destination,
            "--apply",
            "--fingerprint",
            fingerprint,
        )

    def test_dry_run_is_redacted_and_does_not_mutate(self):
        with tempfile.TemporaryDirectory() as directory:
            source, destination = self.copy_fixtures(directory)
            original = destination.read_bytes()

            result, summary = self.dry_run(source, destination)

            self.assertEqual(destination.read_bytes(), original)
            self.assertEqual(summary["source_server_count"], 4)
            self.assertEqual(summary["eligible_source_count"], 2)
            self.assertEqual(summary["add_count"], 1)
            self.assertEqual(summary["conflict_count"], 1)
            self.assertEqual(summary["skipped_sensitive_count"], 1)
            self.assertEqual(summary["skipped_reinstall_count"], 1)
            self.assertTrue(summary["would_change"])
            combined = result.stdout + result.stderr
            source_document = json.loads(source.read_text())
            for server_name, definition in source_document["mcpServers"].items():
                self.assertNotIn(server_name, combined)
                self.assertNotIn(
                    json.dumps(definition, sort_keys=True),
                    combined,
                )
            for forbidden in (
                "SAFE_TOKEN",
                "${SAFE_TOKEN}",
                "LITERAL_SECRET_SENTINEL",
                "MANAGED_INSTALLATION_SENTINEL",
                "headers",
                "env",
            ):
                self.assertNotIn(forbidden, combined)

    def test_apply_is_destination_wins_and_preserves_placeholder(self):
        with tempfile.TemporaryDirectory() as directory:
            source, destination = self.copy_fixtures(directory)
            _, summary = self.dry_run(source, destination)
            result = self.apply(source, destination, summary["fingerprint"])

            self.assertEqual(result.returncode, 0, result.stderr)
            applied = json.loads(destination.read_text())
            servers = applied["servers"]
            self.assertEqual(
                servers["destination-conflict"]["url"],
                "https://destination.invalid/mcp",
            )
            self.assertEqual(
                servers["source-placeholder"]["env"]["SAFE_TOKEN"],
                "${SAFE_TOKEN}",
            )
            self.assertEqual(
                servers["source-placeholder"]["headers"]["Authorization"],
                "Bearer ${SAFE_TOKEN}",
            )
            self.assertNotIn("literal-sensitive", servers)
            self.assertNotIn("managed-installation", servers)
            self.assertTrue(applied["destinationMetadata"]["preserve"])
            output = json.loads(result.stdout)
            self.assertTrue(output["backup_created"])

    def test_apply_creates_restricted_backup_and_destination(self):
        if os.name != "posix":
            self.skipTest("Unix permission assertions")
        with tempfile.TemporaryDirectory() as directory:
            source, destination = self.copy_fixtures(directory)
            _, summary = self.dry_run(source, destination)
            result = self.apply(source, destination, summary["fingerprint"])

            self.assertEqual(result.returncode, 0, result.stderr)
            backups = list(Path(directory).glob("destination.json.backup-*"))
            self.assertEqual(len(backups), 1)
            self.assertEqual(stat.S_IMODE(destination.stat().st_mode), 0o600)
            self.assertEqual(stat.S_IMODE(backups[0].stat().st_mode), 0o600)

    def test_changed_input_rejects_apply_without_mutation(self):
        with tempfile.TemporaryDirectory() as directory:
            source, destination = self.copy_fixtures(directory)
            original_destination = destination.read_bytes()
            _, summary = self.dry_run(source, destination)
            source.write_text(
                source.read_text().replace("placeholder-command", "changed-command")
            )
            result = self.apply(source, destination, summary["fingerprint"])

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("fingerprint_mismatch", result.stderr)
            self.assertEqual(destination.read_bytes(), original_destination)
            self.assertEqual(
                list(Path(directory).glob("destination.json.backup-*")),
                [],
            )

    def test_apply_is_idempotent(self):
        with tempfile.TemporaryDirectory() as directory:
            source, destination = self.copy_fixtures(directory)
            _, first = self.dry_run(source, destination)
            self.assertEqual(
                self.apply(source, destination, first["fingerprint"]).returncode,
                0,
            )
            after_first = destination.read_bytes()

            _, second = self.dry_run(source, destination)
            self.assertEqual(second["add_count"], 0)
            self.assertFalse(second["would_change"])
            second_apply = self.apply(source, destination, second["fingerprint"])

            self.assertEqual(second_apply.returncode, 0, second_apply.stderr)
            self.assertEqual(destination.read_bytes(), after_first)
            self.assertFalse(json.loads(second_apply.stdout)["backup_created"])

    def test_missing_destination_uses_source_wrapper(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "source.json"
            destination = Path(directory) / "nested" / "destination.json"
            source.write_text(
                json.dumps(
                    {
                        "mcp": {
                            "servers": {
                                "new-server": {"url": "https://example.invalid/mcp"}
                            },
                            "unrelated": "do-not-copy",
                        },
                        "sourceMetadata": "do-not-copy",
                    }
                )
            )
            _, summary = self.dry_run(source, destination)
            result = self.apply(source, destination, summary["fingerprint"])

            self.assertEqual(result.returncode, 0, result.stderr)
            output = json.loads(destination.read_text())
            self.assertIn("servers", output["mcp"])
            self.assertNotIn("unrelated", output["mcp"])
            self.assertNotIn("sourceMetadata", output)
            self.assertFalse(json.loads(result.stdout)["backup_created"])
            if os.name == "posix":
                self.assertEqual(
                    stat.S_IMODE(destination.stat().st_mode),
                    0o600,
                )

    def test_missing_source_is_an_empty_no_op(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "missing-source.json"
            destination = Path(directory) / "destination.json"
            original = '{"mcpServers": {}}\n'
            destination.write_text(original)

            result, summary = self.dry_run(source, destination)

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(summary["source_server_count"], 0)
            self.assertEqual(summary["eligible_source_count"], 0)
            self.assertEqual(summary["add_count"], 0)
            self.assertFalse(summary["content_change_required"])
            self.assertEqual(
                summary["would_change"],
                summary["permission_update_required"],
            )
            self.assertEqual(destination.read_text(), original)

    def test_supported_wrapper_forms(self):
        wrappers = {
            "mcpServers": lambda servers: {"mcpServers": servers},
            "mcp_servers": lambda servers: {"mcp_servers": servers},
            "servers": lambda servers: {"servers": servers},
            "mcp.servers": lambda servers: {"mcp": {"servers": servers}},
            "flat": lambda servers: servers,
        }
        for name, wrap in wrappers.items():
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                source = Path(directory) / "source.json"
                destination = Path(directory) / "destination.json"
                source.write_text(
                    json.dumps(
                        wrap({"source-entry": {"command": "placeholder-command"}})
                    )
                )
                destination.write_text(
                    json.dumps(
                        wrap({"destination-entry": {"command": "destination-command"}})
                    )
                )

                _, summary = self.dry_run(source, destination)
                result = self.apply(source, destination, summary["fingerprint"])

                self.assertEqual(result.returncode, 0, result.stderr)
                parsed = merger.load_config(destination)
                self.assertEqual(len(parsed.servers), 2)
                self.assertEqual(
                    parsed.wrapper_path,
                    {
                        "mcpServers": ("mcpServers",),
                        "mcp_servers": ("mcp_servers",),
                        "servers": ("servers",),
                        "mcp.servers": ("mcp", "servers"),
                        "flat": (),
                    }[name],
                )

    def test_invalid_input_is_sanitized_no_op(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "source.json"
            destination = Path(directory) / "destination.json"
            secret = "INVALID_JSON_SECRET_SENTINEL"
            source.write_text('{"mcpServers": {"private": "' + secret)
            original = '{"servers": {}}\n'
            destination.write_text(original)

            result = self.run_script(source, destination, "--dry-run")

            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(result.stdout, "")
            self.assertIn("invalid_json", result.stderr)
            self.assertNotIn(secret, result.stderr)
            self.assertEqual(destination.read_text(), original)

    def test_literal_credentials_in_args_and_urls_are_skipped(self):
        servers = {
            "argument-secret": {
                "command": "command",
                "args": ["--api-key", "LITERAL_ARGUMENT_SECRET"],
            },
            "query-secret": {
                "url": "https://example.invalid/mcp?access_token=LITERAL_QUERY_SECRET"
            },
            "placeholder-argument": {
                "command": "command",
                "args": ["--api-key=${API_KEY}"],
            },
            "placeholder-query": {
                "url": "https://example.invalid/mcp?access_token=${ACCESS_TOKEN}"
            },
            "mixed-literal-placeholder": {
                "command": "command",
                "env": {"API_KEY": "LITERAL_PREFIX_${API_KEY}"},
            },
        }

        eligible, skipped_sensitive, skipped_reinstall = merger._classify_source(
            servers
        )

        self.assertEqual(len(eligible), 2)
        self.assertEqual(skipped_sensitive, 3)
        self.assertEqual(skipped_reinstall, 0)

    def test_invalid_source_transport_is_no_op(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "source.json"
            destination = Path(directory) / "destination.json"
            source.write_text(
                json.dumps(
                    {
                        "mcpServers": {
                            "invalid": {
                                "command": "command",
                                "url": "https://example.invalid/mcp",
                            }
                        }
                    }
                )
            )
            original = '{"servers": {}}\n'
            destination.write_text(original)

            result = self.run_script(source, destination, "--dry-run")

            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(result.stdout, "")
            self.assertIn("invalid_server_definition", result.stderr)
            self.assertNotIn("invalid", result.stderr.removeprefix("error: invalid"))
            self.assertEqual(destination.read_text(), original)

    def test_source_and_destination_symlinks_are_rejected(self):
        if os.name != "posix":
            self.skipTest("symlink creation is not generally available on Windows")
        for symlink_role in ("source", "destination"):
            with self.subTest(
                role=symlink_role
            ), tempfile.TemporaryDirectory() as directory:
                source, destination = self.copy_fixtures(directory)
                selected = source if symlink_role == "source" else destination
                target = selected.with_suffix(".target")
                selected.rename(target)
                selected.symlink_to(target)
                original = target.read_bytes()

                result = self.run_script(source, destination, "--dry-run")

                self.assertNotEqual(result.returncode, 0)
                self.assertIn("config_symlink_rejected", result.stderr)
                self.assertEqual(target.read_bytes(), original)

    def test_empty_source_path_fails_before_filesystem_access(self):
        with tempfile.TemporaryDirectory() as directory:
            destination = Path(directory) / "destination.json"
            original = '{"servers": {}}\n'
            destination.write_text(original)
            result = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT),
                    "--source",
                    "",
                    "--destination",
                    str(destination),
                    "--dry-run",
                ],
                check=False,
                capture_output=True,
                text=True,
                cwd=directory,
            )

            self.assertEqual(destination.read_text(), original)
            self.assertEqual(list(Path(directory).glob("*.backup-*")), [])
        self.assertEqual(result.returncode, 2)
        self.assertEqual(result.stdout, "")
        self.assertIn("source_path_unavailable", result.stderr)

    def test_atomic_replace_failure_leaves_destination_unchanged(self):
        with tempfile.TemporaryDirectory() as directory:
            source, destination = self.copy_fixtures(directory)
            original = destination.read_bytes()
            source_config = merger.load_config(source)
            destination_config = merger.load_config(destination)
            plan = merger.build_plan(source_config, destination_config)

            with mock.patch.object(
                merger.os, "replace", side_effect=OSError("private detail")
            ):
                with self.assertRaisesRegex(merger.MergeError, "atomic_write_failed"):
                    merger.apply_plan(plan, destination, destination_config)

            self.assertEqual(destination.read_bytes(), original)
            self.assertEqual(
                list(Path(directory).glob(".destination.json.stage-*")),
                [],
            )
            backups = list(Path(directory).glob("destination.json.backup-*"))
            self.assertEqual(len(backups), 1)
            self.assertEqual(backups[0].read_bytes(), original)

    def test_apply_repairs_destination_permissions_without_rewrite(self):
        if os.name != "posix":
            self.skipTest("Unix permission assertions")
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "source.json"
            destination = Path(directory) / "destination.json"
            contents = '{"servers": {"same": {"command": "command"}}}\n'
            source.write_text(contents)
            destination.write_text(contents)
            os.chmod(destination, 0o644)

            _, summary = self.dry_run(source, destination)
            self.assertFalse(summary["content_change_required"])
            self.assertTrue(summary["permission_update_required"])
            result = self.apply(source, destination, summary["fingerprint"])

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(destination.read_text(), contents)
            self.assertEqual(stat.S_IMODE(destination.stat().st_mode), 0o600)
            self.assertEqual(
                list(Path(directory).glob("destination.json.backup-*")),
                [],
            )


if __name__ == "__main__":
    unittest.main()
