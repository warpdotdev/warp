use std::fs::{self, File};
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, ensure};
use chrono::Utc;
use serde_json::{Value, json};
use tempfile::NamedTempFile;
use uuid::Uuid;

pub(super) fn rehydrate_pi_transcript(transcript: &[u8], root: &Path, cwd: &Path) -> Result<Uuid> {
    let header: Value = serde_json::from_slice(
        transcript
            .split(|byte| *byte == b'\n')
            .next()
            .unwrap_or_default(),
    )
    .context("Failed to parse Pi session header")?;
    ensure!(header["type"] == "session", "Missing Pi session header");
    let native_id = header["id"]
        .as_str()
        .context("Missing Pi session id")?
        .parse::<Uuid>()
        .context("Invalid Pi session id")?;
    ensure!(
        header["cwd"]
            .as_str()
            .is_some_and(|path| Path::new(path) == cwd),
        "Pi session working directory does not match the run"
    );

    let map_path = session_map_path().context("Could not locate pi-acp session map")?;
    let mut map: Value = match fs::read(&map_path) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).context("Failed to parse pi-acp session map")?
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            json!({"version": 1, "sessions": {}})
        }
        Err(error) => return Err(error).context("Failed to read pi-acp session map"),
    };
    ensure!(
        map["version"] == 1 && map["sessions"].is_object(),
        "Unsupported pi-acp session map"
    );

    let sessions = root.join("sessions");
    fs::create_dir_all(&sessions).context("Failed to create Pi session directory")?;
    let mut file = NamedTempFile::new_in(&sessions)?;
    file.write_all(transcript)?;
    if !transcript.ends_with(b"\n") {
        file.write_all(b"\n")?;
    }
    let path = sessions.join(format!("{native_id}.jsonl"));
    file.persist(&path).map_err(|error| error.error)?;
    let path = path.canonicalize()?;

    // The native transcript does not retain pi-acp's independently generated session id.
    let session_id = Uuid::new_v4();
    map["sessions"][session_id.to_string()] = json!({
        "sessionId": session_id,
        "cwd": cwd,
        "sessionFile": path,
        "updatedAt": Utc::now().to_rfc3339(),
    });
    let map_dir = map_path
        .parent()
        .context("Invalid pi-acp session map path")?;
    fs::create_dir_all(map_dir)?;
    let mut file = NamedTempFile::new_in(map_dir)?;
    serde_json::to_writer(&mut file, &map)?;
    file.flush()?;
    file.persist(map_path).map_err(|error| error.error)?;
    Ok(session_id)
}

fn session_map_path() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join(".pi/pi-acp/session-map.json"))
}

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
    let map_path = session_map_path()?;
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

#[cfg(test)]
#[path = "pi_transcript_tests.rs"]
mod tests;
