//! High performance file indexing for OmniFind.
//! Supports:
//! - Quick Sync: Incremental scan of user folders.
//! - Save Live Cache: Instantly serializes in-memory RAM cache to disk (< 50ms).
//! - Full Rescan: Deep scan of all drives, skipping massive irrelevant system directories.
//! - Safe storage in LocalAppData with portable CWD fallback.
//! - Versioned FST container with backward compatibility.

use fst::{Set, SetBuilder, Streamer};
use jwalk::WalkDir;
use rayon::prelude::*;
use std::collections::HashSet;
use std::fs;
use std::io::{self, Error, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

/// Magic header for OmniFind index: "OMNIFIND" + Version 1 (little-endian u32: 1)
pub const INDEX_MAGIC: &[u8; 8] = b"OMNIFIND";
pub const INDEX_VERSION: u32 = 1;

/// Checks if a drive is active, formatted, and ready (skips empty optical drives or disconnected network shares).
#[cfg(windows)]
pub fn is_ready_drive(letter: char) -> bool {
    let drive_str = format!("{}:\\\0", letter);
    let wide: Vec<u16> = drive_str.encode_utf16().collect();
    unsafe {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetDriveTypeW(lpRootPathName: *const u16) -> u32;
        }
        let dt = GetDriveTypeW(wide.as_ptr());
        // 2: DRIVE_REMOVABLE, 3: DRIVE_FIXED, 6: DRIVE_RAMDISK
        dt == 2 || dt == 3 || dt == 6
    }
}

#[cfg(not(windows))]
pub fn is_ready_drive(_letter: char) -> bool {
    true
}

/// Lowers background worker thread priority so the Windows UI message pump is NEVER starved.
#[cfg(windows)]
pub fn set_background_thread_priority() {
    unsafe {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetCurrentThread() -> *mut std::ffi::c_void;
            fn SetThreadPriority(hThread: *mut std::ffi::c_void, nPriority: i32) -> i32;
        }
        // THREAD_PRIORITY_BELOW_NORMAL (-1) guarantees that GUI thread gets all the CPU it needs
        let _ = SetThreadPriority(GetCurrentThread(), -1);
    }
}

#[cfg(not(windows))]
pub fn set_background_thread_priority() {}

/// Resolves the file path for the persistent index:
/// 1. If `omnifind_index.fst` exists in CWD and is writable, use it (Portable Mode).
/// 2. If `.portable` marker exists and CWD is writable, use local `omnifind_index.fst`.
/// 3. Otherwise use `%LOCALAPPDATA%\OmniFind\omnifind_index.fst`.
pub fn get_index_path() -> PathBuf {
    let local_path = PathBuf::from("omnifind_index.fst");
    if local_path.exists() {
        if fs::OpenOptions::new().write(true).open(&local_path).is_ok() {
            return local_path;
        }
    } else {
        let marker = PathBuf::from(".portable");
        if marker.exists() {
            if fs::OpenOptions::new().write(true).create(true).open(&local_path).is_ok() {
                return local_path;
            }
        }
    }

    if let Some(mut app_data) = dirs::data_local_dir() {
        app_data.push("OmniFind");
        let _ = fs::create_dir_all(&app_data);
        app_data.push("omnifind_index.fst");
        return app_data;
    }

    local_path
}

/// Loads the index from disk into a vector of lowercased paths.
/// Supports both versioned header format (8-byte magic + 4-byte version) and legacy raw FST format.
pub fn load_index_from_disk() -> io::Result<Vec<String>> {
    let path = get_index_path();
    let bytes = fs::read(&path)?;

    let fst_bytes = if bytes.len() >= 12 && (&bytes[0..8] == INDEX_MAGIC || &bytes[0..8] == b"FLASHFND") {
        let version = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        if version != INDEX_VERSION {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!("Unsupported index version: {}", version),
            ));
        }
        &bytes[12..]
    } else if bytes.len() >= 8 && &bytes[0..4] == b"FLSH" {
        // Compatibility for 4-byte header
        &bytes[8..]
    } else {
        // Backward-compatible fallback for legacy unversioned files
        &bytes[..]
    };

    let set = Set::new(fst_bytes).map_err(|e| Error::new(ErrorKind::InvalidData, e))?;
    let mut stream = set.stream();
    let mut cache = Vec::new();
    while let Some(key) = stream.next() {
        if let Ok(s) = std::str::from_utf8(key) {
            cache.push(s.to_string());
        }
    }
    Ok(cache)
}

/// Atomically saves path list to disk wrapped in a versioned FST container.
pub fn save_fst_index(paths: &[String]) -> io::Result<()> {
    debug_assert!(
        paths.windows(2).all(|w| w[0] <= w[1]),
        "save_fst_index requires sorted input"
    );

    let target_path = get_index_path();
    let tmp_path = target_path.with_extension("tmp");

    let mut build = SetBuilder::memory();
    for path in paths {
        if let Err(e) = build.insert(path) {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!("Failed to insert path into FST (paths must be sorted): {}", e),
            ));
        }
    }
    let fst_bytes = build.into_inner().map_err(|e| Error::new(ErrorKind::Other, e))?;

    // Build file with 12-byte header: "OMNIFIND" (8 bytes) + Version (4 bytes)
    let mut file_bytes = Vec::with_capacity(12 + fst_bytes.len());
    file_bytes.extend_from_slice(INDEX_MAGIC);
    file_bytes.extend_from_slice(&INDEX_VERSION.to_le_bytes());
    file_bytes.extend_from_slice(&fst_bytes);

    fs::write(&tmp_path, &file_bytes)?;
    fs::rename(&tmp_path, &target_path)?;
    Ok(())
}

/// Instantly persists the current in-memory RAM cache to disk without scanning (< 50ms).
pub fn persist_live_cache(paths: &[String]) -> io::Result<usize> {
    save_fst_index(paths)?;
    Ok(paths.len())
}

/// System directories to skip during scanning to avoid wasting minutes on 300,000+ internal OS files.
fn is_ignored_dir(path: &Path) -> bool {
    let s = path.to_string_lossy().to_ascii_lowercase();

    // Skip Windows OS directory (C:\Windows)
    if s.starts_with(r"c:\windows") {
        return true;
    }
    // Skip recycle bin & system volume info on any drive
    if s.contains(r"\$recycle.bin") || s.contains(r"\system volume information") {
        return true;
    }
    // Skip temp, compiler intermediate outputs, and recursive junction loops
    if s.contains(r"\appdata\local\temp")
        || s.contains(r"\appdata\local\application data")
        || s.contains(r"\servicing")
        || s.contains(r"\winsxs")
        || s.contains(r"\node_modules")
        || s.contains(r"\.git")
    {
        return true;
    }
    false
}

pub fn run_quick_sync(
    existing_cache: Vec<String>,
    tx_progress: Sender<(f32, String)>,
) -> io::Result<Vec<String>> {
    set_background_thread_priority();

    let start = Instant::now();
    let _ = tx_progress.send((0.1, "Quick Sync: Scanning user folders...".to_string()));

    let user_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from(r"C:\Users"));
    let mut scan_roots = vec![user_dir.clone()];

    // Include common user project roots on C: if they exist
    for extra_c in &["C:\\Projects", "C:\\Data", "C:\\Dev"] {
        let p = PathBuf::from(extra_c);
        if p.exists() && !scan_roots.contains(&p) {
            scan_roots.push(p);
        }
    }

    // Also include any non-C drives (D:\, E:\, etc.) that are ready
    for letter in b'D'..=b'Z' {
        let drive_letter = char::from(letter);
        if is_ready_drive(drive_letter) {
            let root = PathBuf::from(format!("{}:\\", drive_letter));
            if root.exists() {
                scan_roots.push(root);
            }
        }
    }

    let mut current_user_files: HashSet<String> = HashSet::new();

    for root in &scan_roots {
        for entry in WalkDir::new(root)
            .skip_hidden(true)
            .process_read_dir(|_, _, _, children| {
                children.retain(|e| match e {
                    Ok(entry) => !is_ignored_dir(&entry.path()),
                    Err(_) => false,
                });
            })
            .into_iter()
            .flatten()
        {
            if entry.file_type().is_file() {
                if let Some(p_str) = entry.path().to_str() {
                    current_user_files.insert(p_str.to_ascii_lowercase());
                }
            }
        }
    }

    let _ = tx_progress.send((0.6, "Merging changes into index...".to_string()));

    let mut cache_set: HashSet<String> = existing_cache.into_iter().collect();

    // 1. Prune files within all scanned root directories that no longer exist
    let prefixes: Vec<String> = scan_roots
        .iter()
        .map(|r| {
            let mut s = r.to_string_lossy().to_ascii_lowercase();
            if !s.ends_with('\\') {
                s.push('\\');
            }
            s
        })
        .collect();

    cache_set.retain(|p| {
        if prefixes.iter().any(|pref| p.starts_with(pref.as_str())) {
            current_user_files.contains(p)
        } else {
            true
        }
    });

    // 2. Insert all current files
    let mut added_count = 0;
    for file in &current_user_files {
        if cache_set.insert(file.clone()) {
            added_count += 1;
        }
    }

    let mut final_paths: Vec<String> = cache_set.into_iter().collect();
    final_paths.par_sort_unstable();

    let _ = tx_progress.send((0.8, "Saving updated index...".to_string()));
    save_fst_index(&final_paths)?;

    let elapsed = start.elapsed();
    let _ = tx_progress.send((
        1.0,
        format!(
            "Quick Sync done in {:.1}s (+{} new files). Total: {}.",
            elapsed.as_secs_f32(),
            added_count,
            final_paths.len()
        ),
    ));

    Ok(final_paths)
}

pub fn run_full_rescan(tx_progress: Sender<(f32, String)>) -> io::Result<Vec<String>> {
    set_background_thread_priority();

    let start = Instant::now();
    let _ = tx_progress.send((0.05, "Phase 1: Discovering files across drives...".to_string()));

    let mut all_paths: Vec<String> = Vec::new();
    let mut last_progress = Instant::now();

    for letter in b'C'..=b'Z' {
        let drive_letter = char::from(letter);
        if !is_ready_drive(drive_letter) {
            continue;
        }

        let root_str = format!("{}:\\", drive_letter);
        let root_path = PathBuf::from(&root_str);
        if root_path.exists() {
            for dir_entry in WalkDir::new(&root_path)
                .skip_hidden(true)
                .process_read_dir(|_, _, _, children| {
                    children.retain(|e| match e {
                        Ok(entry) => !is_ignored_dir(&entry.path()),
                        Err(_) => false,
                    });
                })
                .into_iter()
                .flatten()
            {
                if dir_entry.file_type().is_file() {
                    if let Some(path_str) = dir_entry.path().to_str() {
                        all_paths.push(path_str.to_ascii_lowercase());

                        // Send live progress update every 150ms and yield so UI thread remains silky smooth
                        if last_progress.elapsed() >= Duration::from_millis(150) {
                            let count = all_paths.len();
                            let prog = (0.05 + (count as f32 / 1_500_000.0).min(0.5)).min(0.55);
                            let _ = tx_progress.send((
                                prog,
                                format!("Scanning drive {}: ({} files indexed)...", drive_letter, count),
                            ));
                            std::thread::yield_now();
                            last_progress = Instant::now();
                        }
                    }
                }
            }
        }
    }

    let total_files = all_paths.len();
    let _ = tx_progress.send((0.6, format!("Phase 2: Sorting {} files...", total_files)));
    std::thread::yield_now();

    all_paths.par_sort_unstable();

    let _ = tx_progress.send((0.8, "Phase 3: Saving index to disk...".to_string()));
    std::thread::yield_now();
    save_fst_index(&all_paths)?;

    let elapsed = start.elapsed();
    let _ = tx_progress.send((
        1.0,
        format!(
            "Done. Full scan finished in {:.1}s. Indexed {} files.",
            elapsed.as_secs_f32(),
            total_files
        ),
    ));

    Ok(all_paths)
}
