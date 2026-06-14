import sqlite3
import json
from pathlib import Path
from datetime import datetime

DB_PATH = Path(__file__).parent / "icons.db"


def get_connection() -> sqlite3.Connection:
    conn = sqlite3.connect(DB_PATH)
    conn.row_factory = sqlite3.Row
    return conn


def init_db():
    with get_connection() as conn:
        conn.execute("""
            CREATE TABLE IF NOT EXISTS icons (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE,
                filename TEXT NOT NULL,
                content TEXT NOT NULL,
                tags TEXT NOT NULL DEFAULT '[]',
                description TEXT,
                ingested_at TEXT NOT NULL
            )
        """)
        conn.commit()


def upsert_icon(name: str, filename: str, content: str, tags: list[str], description: str | None):
    with get_connection() as conn:
        conn.execute("""
            INSERT INTO icons (name, filename, content, tags, description, ingested_at)
            VALUES (?, ?, ?, ?, ?, ?)
            ON CONFLICT(name) DO UPDATE SET
                filename=excluded.filename,
                content=excluded.content,
                tags=excluded.tags,
                description=excluded.description,
                ingested_at=excluded.ingested_at
        """, (name, filename, content, json.dumps(tags), description, datetime.utcnow().isoformat()))
        conn.commit()


def get_all_icons() -> list[dict]:
    with get_connection() as conn:
        rows = conn.execute("SELECT id, name, filename, tags, description FROM icons ORDER BY name").fetchall()
        return [
            {
                "id": r["id"],
                "name": r["name"],
                "filename": r["filename"],
                "tags": json.loads(r["tags"]),
                "description": r["description"],
            }
            for r in rows
        ]


def get_icon_content(name: str) -> str | None:
    with get_connection() as conn:
        row = conn.execute("SELECT content FROM icons WHERE name = ?", (name,)).fetchone()
        return row["content"] if row else None


def get_icon_count() -> int:
    with get_connection() as conn:
        return conn.execute("SELECT COUNT(*) FROM icons").fetchone()[0]


def delete_icon(name: str) -> bool:
    with get_connection() as conn:
        cursor = conn.execute("DELETE FROM icons WHERE name = ?", (name,))
        conn.commit()
        return cursor.rowcount > 0
