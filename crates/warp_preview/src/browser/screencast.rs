// Streams run on their own native threads, which block on channels by design.
#![allow(clippy::disallowed_methods)]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use base64::Engine;
use command::blocking::Command;
use serde_json::{Value, json};

use super::cdp::{Cdp, Message, devtools_url};
use crate::stream::Control;
use crate::{Error, Rate, Stream, StreamEvent, jpeg};

/// How long Chromium gets to start and print its DevTools address.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(20);

/// Longest side of frames for the preview in front, in pixels.
const FULL_MAX_SIDE: u32 = 1920;
/// Longest side of frames for cards behind it.
const THUMBNAIL_MAX_SIDE: u32 = 480;
/// Cards behind the front one get one frame in this many.
const THUMBNAIL_EVERY_NTH_FRAME: u32 = 8;

/// How many characters of a console message a log line keeps.
const MAX_LOG_LINE: usize = 500;

pub(crate) fn start_screencast(
    chromium: PathBuf,
    url: String,
    viewport: (u32, u32),
    rate: Rate,
    events: async_channel::Sender<StreamEvent>,
) -> Result<Stream, Error> {
    let (control_tx, control_rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("preview-browser".to_owned())
        .spawn(move || {
            let profile = profile_dir();
            let result = run(
                &chromium,
                &profile,
                &url,
                viewport,
                rate,
                &control_rx,
                &events,
            );
            let _ = std::fs::remove_dir_all(&profile);
            if let Err(message) = result {
                let _ = events.send_blocking(StreamEvent::Ended(message));
            }
        })
        .map_err(|err| Error::Other(err.to_string()))?;
    Ok(Stream::new(control_tx))
}

/// A fresh profile per preview, so previews never see the user's cookies or each other's.
fn profile_dir() -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "warp-preview-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

/// Ends the Chromium processes and removes the profiles of previews from Warp runs that are no
/// longer alive. Chromium outlives a crashed Warp, so its processes and profiles would otherwise
/// stay behind.
pub(crate) fn clean_up_after_crashed_runs() {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(owner) = name
            .to_str()
            .and_then(|name| name.strip_prefix("warp-preview-"))
            .and_then(|rest| rest.split('-').next())
            .and_then(|pid| pid.parse::<u32>().ok())
        else {
            continue;
        };
        if owner == std::process::id() || process_is_alive(owner) {
            continue;
        }
        let profile = entry.path();
        let _ = Command::new("pkill")
            .arg("-f")
            .arg(format!("--user-data-dir={}", profile.display()))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = std::fs::remove_dir_all(&profile);
    }
}

fn process_is_alive(pid: u32) -> bool {
    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Kills Chromium when the stream ends, however it ends.
struct Browser(Child);

impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn launch(
    chromium: &Path,
    profile: &Path,
    viewport: (u32, u32),
) -> Result<(Browser, String), String> {
    let mut command = Command::new(chromium);
    command
        .arg("--headless")
        .arg("--remote-debugging-port=0")
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg(format!("--window-size={},{}", viewport.0, viewport.1))
        .args([
            "--no-first-run",
            "--no-default-browser-check",
            "--hide-scrollbars",
            "--mute-audio",
            "--disable-background-networking",
            "--disable-features=Translate,MediaRouter",
        ]);
    if running_as_root() {
        // Chromium refuses to start its sandbox as root, which is common in containers.
        command.arg("--no-sandbox");
    }
    command
        .arg("about:blank")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|err| format!("Couldn't start {}: {err}", chromium.display()))?;
    let stderr = child.stderr.take().expect("stderr is piped");
    let browser = Browser(child);

    let (url_tx, url_rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("preview-browser-log".to_owned())
        .spawn(move || {
            // Keeps reading after the address arrives, so Chromium never blocks on a full pipe.
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if let Some(url) = devtools_url(&line) {
                    let _ = url_tx.send(url.to_owned());
                }
            }
        })
        .map_err(|err| err.to_string())?;
    let url = url_rx
        .recv_timeout(STARTUP_TIMEOUT)
        .map_err(|_| "Chromium didn't start".to_owned())?;
    Ok((browser, url))
}

#[cfg(target_os = "linux")]
fn running_as_root() -> bool {
    std::fs::read_to_string("/proc/self/status").is_ok_and(|status| {
        status
            .lines()
            .find_map(|line| line.strip_prefix("Uid:"))
            .and_then(|uids| uids.split_whitespace().nth(1))
            == Some("0")
    })
}

#[cfg(not(target_os = "linux"))]
fn running_as_root() -> bool {
    false
}

fn run(
    chromium: &Path,
    profile: &Path,
    url: &str,
    mut viewport: (u32, u32),
    rate: Rate,
    control: &mpsc::Receiver<Control>,
    events: &async_channel::Sender<StreamEvent>,
) -> Result<(), String> {
    let (mut browser, devtools) = launch(chromium, profile, viewport)?;
    let mut cdp = Cdp::connect(&devtools)?;

    cdp.call("Target.setDiscoverTargets", json!({"discover": true}), None)?;
    let target = cdp.call("Target.createTarget", json!({"url": "about:blank"}), None)?;
    let target_id = target["targetId"].as_str().unwrap_or_default().to_owned();
    let attached = cdp.call(
        "Target.attachToTarget",
        json!({"targetId": target_id, "flatten": true}),
        None,
    )?;
    let session = attached["sessionId"]
        .as_str()
        .ok_or("Chromium didn't open a session")?
        .to_owned();
    let session = Some(session.as_str());
    for domain in ["Page.enable", "Runtime.enable", "Log.enable"] {
        cdp.call(domain, json!({}), session)?;
    }
    set_viewport(&mut cdp, viewport, session)?;
    // Started on the blank page, so a navigation that moves the page to another process carries
    // the screencast with it instead of racing it.
    set_rate(&mut cdp, rate, session)?;
    cdp.call("Page.navigate", json!({"url": url}), session)?;

    let mut title = String::new();
    let mut page_url = String::new();
    loop {
        loop {
            match control.try_recv() {
                Ok(Control::SetRate(rate)) => set_rate(&mut cdp, rate, session)?,
                Ok(Control::Navigate(url)) => {
                    cdp.send("Page.navigate", json!({"url": url}), session)?;
                }
                Ok(Control::Resize(width, height)) => {
                    viewport = (width, height);
                    set_viewport(&mut cdp, viewport, session)?;
                }
                Ok(Control::Rotate) => {}
                Ok(Control::Input(event)) => {
                    for (method, params) in super::input::commands(&event, viewport) {
                        cdp.send(method, params, session)?;
                    }
                }
                Ok(Control::Stop) | Err(mpsc::TryRecvError::Disconnected) => {
                    let _ = cdp.send("Browser.close", json!({}), None);
                    cdp.close();
                    return Ok(());
                }
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }
        if let Ok(Some(status)) = browser.0.try_wait() {
            return Err(format!("Chromium quit ({status})"));
        }
        let Some(Message::Event {
            method,
            params,
            session: event_session,
        }) = cdp.next_event()?
        else {
            continue;
        };
        match method.as_str() {
            "Page.screencastFrame" => {
                cdp.send(
                    "Page.screencastFrameAck",
                    json!({"sessionId": params["sessionId"]}),
                    event_session.as_deref(),
                )?;
                let Some(data) = params["data"].as_str() else {
                    continue;
                };
                let Some(frame) = base64::engine::general_purpose::STANDARD
                    .decode(data)
                    .ok()
                    .and_then(jpeg::frame_from_jpeg)
                else {
                    continue;
                };
                if events.send_blocking(StreamEvent::Frame(frame)).is_err() {
                    return Ok(());
                }
            }
            "Target.targetInfoChanged" => {
                let info = &params["targetInfo"];
                if info["targetId"].as_str() != Some(target_id.as_str()) {
                    continue;
                }
                if let Some(url) = info["url"].as_str()
                    && url != page_url
                {
                    page_url = url.to_owned();
                    let _ = events.send_blocking(StreamEvent::Url(page_url.clone()));
                }
            }
            // Target info reports the URL as the title until a page sets one, so the title is
            // read from the page once it has loaded.
            "Page.domContentEventFired" | "Page.loadEventFired" => {
                let result = cdp.call(
                    "Runtime.evaluate",
                    json!({"expression": "document.title", "returnByValue": true}),
                    session,
                )?;
                let new_title = result["result"]["value"].as_str().unwrap_or_default();
                if new_title != title {
                    title = new_title.to_owned();
                    let _ = events.send_blocking(StreamEvent::Title(title.clone()));
                }
            }
            "Target.targetDestroyed" | "Target.detachedFromTarget"
                if params["targetId"].as_str() == Some(target_id.as_str()) =>
            {
                return Err("The page closed".to_owned());
            }
            method => {
                if let Some(line) = log_line(method, &params) {
                    let _ = events.send_blocking(StreamEvent::Log(line));
                }
            }
        }
    }
}

fn set_viewport(
    cdp: &mut Cdp,
    (width, height): (u32, u32),
    session: Option<&str>,
) -> Result<(), String> {
    cdp.call(
        "Emulation.setDeviceMetricsOverride",
        json!({
            "width": width.max(1),
            "height": height.max(1),
            "deviceScaleFactor": 0,
            "mobile": false,
        }),
        session,
    )
    .map(|_| ())
}

/// How long to wait before starting a screencast again on a page that was moving between processes.
const DETACHED_RETRY_DELAY: Duration = Duration::from_millis(400);

fn set_rate(cdp: &mut Cdp, rate: Rate, session: Option<&str>) -> Result<(), String> {
    let (max_side, every_nth_frame) = match rate {
        Rate::Full => (FULL_MAX_SIDE, 1),
        Rate::Thumbnail => (THUMBNAIL_MAX_SIDE, THUMBNAIL_EVERY_NTH_FRAME),
        Rate::Paused => {
            cdp.call("Page.stopScreencast", json!({}), session)?;
            return Ok(());
        }
    };
    cdp.call("Page.stopScreencast", json!({}), session)?;
    let params = json!({
        "format": "jpeg",
        "quality": jpeg::QUALITY,
        "maxWidth": max_side,
        "maxHeight": max_side,
        "everyNthFrame": every_nth_frame,
    });
    match cdp.call("Page.startScreencast", params.clone(), session) {
        // A page moving to another process is briefly detached; it answers again moments later.
        Err(err) if err.contains("Not attached") => {
            std::thread::sleep(DETACHED_RETRY_DELAY);
            cdp.call("Page.startScreencast", params, session)
                .map(|_| ())
        }
        result => result.map(|_| ()),
    }
}

/// A line for the preview's log from a console message, uncaught error or browser log entry.
pub(crate) fn log_line(method: &str, params: &Value) -> Option<String> {
    let line = match method {
        "Runtime.consoleAPICalled" => {
            let text = params["args"]
                .as_array()?
                .iter()
                .map(|arg| match &arg["value"] {
                    Value::String(text) => text.clone(),
                    Value::Null => arg["description"]
                        .as_str()
                        .or(arg["type"].as_str())
                        .unwrap_or_default()
                        .to_owned(),
                    value => value.to_string(),
                })
                .collect::<Vec<_>>()
                .join(" ");
            format!(
                "console.{}: {text}",
                params["type"].as_str().unwrap_or("log")
            )
        }
        "Runtime.exceptionThrown" => {
            let details = &params["exceptionDetails"];
            let text = details["exception"]["description"]
                .as_str()
                .or(details["text"].as_str())
                .unwrap_or("Uncaught error");
            format!("error: {text}")
        }
        "Log.entryAdded" => {
            let entry = &params["entry"];
            let mut line = format!(
                "{}: {}",
                entry["level"].as_str().unwrap_or("info"),
                entry["text"].as_str().unwrap_or_default()
            );
            if let Some(url) = entry["url"].as_str().filter(|url| !url.is_empty()) {
                line.push_str(&format!(" ({url})"));
            }
            line
        }
        _ => return None,
    };
    Some(line.chars().take(MAX_LOG_LINE).collect())
}

#[cfg(test)]
#[path = "screencast_tests.rs"]
mod tests;
