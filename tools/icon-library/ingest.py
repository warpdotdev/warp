"""
Ingest SVG icons from a directory into the database.

Usage:
    uv run ingest.py <directory>
    uv run ingest.py icons/sample
    uv run ingest.py --clear icons/sample   # wipe DB before ingesting
"""

import sys
import re
from pathlib import Path
import db


def name_from_filename(filename: str) -> str:
    """Convert a filename like 'arrow-right.svg' to a clean name 'arrow-right'."""
    return Path(filename).stem


def tags_from_name(name: str) -> list[str]:
    """Derive tags from the icon name by splitting on hyphens/underscores."""
    parts = re.split(r"[-_]", name.lower())
    tags = list(dict.fromkeys(parts))  # deduplicate while preserving order
    tags.append(name.lower())
    return tags


def ingest_directory(directory: Path, clear: bool = False) -> tuple[int, int]:
    """Returns (added, skipped) counts."""
    db.init_db()

    if clear:
        import sqlite3
        with db.get_connection() as conn:
            conn.execute("DELETE FROM icons")
            conn.commit()
        print("Database cleared.")

    svg_files = sorted(directory.glob("**/*.svg"))
    if not svg_files:
        print(f"No SVG files found in {directory}")
        return 0, 0

    added = 0
    skipped = 0
    for svg_path in svg_files:
        try:
            content = svg_path.read_text(encoding="utf-8").strip()
            name = name_from_filename(svg_path.name)
            tags = tags_from_name(name)
            db.upsert_icon(
                name=name,
                filename=svg_path.name,
                content=content,
                tags=tags,
                description=None,
            )
            print(f"  + {name} ({svg_path.name})")
            added += 1
        except Exception as e:
            print(f"  ! skipped {svg_path.name}: {e}")
            skipped += 1

    return added, skipped


def main():
    args = sys.argv[1:]
    clear = "--clear" in args
    args = [a for a in args if a != "--clear"]

    if not args:
        print("Usage: uv run ingest.py [--clear] <directory> [<directory2> ...]")
        sys.exit(1)

    total_added = total_skipped = 0
    for path_str in args:
        directory = Path(path_str)
        if not directory.exists():
            print(f"Directory not found: {directory}")
            sys.exit(1)
        print(f"\nIngesting from {directory}/")
        added, skipped = ingest_directory(directory, clear=clear)
        total_added += added
        total_skipped += skipped

    total = db.get_icon_count()
    print(f"\nDone. Added/updated: {total_added}  Skipped: {total_skipped}")
    print(f"Total icons in database: {total}")


if __name__ == "__main__":
    main()
