use std::fs::{self, File};
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, anyhow, ensure};
use chrono::Utc;
use serde_json::{Value, json};
use tempfile::NamedTempFile;
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

pub(super) fn session_map_path() -> Result<PathBuf> {
    // pi-acp keeps its map under HOME even when Pi's session directory is overridden.
    Ok(dirs::home_dir()
        .ok_or_else(|| anyhow!("Cannot locate the pi-acp session map without a home directory"))?
        .join(".pi/pi-acp/session-map.json"))
}

fn mapped_session_file(session_dir: &Path, cwd: &Path, session_id: Uuid) -> Option<PathBuf> {
    let map_path = session_map_path().ok()?;
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
    parse_session_header(header.as_bytes(), cwd)
}

fn parse_session_header(header: &[u8], cwd: &Path) -> Option<Uuid> {
    let header = serde_json::from_slice::<Value>(header).ok()?;
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

pub(super) fn rehydrate_pi_transcript(
    transcript: &[u8],
    root: &Path,
    cwd: &Path,
    map_path: &Path,
) -> Result<Uuid> {
    let header_end = transcript
        .iter()
        .position(|byte| *byte == b'\n')
        .unwrap_or(transcript.len());
    let mut header: Value =
        serde_json::from_slice(&transcript[..header_end]).context("Invalid Pi session header")?;
    ensure!(
        header["type"] == "session" && header["cwd"].is_string(),
        "Invalid Pi session header"
    );
    let native_id = header["id"]
        .as_str()
        .and_then(|id| Uuid::parse_str(id).ok())
        .context("Invalid Pi session id")?;
    let relocated = header["cwd"]
        .as_str()
        .is_some_and(|path| Path::new(path) != cwd);
    let mut map: Value = match fs::read(map_path) {
        Ok(bytes) => serde_json::from_slice(&bytes).context("Invalid pi-acp session map")?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            json!({"version": 1, "sessions": {}})
        }
        Err(error) => return Err(error).context("Failed to read the pi-acp session map"),
    };
    ensure!(
        map["version"] == 1 && map["sessions"].is_object(),
        "Unsupported pi-acp session map"
    );

    let session_dir = root.join("sessions");
    fs::create_dir_all(&session_dir)?;
    let session_path = session_dir
        .canonicalize()?
        .join(format!("{native_id}.jsonl"));
    let mut file = NamedTempFile::new_in(&session_dir)?;
    if relocated {
        header["cwd"] = json!(cwd);
        serde_json::to_writer(&mut file, &header)?;
        file.write_all(&transcript[header_end..])?;
    } else {
        file.write_all(transcript)?;
    }
    if !transcript.ends_with(b"\n") {
        file.write_all(b"\n")?;
    }
    file.flush()?;
    file.persist(&session_path).map_err(|error| error.error)?;

    // The native transcript does not carry pi-acp's id; each restore needs its own mapping.
    let session_id = Uuid::new_v4();
    map["sessions"][session_id.to_string()] = json!({
        "sessionId": session_id,
        "cwd": cwd,
        "sessionFile": session_path,
        "updatedAt": Utc::now().to_rfc3339(),
    });
    let map_dir = map_path
        .parent()
        .context("pi-acp session map has no parent")?;
    fs::create_dir_all(map_dir)?;
    let mut file = NamedTempFile::new_in(map_dir)?;
    serde_json::to_writer(&mut file, &map)?;
    file.flush()?;
    file.persist(map_path).map_err(|error| error.error)?;
    Ok(session_id)
}

#[cfg(test)]
#[path = "pi_transcript_tests.rs"]
mod tests;
