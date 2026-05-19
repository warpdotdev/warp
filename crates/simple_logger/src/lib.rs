use std::{
    path::PathBuf,
    sync::{Arc, Weak},
};

use async_channel::Sender;
use async_fs::OpenOptions;
use futures::AsyncWriteExt as _;
use warpui::r#async::executor::{Background, BackgroundTask};

pub mod manager;

const MAX_LOG_FILE_SIZE: u64 = 50 * 1024 * 1024;
const MAX_ROTATED_FILES: usize = 5;
const MAX_LOG_LINE_BYTES: usize = 1024 * 1024;

fn rotated_path(path: &PathBuf, index: usize) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(format!(".old.{}", index));
    PathBuf::from(s)
}

pub(crate) struct LogFileWriter {
    log_tx: Sender<String>,
    _logging_task: BackgroundTask,
}

impl LogFileWriter {
    pub(crate) fn is_closed(&self) -> bool {
        self.log_tx.is_closed()
    }
}

#[derive(Clone)]
pub struct SimpleLogger {
    writer: Arc<LogFileWriter>,
}

impl SimpleLogger {
    pub(crate) fn new(log_path: PathBuf, executor: Arc<Background>) -> Self {
        let (log_tx, log_rx) = async_channel::unbounded::<String>();

        if let Some(directory) = log_path.parent() {
            let _ = std::fs::create_dir_all(directory);
        }

        let logging_task = executor.spawn(async move {
            let mut log_file = match OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&log_path)
                .await
            {
                Ok(log_file) => log_file,
                Err(e) => {
                    log::warn!("Could not open file for logging: {:?}. {:?}", &log_path, e);
                    return;
                }
            };
            let mut bytes_written: u64 = 0;
            loop {
                match log_rx.recv().await {
                    Ok(log_line) => {
                        let line = format!(
                            "{} | {}\n",
                            chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
                            log_line
                        );
                        let line_bytes = line.len() as u64;
                        if bytes_written + line_bytes > MAX_LOG_FILE_SIZE {
                            let _ = log_file.flush().await;
                            drop(log_file);
                            let largest = MAX_ROTATED_FILES.saturating_sub(1);
                            let _ = std::fs::remove_file(rotated_path(&log_path, largest));
                            for i in (0..largest).rev() {
                                let _ =
                                    std::fs::rename(rotated_path(&log_path, i), rotated_path(&log_path, i + 1));
                            }
                            let _ = std::fs::rename(&log_path, rotated_path(&log_path, 0));
                            log_file = match OpenOptions::new()
                                .write(true)
                                .create(true)
                                .truncate(true)
                                .open(&log_path)
                                .await
                            {
                                Ok(f) => f,
                                Err(e) => {
                                    log::warn!(
                                        "Could not reopen log file after rotation: {:?}. {:?}",
                                        &log_path,
                                        e
                                    );
                                    return;
                                }
                            };
                            bytes_written = 0;
                        }
                        if line_bytes > MAX_LOG_LINE_BYTES as u64 {
                            let _ = log_file.write_all(&line.as_bytes()[..MAX_LOG_LINE_BYTES]).await;
                            let _ = log_file.write_all(b"... [truncated]\n").await;
                            let _ = log_file.flush().await;
                            bytes_written += MAX_LOG_LINE_BYTES as u64 + 16;
                        } else {
                            let _ = log_file.write_all(line.as_bytes()).await;
                            let _ = log_file.flush().await;
                            bytes_written += line_bytes;
                        }
                    }
                    Err(e) => {
                        log::warn!("SimpleLogger: channel closed: {e}");
                        break;
                    }
                }
            }

            let _ = log_file.flush().await;
        });

        Self {
            writer: Arc::new(LogFileWriter {
                log_tx,
                _logging_task: logging_task,
            }),
        }
    }

    pub fn log(&self, message: String) {
        let _ = self.writer.log_tx.try_send(message);
    }

    pub fn close(&self) {
        self.writer.log_tx.close();
    }

    pub(crate) fn downgrade(&self) -> Weak<LogFileWriter> {
        Arc::downgrade(&self.writer)
    }
}
