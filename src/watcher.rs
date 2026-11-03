//! Real-time filesystem watcher using `notify`.
//! Monitors user directories (C:\Users) and all data drive roots (D:\, E:\, etc.).
//! Handles creation, deletion, file renames, directory renames, and buffer overflow recovery.

use notify::event::{ModifyKind, RenameMode};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

/// Filter out high-frequency noise directories (temp files, browser caches, compiler build outputs)
fn is_ignored_path(path: &Path) -> bool {
    let s = path.to_string_lossy().to_ascii_lowercase();
    s.contains(r"\appdata\local\temp")
        || s.contains(r"\appdata\local\microsoft")
        || s.contains(r"\appdata\local\google")
        || s.contains(r"\appdata\roaming\code")
        || s.contains(r"\.cargo")
        || s.contains(r"\.rustup")
        || s.contains(r"\node_modules")
        || s.contains(r"\target\debug")
        || s.contains(r"\target\release")
        || s.contains(r"\.git\")
        || s.contains(r"\$recycle.bin")
}

#[derive(Debug, Clone)]
pub enum FsWatchEvent {
    CreatedOrModified(String),
    Deleted(String),
    Renamed {
        from: String,
        to: String,
        is_dir: bool,
    },
    Overflow,
}

pub struct FsWatcher {
    _watcher: RecommendedWatcher,
}

impl FsWatcher {
    pub fn start(tx: Sender<FsWatchEvent>) -> Option<Self> {
        let tx_event = tx.clone();

        let event_handler = move |res: notify::Result<Event>| match res {
            Ok(event) => {
                match event.kind {
                    // File creation or content modification
                    EventKind::Create(_) | EventKind::Modify(ModifyKind::Data(_)) => {
                        for path in event.paths {
                            if is_ignored_path(&path) {
                                continue;
                            }
                            if path.is_file() {
                                if let Some(p_str) = path.to_str() {
                                    let _ = tx_event.send(FsWatchEvent::CreatedOrModified(
                                        p_str.to_ascii_lowercase(),
                                    ));
                                }
                            }
                        }
                    }

                    // Rename events
                    EventKind::Modify(ModifyKind::Name(mode)) => match mode {
                        RenameMode::Both => {
                            if event.paths.len() >= 2 {
                                let from_path = &event.paths[0];
                                let to_path = &event.paths[1];

                                if is_ignored_path(from_path) && is_ignored_path(to_path) {
                                    return;
                                }

                                if let (Some(from_str), Some(to_str)) =
                                    (from_path.to_str(), to_path.to_str())
                                {
                                    let is_dir = to_path.is_dir();
                                    let _ = tx_event.send(FsWatchEvent::Renamed {
                                        from: from_str.to_ascii_lowercase(),
                                        to: to_str.to_ascii_lowercase(),
                                        is_dir,
                                    });
                                }
                            }
                        }
                        RenameMode::From => {
                            for path in event.paths {
                                if is_ignored_path(&path) {
                                    continue;
                                }
                                if let Some(p_str) = path.to_str() {
                                    let _ = tx_event.send(FsWatchEvent::Deleted(
                                        p_str.to_ascii_lowercase(),
                                    ));
                                }
                            }
                        }
                        RenameMode::To => {
                            for path in event.paths {
                                if is_ignored_path(&path) {
                                    continue;
                                }
                                if path.is_file() {
                                    if let Some(p_str) = path.to_str() {
                                        let _ = tx_event.send(FsWatchEvent::CreatedOrModified(
                                            p_str.to_ascii_lowercase(),
                                        ));
                                    }
                                }
                            }
                        }
                        _ => {
                            // Any other rename mode: check paths
                            if event.paths.len() == 2 {
                                let from_path = &event.paths[0];
                                let to_path = &event.paths[1];

                                if is_ignored_path(from_path) && is_ignored_path(to_path) {
                                    return;
                                }

                                if let (Some(from_str), Some(to_str)) =
                                    (from_path.to_str(), to_path.to_str())
                                {
                                    let is_dir = to_path.is_dir();
                                    let _ = tx_event.send(FsWatchEvent::Renamed {
                                        from: from_str.to_ascii_lowercase(),
                                        to: to_str.to_ascii_lowercase(),
                                        is_dir,
                                    });
                                }
                            }
                        }
                    },

                    // Deletion
                    EventKind::Remove(_) => {
                        for path in event.paths {
                            if is_ignored_path(&path) {
                                continue;
                            }
                            if let Some(p_str) = path.to_str() {
                                let _ = tx_event.send(FsWatchEvent::Deleted(
                                    p_str.to_ascii_lowercase(),
                                ));
                            }
                        }
                    }

                    _ => {}
                }
            }
            Err(_) => {
                // Buffer overflow or watcher error: notify app to perform quick recovery sync
                let _ = tx_event.send(FsWatchEvent::Overflow);
            }
        };

        let mut watcher = RecommendedWatcher::new(event_handler, Config::default()).ok()?;

        // 1. Watch user directory (C:\Users)
        let user_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from(r"C:\Users"));
        if user_dir.exists() {
            let _ = watcher.watch(&user_dir, RecursiveMode::Recursive);
        }

        // 2. Watch all additional data drives (D:\, E:\, etc.)
        for letter in b'D'..=b'Z' {
            let drive_path = PathBuf::from(format!("{}:\\", char::from(letter)));
            if drive_path.exists() {
                let _ = watcher.watch(&drive_path, RecursiveMode::Recursive);
            }
        }

        Some(Self { _watcher: watcher })
    }
}
