#!/usr/bin/env python3
"""Generate Warp's registry seed from pinned VS Code and Neovim checkouts."""

from __future__ import annotations

import argparse
import csv
import json
import re
import subprocess
from collections import defaultdict
from dataclasses import dataclass
from pathlib import Path

VS_CODE_REVISION = "ae0e51e8241f55c818f43e816fda2fcfd2a53ea3"
NEOVIM_REVISION = "d06bac614c2bc09db62e439b2e42ac7101653333"
WARP_REVISION = "abea51cd1e102b363935f1b25ef03d335bc7b36f"


@dataclass(frozen=True)
class BaselineEntry:
    kind: str
    selector: str
    grammar: str | None
    built_in_lsp: str | None


@dataclass(frozen=True)
class SeedEntry:
    kind: str
    selector: str
    identity: str
    grammar: str | None
    language_id: str | None
    built_in_lsp: str | None
    source: str


BUILT_IN_LANGUAGE_IDS = {
    "Rust": "rust",
    "Go": "go",
    "Python": "python",
    "TypeScript": "typescript",
    "TypeScriptReact": "typescriptreact",
    "JavaScript": "javascript",
    "JavaScriptReact": "javascriptreact",
    "C": "c",
    "Cpp": "cpp",
}


def decode_utf8_lf(data: bytes, description: str) -> str:
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError as error:
        raise SystemExit(f"{description} is not valid UTF-8: {error}") from error
    if "\r" in text:
        raise SystemExit(f"{description} must use LF line endings")
    return text


def git_output(root: Path, arguments: list[str], description: str) -> str:
    result = subprocess.run(
        ["git", *arguments],
        cwd=root,
        check=True,
        capture_output=True,
    )
    return decode_utf8_lf(result.stdout, description)


def git_show(root: Path, revision: str, path: str, description: str) -> str:
    return git_output(root, ["show", f"{revision}:{path}"], description)


def add_mapping(
    mappings: dict[tuple[str, str], set[str]], kind: str, selector: str, identity: str
) -> None:
    mappings[(kind, selector)].add(identity)


def load_vscode(root: Path) -> dict[tuple[str, str], set[str]]:
    mappings: dict[tuple[str, str], set[str]] = defaultdict(set)
    tree = git_output(
        root,
        ["ls-tree", "-r", "--name-only", VS_CODE_REVISION, "extensions"],
        "VS Code source tree",
    )
    package_paths = sorted(
        path
        for path in tree.splitlines()
        if re.fullmatch(r"extensions/[^/]+/package\.json", path)
    )
    for package_path in package_paths:
        source = git_show(
            root,
            VS_CODE_REVISION,
            package_path,
            f"VS Code {package_path}",
        )
        package = json.loads(source)
        for language in package.get("contributes", {}).get("languages", []):
            identity = language["id"]
            for extension in language.get("extensions", []):
                add_mapping(mappings, "extension", extension.removeprefix("."), identity)
            for filename in language.get("filenames", []):
                add_mapping(mappings, "filename", filename, identity)
    return mappings


def literal_lua_mappings(block: str, kind: str) -> dict[tuple[str, str], set[str]]:
    mappings: dict[tuple[str, str], set[str]] = defaultdict(set)
    assignment = re.compile(
        r"^  (?:\[['\"](?P<bracket>[^'\"]+)['\"]\]|(?P<plain>[A-Za-z0-9_+.-]+))"
        r"\s*=\s*['\"](?P<identity>[^'\"]+)['\"]\s*,",
        re.MULTILINE,
    )
    for match in assignment.finditer(block):
        selector = match.group("bracket") or match.group("plain")
        add_mapping(mappings, kind, selector, match.group("identity"))
    return mappings


def load_neovim(root: Path) -> dict[tuple[str, str], set[str]]:
    path = "runtime/lua/vim/filetype.lua"
    source = git_show(root, NEOVIM_REVISION, path, f"Neovim {path}")
    extension_start = source.index("local extension = {")
    filename_start = source.index("local filename = {")
    pattern_start = source.index("local pattern = {")

    mappings = literal_lua_mappings(
        source[extension_start:filename_start], "extension"
    )
    for key, identities in literal_lua_mappings(
        source[filename_start:pattern_start], "filename"
    ).items():
        mappings[key].update(identities)
    return mappings


def optional(value: str) -> str | None:
    return None if value == "-" else value


def load_baseline(path: Path) -> list[BaselineEntry]:
    rows = []
    source_text = decode_utf8_lf(path.read_bytes(), str(path))
    for row in csv.reader(
        (line for line in source_text.splitlines() if not line.startswith("#")),
        delimiter="\t",
    ):
        if not row:
            continue
        kind, selector, grammar, built_in_lsp = row
        built_in_lsp = optional(built_in_lsp)
        if built_in_lsp is not None and built_in_lsp not in BUILT_IN_LANGUAGE_IDS:
            raise SystemExit(
                f"unknown built-in LSP language for {kind} {selector}: {built_in_lsp}"
            )
        rows.append(
            BaselineEntry(
                kind,
                selector,
                optional(grammar),
                built_in_lsp,
            )
        )
    return rows


def single_mapping(
    mappings: dict[tuple[str, str], set[str]], entry: BaselineEntry, source: str
) -> str | None:
    candidates = mappings.get((entry.kind, entry.selector), set())
    if len(candidates) > 1:
        options = ", ".join(sorted(candidates))
        raise SystemExit(
            f"ambiguous {source} mapping for {entry.kind} {entry.selector}: {options}"
        )
    return next(iter(candidates), None)


def build_seed(
    baseline: list[BaselineEntry],
    vscode: dict[tuple[str, str], set[str]],
    neovim: dict[tuple[str, str], set[str]],
) -> list[SeedEntry]:
    seed = []
    for entry in baseline:
        if entry.built_in_lsp:
            language_id = BUILT_IN_LANGUAGE_IDS[entry.built_in_lsp]
            source = "warp-lsp"
        else:
            language_id = single_mapping(vscode, entry, "VS Code")
            source = "vscode"
            if language_id is None:
                language_id = single_mapping(neovim, entry, "Neovim")
                source = "neovim" if language_id else "warp-grammar"

        identity = language_id or entry.grammar
        if identity is None:
            raise SystemExit(f"entry has no identity: {entry.kind} {entry.selector}")
        seed.append(
            SeedEntry(
                entry.kind,
                entry.selector,
                identity,
                entry.grammar,
                language_id,
                entry.built_in_lsp,
                source,
            )
        )
    return seed


def write_fixture(path: Path, seed: list[SeedEntry]) -> None:
    with path.open("w", encoding="utf-8", newline="") as output:
        output.write(f"# warp_revision\t{WARP_REVISION}\n")
        output.write(f"# vscode_revision\t{VS_CODE_REVISION}\n")
        output.write(f"# neovim_revision\t{NEOVIM_REVISION}\n")
        output.write(
            "# selector_kind\tselector\tid\tgrammar\tlanguage_id\tbuilt_in_lsp\tsource\n"
        )
        writer = csv.writer(output, delimiter="\t", lineterminator="\n")
        for entry in seed:
            writer.writerow(
                [
                    entry.kind,
                    entry.selector,
                    entry.identity,
                    entry.grammar or "-",
                    entry.language_id or "-",
                    entry.built_in_lsp or "-",
                    entry.source,
                ]
            )


def rust_string(value: str) -> str:
    return '"' + value.replace("\\", "\\\\").replace('"', '\\"') + '"'


def rust_option(value: str | None) -> str:
    return "None" if value is None else f"Some({rust_string(value)})"


def write_rust(path: Path, seed: list[SeedEntry]) -> None:
    grouped: dict[
        tuple[str, str, str | None, str | None, str | None], list[str]
    ] = defaultdict(list)
    for entry in seed:
        grouped[
            (
                entry.kind,
                entry.identity,
                entry.grammar,
                entry.language_id,
                entry.built_in_lsp,
            )
        ].append(entry.selector)

    lines = [
        "// @generated by script/generate_seed.py; do not edit by hand.",
        f"// Warp: {WARP_REVISION}",
        f"// VS Code: {VS_CODE_REVISION}",
        f"// Neovim: {NEOVIM_REVISION}",
        "pub const LANGUAGE_ENTRIES: &[LanguageEntry] = &[",
    ]
    kind_order = {"filename": 0, "filename_prefix": 1, "extension": 2}
    for (kind, identity, grammar, language_id, built_in_lsp), selectors in sorted(
        grouped.items(), key=lambda item: (kind_order[item[0][0]], item[0][1], item[1])
    ):
        rendered = ", ".join(rust_string(selector) for selector in selectors)
        fields = {
            "extensions": "NONE",
            "filenames": "NONE",
            "filename_prefixes": "NONE",
        }
        fields[
            {
                "extension": "extensions",
                "filename": "filenames",
                "filename_prefix": "filename_prefixes",
            }[kind]
        ] = f"&[{rendered}]"
        lines.extend(
            [
                "    LanguageEntry {",
                f"        id: {rust_string(identity)},",
                f'        extensions: {fields["extensions"]},',
                f'        filenames: {fields["filenames"]},',
                f'        filename_prefixes: {fields["filename_prefixes"]},',
                f"        language_id: {rust_option(language_id)},",
                "        built_in_lsp: "
                + (
                    "None"
                    if built_in_lsp is None
                    else f"Some(BuiltInLspLanguage::{built_in_lsp})"
                )
                + ",",
                f"        grammar: {rust_option(grammar)},",
                "    },",
            ]
        )
    lines.extend(["];"])
    with path.open("w", encoding="utf-8", newline="") as output:
        output.write("\n".join(lines) + "\n")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--vscode-root", type=Path, required=True)
    parser.add_argument("--neovim-root", type=Path, required=True)
    args = parser.parse_args()

    crate_root = Path(__file__).resolve().parents[1]
    baseline = load_baseline(crate_root / "fixtures/warp_baseline.tsv")
    seed = build_seed(baseline, load_vscode(args.vscode_root), load_neovim(args.neovim_root))
    write_fixture(crate_root / "fixtures/registry_seed.tsv", seed)
    write_rust(crate_root / "src/generated.rs", seed)


if __name__ == "__main__":
    main()
