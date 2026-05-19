"""
Icon Library — entry point.

Commands:
  uv run ingest.py <dir>          Ingest SVGs from a directory
  uv run query.py "<use case>"    Find icons for a use case
  uv run query.py --list          List all icons in the database
  uv run query.py --svg <name>    Print raw SVG for an icon

Quick start:
  uv run ingest.py icons/sample
  uv run query.py "a button to go back to the homepage"
"""

if __name__ == "__main__":
    print(__doc__)
