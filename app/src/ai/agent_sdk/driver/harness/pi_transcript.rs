use std::fs::{self, File};
use std::io::{BufRead as _, BufReader, Read as _};
use std::path::{Path, PathBuf};

use serde_json::Value;
use uuid::Uuid;

pub(super) fn find_session_file(root: &Path, cwd: &Path, session_id: Uuid) -> Option<PathBuf> {
    let session_dir = root.join("sessions");
    if let Some(path) = mapped_session_file(&session_dir, cwd, session_id) {
        return Some(path);
    }
    // Pi writes an explicit session directory flat; the adapter's id can differ from Pi's.
    let suffix = format!("_{session_id}.jsonl");
    fs::read_dir(&session_dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "jsonl")
        })
        .filter_map(|entry| session_header(&entry.path(), cwd).map(|id| (entry, id)))
        .max_by_key(|(entry, id)| {
            (
                *id == session_id,
                entry.file_name().to_string_lossy().ends_with(&suffix),
                entry
                    .metadata()
                    .ok()
                    .and_then(|metadata| metadata.modified().ok()),
            )
        })
        .map(|(entry, _)| entry.path())
}

fn mapped_session_file(session_dir: &Path, cwd: &Path, session_id: Uuid) -> Option<PathBuf> {
    // pi-acp keeps its map under HOME even when Pi's session directory is overridden.
    let map_path = dirs::home_dir()?.join(".pi/pi-acp/session-map.json");
    let map: Value = serde_json::from_slice(&fs::read(map_path).ok()?).ok()?;
    if map["version"] != 1 {
        return None;
    }
    let entry = map["sessions"].get(session_id.to_string())?;
    let path = Path::new(entry["sessionFile"].as_str()?)
        .canonicalize()
        .ok()?;
    if !path.starts_with(session_dir.canonicalize().ok()?) {
        return None;
    }
    session_header(&path, cwd)?;
    Some(path)
}

fn session_header(path: &Path, cwd: &Path) -> Option<Uuid> {
    let file = File::open(path).ok()?;
    let mut header = String::new();
    BufReader::new(file.take(8 * 1024 * 1024))
        .read_line(&mut header)
        .ok()?;
    let header = serde_json::from_str::<Value>(&header).ok()?;
    if header["type"] != "session"
        || header["cwd"]
            .as_str()
            .is_none_or(|path| Path::new(path) != cwd)
    {
        return None;
    }
    header["id"]
        .as_str()
        .and_then(|id| Uuid::parse_str(id).ok())
}
