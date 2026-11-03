#![windows_subsystem = "windows"]

//! OmniFind - High Performance File Search Engine
//!
//! Architecture:
//! 1. Indexing: Fast parallel directory scanning with Quick Sync (< 2s) & Full Rescan.
//! 2. Live Watching: Real-time filesystem watcher using `notify` for instantaneous file updates across drives.
//! 3. Storage: Compresses paths into a versioned FST container on disk with safe AppData/Local resolution.
//! 4. Search: RAM-resident parallel scanner using Rayon with live existence verification and atomic cancellation.
//!    Supports advanced Google-like dorks: ext:, name:, path:, drive:, quotes, -negation, OR.
//!    Supports multi-file selection, Windows Clipboard Copy / Cut (CF_HDROP), and native Explorer reveal.
//!    Modern Windows 11 Light Dashboard UI Style.

mod clipboard;
mod filter;
mod indexer;
mod style;
mod watcher;

use filter::SearchFilter;
use watcher::{FsWatchEvent, FsWatcher};

use iced::futures::SinkExt;

use iced::widget::{
    Column, Space, button, checkbox, column, container, progress_bar, row, scrollable, text,
    text_input,
};
use iced::window::icon;
use iced::{Alignment, Application, Command, Element, Length, Settings, Size, Theme, executor};
use rayon::prelude::*;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, RwLock, mpsc};
use std::thread;
use std::time::{Duration, Instant};

/// Application Version
const CURRENT_VERSION: &str = "1.0.0";

/// Native Windows Icons UI Kit font (Segoe MDL2 Assets, built into Windows 10 & 11)
pub const ICON_FONT: iced::Font = iced::Font::with_name("Segoe MDL2 Assets");

pub mod icons {
    // Official Windows MDL2 Assets UI Kit Glyphs
    pub const SEARCH: char = '\u{E721}';      // Magnifying glass
    pub const DASHBOARD: char = '\u{E80F}';   // Home / Dashboard
    pub const RULES: char = '\u{E9D2}';       // View / Rules
    pub const DRIVES: char = '\u{EDA2}';      // Hard Drive / Storage
    pub const REVEAL: char = '\u{E8B7}';      // Folder Open
    pub const SAVE: char = '\u{E74E}';        // Save Disk
    pub const SYNC: char = '\u{E895}';        // Sync
    pub const RESCAN: char = '\u{E72C}';      // Refresh / Rescan
    pub const COPY: char = '\u{E8C8}';        // Copy
    pub const CUT: char = '\u{E8C6}';         // Cut
    pub const LIGHTNING: char = '\u{E945}';   // Lightning / Speed
    pub const CLEAR: char = '\u{E711}';       // Chrome Close
    pub const CHECK: char = '\u{E73E}';       // Checkmark
    pub const FOLDER: char = '\u{E838}';      // Folder
}

fn win_icon(codepoint: char, size: u16) -> iced::widget::Text<'static> {
    text(codepoint.to_string())
        .font(ICON_FONT)
        .size(size)
}

pub fn main() -> iced::Result {
    let icon = load_icon().ok();

    // Window configuration
    let settings = Settings {
        window: iced::window::Settings {
            icon,
            min_size: Some(Size::new(960.0, 680.0)),
            size: Size::new(1200.0, 800.0), // Spacious start size
            ..iced::window::Settings::default()
        },
        ..Settings::default()
    };

    OmniFind::run(settings)
}

/// Helper: Loads the application icon from the embedded binary data.
fn load_icon() -> Result<icon::Icon, Box<dyn std::error::Error>> {
    let bytes = include_bytes!("icon.png");
    let img = image::load_from_memory(bytes)?.to_rgba8();
    let (width, height) = img.dimensions();
    let rgba = img.into_raw();
    Ok(icon::from_rgba(rgba, width, height)?)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Overview,
    DorksGuide,
    Drives,
}

#[derive(Debug, Clone)]
enum Message {
    SelectTab(Tab),
    SearchChanged(String),
    FileClicked(String),
    OpenFolderClicked(String),
    QuickFilterClicked(String),
    ToggleFileSelection(String),
    SelectAll,
    DeselectAll,
    CopySelected,
    CutSelected,
    CopySingleFile(String),
    CutSingleFile(String),
    ClipboardFinished {
        success: bool,
        count: usize,
        is_cut: bool,
        label: String,
    },
    StartSaveIndex,
    StartQuickSync,
    StartFullRescan,
    IndexProgress(f32, String),
    SearchFinished(Vec<String>, u128, usize),
    CacheLoaded(Arc<RwLock<Vec<String>>>),
    FsBatchReceived(Vec<FsWatchEvent>),
}

/// Helper struct for ranking search results
#[derive(Debug, Eq, PartialEq)]
struct ScoredResult {
    path: String,
    score: i32,
    len: usize,
}

// Implement Ord for Min-Heap logic (Priority Queue)
impl Ord for ScoredResult {
    fn cmp(&self, other: &Self) -> Ordering {
        // Lower score is worse (higher in Min-Heap, so it gets popped first).
        // Longer length is worse when scores are tied.
        other
            .score
            .cmp(&self.score)
            .then_with(|| self.len.cmp(&other.len))
    }
}

impl PartialOrd for ScoredResult {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

// Type aliases for cleaner thread communication
type IndexReceiver = Arc<Mutex<mpsc::Receiver<(f32, String)>>>;
type SearchReceiver = Arc<Mutex<mpsc::Receiver<(Vec<String>, u128, usize)>>>;
type CacheReceiver = Arc<Mutex<mpsc::Receiver<Arc<RwLock<Vec<String>>>>>>;

struct OmniFind {
    // UI Navigation State
    current_tab: Tab,

    // UI Search State
    query: String,
    parsed_filter: SearchFilter,
    results: Vec<String>,
    selected_files: HashSet<String>,
    progress: f32,
    status_msg: String,
    search_time_msg: String,
    last_search_micros: Option<u128>,
    is_indexing: bool,
    is_searching: bool,
    active_drives: usize,

    // Core Data
    search_cache: Option<Arc<RwLock<Vec<String>>>>, // The RAM-resident file list with thread-safe read/write

    // Concurrency Channels
    index_receiver: Option<IndexReceiver>,
    search_sender: mpsc::Sender<(Vec<String>, u128, usize)>,
    search_receiver: SearchReceiver,
    cache_loader_sender: mpsc::Sender<Arc<RwLock<Vec<String>>>>,
    cache_loader_receiver: CacheReceiver,
    fs_watch_receiver: Arc<Mutex<mpsc::Receiver<FsWatchEvent>>>,
    _fs_watcher: Option<FsWatcher>,

    // Search Generation & Cancellation
    search_generation: Arc<AtomicUsize>,
}

impl Application for OmniFind {
    type Message = Message;
    type Theme = Theme;
    type Executor = executor::Default;
    type Flags = ();

    fn new(_flags: ()) -> (Self, Command<Message>) {
        let (tx_search, rx_search) = mpsc::channel();
        let (tx_cache, rx_cache) = mpsc::channel();
        let (tx_watch, rx_watch) = mpsc::channel();

        // Start Real-Time Filesystem Watcher across drives
        let watcher_handle = FsWatcher::start(tx_watch);

        // THREAD 1: Load Disk Index into RAM Cache on startup (using safe versioned index loader)
        let tx_cache_clone = tx_cache.clone();
        thread::spawn(move || {
            match indexer::load_index_from_disk() {
                Ok(cache) => {
                    let _ = tx_cache_clone.send(Arc::new(RwLock::new(cache)));
                }
                Err(e) => {
                    eprintln!("Index load notice: {}", e);
                    let _ = tx_cache_clone.send(Arc::new(RwLock::new(Vec::new())));
                }
            }
        });

        let active_drives = count_active_drives();

        (
            Self {
                current_tab: Tab::Overview,
                query: String::new(),
                parsed_filter: SearchFilter::default(),
                results: Vec::new(),
                selected_files: HashSet::new(),
                search_cache: None,
                is_indexing: false,
                is_searching: false,
                active_drives,
                progress: 0.0,
                status_msg: "Initializing...".to_string(),
                search_time_msg: String::new(),
                last_search_micros: None,

                index_receiver: None,
                search_sender: tx_search,
                search_receiver: Arc::new(Mutex::new(rx_search)),
                cache_loader_sender: tx_cache,
                cache_loader_receiver: Arc::new(Mutex::new(rx_cache)),
                fs_watch_receiver: Arc::new(Mutex::new(rx_watch)),
                _fs_watcher: watcher_handle,
                search_generation: Arc::new(AtomicUsize::new(0)),
            },
            Command::none(),
        )
    }

    fn title(&self) -> String {
        String::from("OmniFind")
    }

    fn update(&mut self, message: Message) -> Command<Message> {
        match message {
            Message::SelectTab(tab) => {
                self.current_tab = tab;
                Command::none()
            }

            Message::QuickFilterClicked(snippet) => {
                let trimmed = self.query.trim();
                let new_query = if trimmed.is_empty() {
                    snippet
                } else if trimmed.ends_with(' ') {
                    format!("{}{}", trimmed, snippet)
                } else {
                    format!("{} {}", trimmed, snippet)
                };
                self.update(Message::SearchChanged(new_query))
            }

            // Real-time Filesystem Batch Event Handler (drains all pending events in batch)
            Message::FsBatchReceived(events) => {
                let mut needs_refresh = false;
                let mut has_overflow = false;

                if let Some(cache_arc) = &self.search_cache {
                    if let Ok(mut cache) = cache_arc.write() {
                        for event in events {
                            match event {
                                FsWatchEvent::CreatedOrModified(path) => {
                                    if !cache.contains(&path) {
                                        cache.push(path);
                                        needs_refresh = true;
                                    }
                                }
                                FsWatchEvent::Deleted(path) => {
                                    let prefix = format!("{}\\", path.trim_end_matches('\\'));
                                    cache.retain(|p| p != &path && !p.starts_with(&prefix));
                                    needs_refresh = true;
                                }
                                FsWatchEvent::Renamed { from, to, is_dir } => {
                                    if is_dir {
                                        let from_prefix = format!("{}\\", from.trim_end_matches('\\'));
                                        let to_prefix = format!("{}\\", to.trim_end_matches('\\'));
                                        for p in cache.iter_mut() {
                                            if p.starts_with(&from_prefix) {
                                                *p = format!("{}{}", to_prefix, &p[from_prefix.len()..]);
                                            }
                                        }
                                    } else {
                                        if let Some(pos) = cache.iter().position(|p| p == &from) {
                                            cache[pos] = to;
                                        } else if !cache.contains(&to) {
                                            cache.push(to);
                                        }
                                    }
                                    needs_refresh = true;
                                }
                                FsWatchEvent::Overflow => {
                                    has_overflow = true;
                                }
                            }
                        }
                    }
                }

                if has_overflow {
                    self.status_msg = "Notice: Watcher buffer overflow detected. Triggering quick sync...".to_string();
                    return self.update(Message::StartQuickSync);
                }

                if needs_refresh && !self.query.trim().is_empty() {
                    return self.update(Message::SearchChanged(self.query.clone()));
                }
                Command::none()
            }

            // Selection & Clipboard Handlers
            Message::ToggleFileSelection(path) => {
                if self.selected_files.contains(&path) {
                    self.selected_files.remove(&path);
                } else {
                    self.selected_files.insert(path);
                }
                Command::none()
            }

            Message::SelectAll => {
                for p in &self.results {
                    self.selected_files.insert(p.clone());
                }
                Command::none()
            }

            Message::DeselectAll => {
                self.selected_files.clear();
                Command::none()
            }

            Message::CopySelected => {
                let paths: Vec<String> = self.selected_files.iter().cloned().collect();
                if !paths.is_empty() {
                    let count = paths.len();
                    self.status_msg = format!("Copying {} {} to clipboard...", count, if count == 1 { "file" } else { "files" });
                    Command::perform(
                        async move {
                            let ok = clipboard::copy_or_cut_files(&paths, false);
                            (ok, count, false, String::new())
                        },
                        |(ok, count, is_cut, label)| Message::ClipboardFinished { success: ok, count, is_cut, label },
                    )
                } else {
                    Command::none()
                }
            }

            Message::CutSelected => {
                let paths: Vec<String> = self.selected_files.iter().cloned().collect();
                if !paths.is_empty() {
                    let count = paths.len();
                    self.status_msg = format!("Cutting {} {} to clipboard...", count, if count == 1 { "file" } else { "files" });
                    Command::perform(
                        async move {
                            let ok = clipboard::copy_or_cut_files(&paths, true);
                            (ok, count, true, String::new())
                        },
                        |(ok, count, is_cut, label)| Message::ClipboardFinished { success: ok, count, is_cut, label },
                    )
                } else {
                    Command::none()
                }
            }

            Message::CopySingleFile(path) => {
                let fname = path.rfind('\\').map(|i| &path[i + 1..]).unwrap_or(&path).to_string();
                self.status_msg = format!("Copying to clipboard: {}", fname);
                let label = fname.clone();
                Command::perform(
                    async move {
                        let ok = clipboard::copy_or_cut_files(&[path], false);
                        (ok, 1, false, label)
                    },
                    |(ok, count, is_cut, label)| Message::ClipboardFinished { success: ok, count, is_cut, label },
                )
            }

            Message::CutSingleFile(path) => {
                let fname = path.rfind('\\').map(|i| &path[i + 1..]).unwrap_or(&path).to_string();
                self.status_msg = format!("Cutting to clipboard: {}", fname);
                let label = fname.clone();
                Command::perform(
                    async move {
                        let ok = clipboard::copy_or_cut_files(&[path], true);
                        (ok, 1, true, label)
                    },
                    |(ok, count, is_cut, label)| Message::ClipboardFinished { success: ok, count, is_cut, label },
                )
            }

            Message::ClipboardFinished { success, count, is_cut, label } => {
                if success {
                    if !label.is_empty() {
                        let action = if is_cut { "Cut to clipboard" } else { "Copied to clipboard" };
                        self.status_msg = format!("{}: {}", action, label);
                    } else {
                        let action = if is_cut { "Cut" } else { "Copied" };
                        let move_or_paste = if is_cut { "move" } else { "paste" };
                        self.status_msg = format!(
                            "{} {} {} to clipboard (ready to {} in Explorer)",
                            action,
                            count,
                            if count == 1 { "file" } else { "files" },
                            move_or_paste
                        );
                    }
                } else {
                    self.status_msg = "Clipboard is locked by another application. Please try again.".to_string();
                }
                Command::none()
            }


            // Index loading complete - Ready to search
            Message::CacheLoaded(cache) => {
                let count = cache.read().map(|c| c.len()).unwrap_or(0);
                self.search_cache = Some(cache);
                self.is_indexing = false;
                if count == 0 {
                    self.status_msg = "No index found — run Quick Sync or Full Rescan to build one.".to_string();
                } else {
                    self.status_msg = format!("Ready. {} files in RAM (live watching active).", count);
                }
                Command::none()
            }

            // Core Search Logic with Atomic Cancellation & Live Validation
            Message::SearchChanged(new_query) => {
                self.query = new_query;
                let current_id = self.search_generation.fetch_add(1, AtomicOrdering::SeqCst) + 1;

                let filter = SearchFilter::parse(&self.query);
                self.parsed_filter = filter.clone();

                if filter.is_empty() {
                    self.results.clear();
                    self.selected_files.clear();
                    self.search_time_msg.clear();
                    self.last_search_micros = None;
                    self.is_searching = false;
                    return Command::none();
                }

                if let Some(cache_arc) = &self.search_cache {
                    let cache = cache_arc.clone();
                    let tx = self.search_sender.clone();
                    let gen_check = self.search_generation.clone();
                    self.is_searching = true;

                    thread::spawn(move || {
                        let start = Instant::now();

                        // Immediate pre-check: if a newer keystroke has already arrived, abort
                        if gen_check.load(AtomicOrdering::Relaxed) != current_id {
                            return;
                        }

                        let top_docs: Vec<String> = if let Ok(cache_guard) = cache.read() {
                            cache_guard
                                .par_iter()
                                .fold(
                                    || BinaryHeap::with_capacity(51),
                                    |mut heap: BinaryHeap<ScoredResult>, path| {
                                        // Periodic cancellation check across threads
                                        if gen_check.load(AtomicOrdering::Relaxed) != current_id {
                                            return heap;
                                        }

                                        if let Some(score) = filter.matches_and_score(path) {
                                            heap.push(ScoredResult {
                                                path: path.clone(),
                                                score,
                                                len: path.len(),
                                            });
                                            if heap.len() > 50 {
                                                heap.pop();
                                            }
                                        }
                                        heap
                                    },
                                )
                                .reduce(BinaryHeap::new, |mut a, b| {
                                    for item in b {
                                        a.push(item);
                                        if a.len() > 50 {
                                            a.pop();
                                        }
                                    }
                                    a
                                })
                                .into_sorted_vec()
                                .into_iter()
                                .rev()
                                .map(|r| r.path)
                                .collect()
                        } else {
                            Vec::new()
                        };

                        // Final check before sending results
                        if gen_check.load(AtomicOrdering::Relaxed) == current_id {
                            let _ = tx.send((top_docs, start.elapsed().as_micros(), current_id));
                        }
                    });
                }
                Command::none()
            }

            Message::SearchFinished(results, micros, id) => {
                // Ensure we only show results for the current keystroke
                if id == self.search_generation.load(AtomicOrdering::Relaxed) {
                    self.is_searching = false;
                    self.last_search_micros = Some(micros);

                    // LIVE VALIDATION: Discard any stale or deleted paths so user NEVER sees old paths
                    let mut valid_results = Vec::new();
                    let mut stale_paths = Vec::new();

                    for p in results {
                        if Path::new(&p).exists() {
                            valid_results.push(p);
                        } else {
                            stale_paths.push(p);
                        }
                    }

                    // O(N) Pruning of stale paths from RAM cache using HashSet lookup
                    if !stale_paths.is_empty() {
                        let stale_set: HashSet<&str> = stale_paths.iter().map(|s| s.as_str()).collect();
                        if let Some(cache_arc) = &self.search_cache {
                            if let Ok(mut cache) = cache_arc.write() {
                                cache.retain(|p| !stale_set.contains(p.as_str()));
                            }
                        }
                    }

                    let count = valid_results.len();
                    self.results = valid_results;
                    self.selected_files.retain(|p| self.results.contains(p));

                    let time_str = if micros < 1000 {
                        format!("{}µs", micros)
                    } else {
                        format!("{:.2}ms", micros as f64 / 1000.0)
                    };

                    if count == 0 {
                        self.search_time_msg = format!("No matching files ({})", time_str);
                    } else if count >= 50 {
                        self.search_time_msg = format!("Found 50+ files in {}", time_str);
                    } else {
                        self.search_time_msg = format!(
                            "Found {} {} in {}",
                            count,
                            if count == 1 { "file" } else { "files" },
                            time_str
                        );
                    }
                }
                Command::none()
            }

            // Open the file with default system application
            Message::FileClicked(path) => {
                let _ = open::that(path);
                Command::none()
            }

            // Open containing folder in Windows Explorer with file selected and scrolled to view
            Message::OpenFolderClicked(path) => {
                thread::spawn(move || {
                    reveal_in_explorer(&path);
                });
                Command::none()
            }

            // Save Index: Instantly writes current live RAM cache to disk (< 50ms)
            Message::StartSaveIndex => {
                if self.is_indexing {
                    return Command::none();
                }
                if let Some(cache_arc) = self.search_cache.clone() {
                    self.is_indexing = true;
                    self.progress = 0.0;
                    self.status_msg = "Saving live cache to disk...".to_string();

                    let (tx_prog, rx_prog) = mpsc::channel();
                    self.index_receiver = Some(Arc::new(Mutex::new(rx_prog)));

                    thread::spawn(move || {
                        let tx_prog_err = tx_prog.clone();
                        let _ = tx_prog.send((0.2, "Sorting live cache...".to_string()));
                        let mut cache = cache_arc.read().map(|c| c.clone()).unwrap_or_default();
                        cache.par_sort_unstable();
                        let count = cache.len();
                        let _ = tx_prog.send((0.6, format!("Writing {} files to index...", count)));
                        match indexer::persist_live_cache(&cache) {
                            Ok(_) => {
                                let _ = tx_prog.send((1.0, format!("Index saved to disk ({} files in sync).", count)));
                            }
                            Err(e) => {
                                eprintln!("Save index error: {}", e);
                                let _ = tx_prog_err.send((1.0, format!("Failed to save index: {}", e)));
                            }
                        }
                    });
                }
                Command::none()
            }

            // Quick Sync: Incremental scan of user folders in 1-2 seconds
            Message::StartQuickSync => {
                if self.is_indexing {
                    return Command::none();
                }
                self.is_indexing = true;
                self.progress = 0.0;
                self.status_msg = "Starting Quick Sync...".to_string();

                let (tx_prog, rx_prog) = mpsc::channel();
                self.index_receiver = Some(Arc::new(Mutex::new(rx_prog)));

                let cache_arc = self.search_cache.clone();
                let tx_cache = self.cache_loader_sender.clone();
                thread::spawn(move || {
                    let existing = cache_arc
                        .and_then(|c| c.read().ok().map(|g| g.clone()))
                        .unwrap_or_default();
                    let tx_prog_err = tx_prog.clone();
                    match indexer::run_quick_sync(existing, tx_prog) {
                        Ok(new_paths) => {
                            let _ = tx_cache.send(Arc::new(RwLock::new(new_paths)));
                        }
                        Err(e) => {
                            eprintln!("Quick sync error: {}", e);
                            let _ = tx_prog_err.send((1.0, format!("Quick Sync failed: {}", e)));
                        }
                    }
                });
                Command::none()
            }

            // Full Rescan: Deep scan across all drives, skipping internal system folders
            Message::StartFullRescan => {
                if self.is_indexing {
                    return Command::none();
                }
                self.is_indexing = true;
                self.progress = 0.0;
                self.status_msg = "Starting Full System Rescan...".to_string();

                let (tx_prog, rx_prog) = mpsc::channel();
                self.index_receiver = Some(Arc::new(Mutex::new(rx_prog)));

                let tx_cache = self.cache_loader_sender.clone();
                thread::spawn(move || {
                    let tx_prog_err = tx_prog.clone();
                    match indexer::run_full_rescan(tx_prog) {
                        Ok(new_paths) => {
                            let _ = tx_cache.send(Arc::new(RwLock::new(new_paths)));
                        }
                        Err(e) => {
                            eprintln!("Full rescan error: {}", e);
                            let _ = tx_prog_err.send((1.0, format!("Full rescan failed: {}", e)));
                        }
                    }
                });
                Command::none()
            }

            Message::IndexProgress(prog, msg) => {
                self.progress = prog;
                self.status_msg = msg;
                if prog >= 1.0 {
                    self.is_indexing = false;
                }
                Command::none()
            }
        }
    }

    // Subscription Management: Event-driven channels that ONLY emit messages when real data arrives
    // Eliminates all dummy Tick messages, saving CPU and preventing UI freezes during heavy I/O
    fn subscription(&self) -> iced::Subscription<Message> {
        let index_sub = if self.is_indexing {
            if let Some(rx_arc) = &self.index_receiver {
                let rx_clone = rx_arc.clone();
                iced::subscription::channel(
                    "index_receiver_channel",
                    20,
                    |mut output| async move {
                        loop {
                            let maybe_msg = rx_clone.try_lock().ok().and_then(|rx| rx.try_recv().ok());
                            if let Some((prog, msg)) = maybe_msg {
                                let _ = output.send(Message::IndexProgress(prog, msg)).await;
                            }
                            tokio::time::sleep(Duration::from_millis(60)).await;
                        }
                    },
                )
            } else {
                iced::Subscription::none()
            }
        } else {
            iced::Subscription::none()
        };

        // Real-Time Filesystem Watcher: Batch drains events and only emits when events exist
        let rx_watch = self.fs_watch_receiver.clone();
        let watch_sub = iced::subscription::channel(
            "fs_watch_channel",
            50,
            |mut output| async move {
                loop {
                    let batch = rx_watch.try_lock().ok().map(|rx| {
                        let mut b = Vec::new();
                        while let Ok(event) = rx.try_recv() {
                            b.push(event);
                        }
                        b
                    }).unwrap_or_default();

                    if !batch.is_empty() {
                        let _ = output.send(Message::FsBatchReceived(batch)).await;
                    }
                    tokio::time::sleep(Duration::from_millis(80)).await;
                }
            },
        );

        // Search Polling: Only runs when a search is actively in-flight
        let search_sub = if self.is_searching {
            let rx_clone = self.search_receiver.clone();
            iced::subscription::channel(
                "search_channel",
                10,
                |mut output| async move {
                    loop {
                        let maybe_res = rx_clone.try_lock().ok().and_then(|rx| rx.try_recv().ok());
                        if let Some((res, time, id)) = maybe_res {
                            let _ = output.send(Message::SearchFinished(res, time, id)).await;
                        }
                        tokio::time::sleep(Duration::from_millis(16)).await;
                    }
                },
            )
        } else {
            iced::Subscription::none()
        };

        // Cache Loader Polling: Only emits when cache is loaded
        let rx_cache = self.cache_loader_receiver.clone();
        let cache_sub = iced::subscription::channel(
            "cache_loader_channel",
            10,
            |mut output| async move {
                loop {
                    let maybe_cache = rx_cache.try_lock().ok().and_then(|rx| rx.try_recv().ok());
                    if let Some(cache) = maybe_cache {
                        let _ = output.send(Message::CacheLoaded(cache)).await;
                    }
                    tokio::time::sleep(Duration::from_millis(80)).await;
                }
            },
        );

        iced::Subscription::batch(vec![index_sub, watch_sub, search_sub, cache_sub])
    }

    fn view(&self) -> Element<'_, Message> {
        let file_count = self
            .search_cache
            .as_ref()
            .and_then(|c| c.read().ok())
            .map(|c| c.len())
            .unwrap_or(0);

        let latency_str = if let Some(micros) = self.last_search_micros {
            if micros < 1000 {
                format!("{}µs", micros)
            } else {
                format!("{:.1}ms", micros as f64 / 1000.0)
            }
        } else {
            "—".to_string()
        };

        // ==========================================
        // 1. LEFT SIDEBAR (Matching uploaded UI screenshot)
        // ==========================================
        let logo_row = row![
            win_icon(icons::LIGHTNING, 20).style(iced::theme::Text::Color(style::COLOR_PRIMARY)),
            text("OmniFind").size(18),
        ]
        .spacing(8)
        .align_items(Alignment::Center);

        let nav_overview = button(
            row![
                win_icon(icons::DASHBOARD, 14),
                text("Overview Dashboard").size(13),
            ]
            .spacing(10)
            .align_items(Alignment::Center),
        )
        .on_press(Message::SelectTab(Tab::Overview))
        .style(iced::theme::Button::Custom(Box::new(style::NavItemBtn {
            is_active: self.current_tab == Tab::Overview,
        })))
        .padding([8, 12])
        .width(Length::Fill);

        let nav_dorks = button(
            row![
                win_icon(icons::RULES, 14),
                text("Dorks & Rules").size(13),
            ]
            .spacing(10)
            .align_items(Alignment::Center),
        )
        .on_press(Message::SelectTab(Tab::DorksGuide))
        .style(iced::theme::Button::Custom(Box::new(style::NavItemBtn {
            is_active: self.current_tab == Tab::DorksGuide,
        })))
        .padding([8, 12])
        .width(Length::Fill);

        let nav_drives = button(
            row![
                win_icon(icons::DRIVES, 14),
                text("Drives & Storage").size(13),
            ]
            .spacing(10)
            .align_items(Alignment::Center),
        )
        .on_press(Message::SelectTab(Tab::Drives))
        .style(iced::theme::Button::Custom(Box::new(style::NavItemBtn {
            is_active: self.current_tab == Tab::Drives,
        })))
        .padding([8, 12])
        .width(Length::Fill);

        let nav_col = column![nav_overview, nav_dorks, nav_drives].spacing(4);

        // Sidebar Info Card (Matching the blue gradient card in the screenshot)
        let sidebar_card = container(
            column![
                row![
                    win_icon(icons::LIGHTNING, 12).style(iced::theme::Text::Color(style::COLOR_PRIMARY)),
                    text("RAM-Resident Search").size(12),
                ]
                .spacing(6)
                .align_items(Alignment::Center),
                text("Live index active across drives. Instant keyword & dork search.")
                    .size(11)
                    .style(iced::theme::Text::Color(style::COLOR_TEXT_MUTED)),
                button(text("Quick Dorks Guide").size(11))
                    .on_press(Message::SelectTab(Tab::DorksGuide))
                    .style(iced::theme::Button::Custom(Box::new(style::SecondaryPillBtn)))
                    .padding([4, 8])
                    .width(Length::Fill),
            ]
            .spacing(8),
        )
        .padding(12)
        .style(iced::theme::Container::Custom(Box::new(style::CardStyle)));

        let sidebar_footer = column![
            row![
                text("•").size(12).style(iced::theme::Text::Color(iced::Color::from_rgb(0.2, 0.75, 0.4))),
                text("Live Watcher Active").size(11).style(iced::theme::Text::Color(style::COLOR_TEXT_MUTED)),
            ]
            .spacing(6)
            .align_items(Alignment::Center),
            text("100% Local Processing").size(10).style(iced::theme::Text::Color(style::COLOR_TEXT_MUTED)),
        ]
        .spacing(4);

        let sidebar_content = column![
            logo_row,
            Space::with_height(Length::Fixed(16.0)),
            nav_col,
            Space::with_height(Length::Fill),
            sidebar_card,
            Space::with_height(Length::Fixed(12.0)),
            sidebar_footer,
        ]
        .spacing(8)
        .padding(16);

        let sidebar = container(sidebar_content)
            .width(Length::Fixed(230.0))
            .height(Length::Fill)
            .style(iced::theme::Container::Custom(Box::new(style::SidebarStyle)));

        // ==========================================
        // 2. MAIN DASHBOARD CONTENT (Matching uploaded UI screenshot)
        // ==========================================

        // Header Title + Top Action Button
        let header_left = column![
            text("Overview Dashboard").size(24),
            row![
                text("•").size(12).style(iced::theme::Text::Color(style::COLOR_PRIMARY)),
                text("Live RAM index active — files are tracked in real-time").size(12).style(iced::theme::Text::Color(style::COLOR_TEXT_MUTED)),
            ]
            .spacing(6)
            .align_items(Alignment::Center),
        ]
        .spacing(4);

        let save_index_btn = button(
            row![
                win_icon(icons::SAVE, 12),
                text("Save Index").size(12),
            ]
            .spacing(6)
            .align_items(Alignment::Center),
        )
        .on_press(Message::StartSaveIndex)
        .style(iced::theme::Button::Custom(Box::new(style::PrimaryPillBtn)))
        .padding([8, 16]);

        let quick_sync_btn = button(
            row![
                win_icon(icons::SYNC, 12),
                text("Quick Sync").size(12),
            ]
            .spacing(6)
            .align_items(Alignment::Center),
        )
        .on_press(Message::StartQuickSync)
        .style(iced::theme::Button::Custom(Box::new(style::SecondaryPillBtn)))
        .padding([8, 14]);

        let full_rescan_btn = button(
            row![
                win_icon(icons::RESCAN, 12),
                text("Full Rescan").size(12),
            ]
            .spacing(6)
            .align_items(Alignment::Center),
        )
        .on_press(Message::StartFullRescan)
        .style(iced::theme::Button::Custom(Box::new(style::SecondaryPillBtn)))
        .padding([8, 14]);

        let header_actions = row![save_index_btn, quick_sync_btn, full_rescan_btn].spacing(8);

        let main_header = row![
            header_left,
            Space::with_width(Length::Fill),
            header_actions,
        ]
        .align_items(Alignment::Center);

        // 3 METRIC CARDS (Identical to top cards in screenshot: "0 Files Organized", "3 Active Routes", "1 Monitored Folders")
        let stat_card_1 = container(
            column![
                text(format_number(file_count)).size(26),
                text("Files in RAM").size(12).style(iced::theme::Text::Color(style::COLOR_TEXT_MUTED)),
            ]
            .spacing(2),
        )
        .padding([14, 18])
        .width(Length::FillPortion(1))
        .style(iced::theme::Container::Custom(Box::new(style::CardStyle)));

        let stat_card_2 = container(
            column![
                text(latency_str).size(26),
                text("Search Latency").size(12).style(iced::theme::Text::Color(style::COLOR_TEXT_MUTED)),
            ]
            .spacing(2),
        )
        .padding([14, 18])
        .width(Length::FillPortion(1))
        .style(iced::theme::Container::Custom(Box::new(style::CardStyle)));

        let stat_card_3 = container(
            column![
                text(format!("{} Drives", self.active_drives)).size(26),
                text("Monitored Drives").size(12).style(iced::theme::Text::Color(style::COLOR_TEXT_MUTED)),
            ]
            .spacing(2),
        )
        .padding([14, 18])
        .width(Length::FillPortion(1))
        .style(iced::theme::Container::Custom(Box::new(style::CardStyle)));

        let stats_row = row![stat_card_1, stat_card_2, stat_card_3].spacing(14);

        // MAIN CONTENT SWITCHER BASED ON TAB
        let main_body: Element<'_, Message> = match self.current_tab {
            Tab::Overview => {
                // Search Studio Card (Matching the "Folder snapshot" card in screenshot)
                let search_header_row = row![
                    win_icon(icons::SEARCH, 15).style(iced::theme::Text::Color(style::COLOR_PRIMARY)),
                    text("Instant File Search").size(15),
                    Space::with_width(Length::Fill),
                    text(&self.status_msg).size(11).style(iced::theme::Text::Color(style::COLOR_TEXT_MUTED)),
                ]
                .spacing(8)
                .align_items(Alignment::Center);

                let search_box = text_input(
                    "Search files... (e.g. invoice ext:pdf path:finance -draft)",
                    &self.query,
                )
                .on_input(Message::SearchChanged)
                .padding([12, 14])
                .size(16);

                let quick_dorks_row = row![
                    text("Dorks:").size(11).style(iced::theme::Text::Color(style::COLOR_TEXT_MUTED)),
                    button(text("ext:pdf").size(11)).on_press(Message::QuickFilterClicked("ext:pdf".into())).padding([3, 8]).style(iced::theme::Button::Custom(Box::new(style::QuickChipBtn))),
                    button(text("ext:docx").size(11)).on_press(Message::QuickFilterClicked("ext:docx".into())).padding([3, 8]).style(iced::theme::Button::Custom(Box::new(style::QuickChipBtn))),
                    button(text("ext:xlsx").size(11)).on_press(Message::QuickFilterClicked("ext:xlsx".into())).padding([3, 8]).style(iced::theme::Button::Custom(Box::new(style::QuickChipBtn))),
                    button(text("ext:png,jpg").size(11)).on_press(Message::QuickFilterClicked("ext:png,jpg".into())).padding([3, 8]).style(iced::theme::Button::Custom(Box::new(style::QuickChipBtn))),
                    button(text("*.rs").size(11)).on_press(Message::QuickFilterClicked("*.rs".into())).padding([3, 8]).style(iced::theme::Button::Custom(Box::new(style::QuickChipBtn))),
                    button(text("path:").size(11)).on_press(Message::QuickFilterClicked("path:".into())).padding([3, 8]).style(iced::theme::Button::Custom(Box::new(style::QuickChipBtn))),
                    button(text("name:").size(11)).on_press(Message::QuickFilterClicked("name:".into())).padding([3, 8]).style(iced::theme::Button::Custom(Box::new(style::QuickChipBtn))),
                    button(text("-exclude").size(11)).on_press(Message::QuickFilterClicked("-".into())).padding([3, 8]).style(iced::theme::Button::Custom(Box::new(style::QuickChipBtn))),
                    button(text("OR").size(11)).on_press(Message::QuickFilterClicked("OR".into())).padding([3, 8]).style(iced::theme::Button::Custom(Box::new(style::QuickChipBtn))),
                ]
                .spacing(6)
                .align_items(Alignment::Center);

                let badges = self.parsed_filter.badges();
                let mut badges_row = row![].spacing(6).align_items(Alignment::Center);
                for (kind, val) in &badges {
                    let badge_label = format!("{}: {}", kind, val);
                    let badge_item = container(
                        text(badge_label)
                            .size(11)
                            .style(iced::theme::Text::Color(style::COLOR_PRIMARY)),
                    )
                    .padding([2, 8])
                    .style(iced::theme::Container::Box);
                    badges_row = badges_row.push(badge_item);
                }

                let mut search_card_col = column![
                    search_header_row,
                    search_box,
                    quick_dorks_row,
                ]
                .spacing(10);

                if !badges.is_empty() {
                    search_card_col = search_card_col.push(badges_row);
                }

                let search_card = container(search_card_col)
                    .padding(16)
                    .width(Length::Fill)
                    .style(iced::theme::Container::Custom(Box::new(style::CardStyle)));

                // Selection & Action Toolbar
                let mut actions_row = row![
                    text("Recent Results").size(15),
                    Space::with_width(Length::Fixed(12.0)),
                    text(&self.search_time_msg).size(12).style(iced::theme::Text::Color(style::COLOR_PRIMARY)),
                    Space::with_width(Length::Fill),
                ]
                .align_items(Alignment::Center);

                if !self.results.is_empty() {
                    let all_selected = self.results.len() == self.selected_files.len();
                    let select_btn = if all_selected {
                        button(text("Deselect All").size(11))
                            .on_press(Message::DeselectAll)
                            .padding([3, 8])
                            .style(iced::theme::Button::Custom(Box::new(style::SecondaryPillBtn)))
                    } else {
                        button(text("Select All").size(11))
                            .on_press(Message::SelectAll)
                            .padding([3, 8])
                            .style(iced::theme::Button::Custom(Box::new(style::SecondaryPillBtn)))
                    };
                    actions_row = actions_row.push(select_btn);

                    if !self.selected_files.is_empty() {
                        let count = self.selected_files.len();
                        let count_badge = container(
                            text(format!("{} selected", count))
                                .size(11)
                                .style(iced::theme::Text::Color(style::COLOR_PRIMARY)),
                        )
                        .padding([3, 8])
                        .style(iced::theme::Container::Box);

                        let copy_btn = button(
                            row![
                                win_icon(icons::COPY, 12),
                                text(format!("Copy ({})", count)).size(11),
                            ]
                            .spacing(6)
                            .align_items(Alignment::Center),
                        )
                        .on_press(Message::CopySelected)
                        .padding([3, 10])
                        .style(iced::theme::Button::Custom(Box::new(style::PrimaryPillBtn)));

                        let cut_btn = button(
                            row![
                                win_icon(icons::CUT, 12),
                                text(format!("Cut ({})", count)).size(11),
                            ]
                            .spacing(6)
                            .align_items(Alignment::Center),
                        )
                        .on_press(Message::CutSelected)
                        .padding([3, 10])
                        .style(iced::theme::Button::Custom(Box::new(style::SecondaryPillBtn)));

                        let clear_btn = button(
                            row![
                                win_icon(icons::CLEAR, 10),
                                text("Clear").size(11),
                            ]
                            .spacing(4)
                            .align_items(Alignment::Center),
                        )
                        .on_press(Message::DeselectAll)
                        .padding([3, 8])
                        .style(iced::theme::Button::Custom(Box::new(style::SecondaryPillBtn)));

                        actions_row = actions_row.push(count_badge);
                        actions_row = actions_row.push(copy_btn);
                        actions_row = actions_row.push(cut_btn);
                        actions_row = actions_row.push(clear_btn);
                    }
                }

                // Results Container Card
                let mut results_col = Column::new().spacing(6);
                if self.results.is_empty() {
                    let empty_msg = if self.query.trim().is_empty() {
                        "Type a keyword or dork above (e.g. invoice ext:pdf or path:work) to search instantly."
                    } else {
                        "No matching files found. Check your query filters or try Quick Sync."
                    };
                    results_col = results_col.push(
                        text(empty_msg)
                            .size(13)
                            .style(iced::theme::Text::Color(style::COLOR_TEXT_MUTED)),
                    );
                } else {
                    for path in &self.results {
                        let (dir_part, filename) = match path.rfind('\\') {
                            Some(idx) => (&path[..idx], &path[idx + 1..]),
                            None => ("", path.as_str()),
                        };

                        let is_selected = self.selected_files.contains(path);
                        let path_for_toggle = path.clone();

                        let check_widget = checkbox("", is_selected)
                            .on_toggle(move |_| Message::ToggleFileSelection(path_for_toggle.clone()))
                            .size(16);

                        let file_label = column![
                            text(filename).size(14),
                            text(dir_part)
                                .size(11)
                                .style(iced::theme::Text::Color(style::COLOR_TEXT_MUTED)),
                        ]
                        .spacing(1);

                        let open_file_btn = button(file_label)
                            .on_press(Message::FileClicked(path.clone()))
                            .style(iced::theme::Button::Custom(Box::new(style::SecondaryPillBtn)))
                            .padding([6, 10])
                            .width(Length::Fill);

                        let open_dir_btn = button(
                            row![
                                win_icon(icons::REVEAL, 12),
                                text("Reveal").size(11),
                            ]
                            .spacing(5)
                            .align_items(Alignment::Center),
                        )
                        .on_press(Message::OpenFolderClicked(path.clone()))
                        .style(iced::theme::Button::Custom(Box::new(style::SecondaryPillBtn)))
                        .padding([5, 8]);

                        let copy_btn = button(
                            row![
                                win_icon(icons::COPY, 12),
                                text("Copy").size(11),
                            ]
                            .spacing(5)
                            .align_items(Alignment::Center),
                        )
                        .on_press(Message::CopySingleFile(path.clone()))
                        .style(iced::theme::Button::Custom(Box::new(style::SecondaryPillBtn)))
                        .padding([5, 8]);

                        let cut_btn = button(
                            row![
                                win_icon(icons::CUT, 12),
                                text("Cut").size(11),
                            ]
                            .spacing(5)
                            .align_items(Alignment::Center),
                        )
                        .on_press(Message::CutSingleFile(path.clone()))
                        .style(iced::theme::Button::Custom(Box::new(style::SecondaryPillBtn)))
                        .padding([5, 8]);

                        let item_row = row![
                            check_widget,
                            open_file_btn,
                            open_dir_btn,
                            copy_btn,
                            cut_btn,
                        ]
                        .spacing(6)
                        .align_items(Alignment::Center);

                        results_col = results_col.push(item_row);
                    }
                }

                let results_card = container(
                    column![
                        actions_row,
                        scrollable(results_col).height(Length::Fill),
                    ]
                    .spacing(10),
                )
                .padding(16)
                .width(Length::Fill)
                .height(Length::Fill)
                .style(iced::theme::Container::Custom(Box::new(style::CardStyle)));

                column![
                    search_card,
                    results_card,
                ]
                .spacing(14)
                .height(Length::Fill)
                .into()
            }

            Tab::DorksGuide => {
                let guide_card = container(
                    column![
                        row![
                            win_icon(icons::RULES, 18).style(iced::theme::Text::Color(style::COLOR_PRIMARY)),
                            text("Search Dorks & Filter Rules Reference").size(18),
                        ]
                        .spacing(8)
                        .align_items(Alignment::Center),
                        text("Filter through 1.4 million files with surgical precision:").size(13).style(iced::theme::Text::Color(style::COLOR_TEXT_MUTED)),
                        Space::with_height(Length::Fixed(8.0)),
                        row![
                            text("• File Extension:").size(13),
                            text("ext:pdf   filetype:docx   ext:png,jpg   *.rs   .toml").size(13).style(iced::theme::Text::Color(style::COLOR_PRIMARY)),
                        ].spacing(10),
                        row![
                            text("• Filename Only:").size(13),
                            text("name:report   filename:invoice   intitle:readme").size(13).style(iced::theme::Text::Color(style::COLOR_PRIMARY)),
                        ].spacing(10),
                        row![
                            text("• Folder / Path:").size(13),
                            text("path:documents   dir:work   folder:projects   inurl:src").size(13).style(iced::theme::Text::Color(style::COLOR_PRIMARY)),
                        ].spacing(10),
                        row![
                            text("• Drive / Site:").size(13),
                            text("drive:c   site:d:").size(13).style(iced::theme::Text::Color(style::COLOR_PRIMARY)),
                        ].spacing(10),
                        row![
                            text("• Exclude / NOT:").size(13),
                            text("-draft   -ext:tmp   -path:node_modules   -\"old backup\"").size(13).style(iced::theme::Text::Color(style::COLOR_PRIMARY)),
                        ].spacing(10),
                        row![
                            text("• Exact Phrase:").size(13),
                            text("\"quarterly report 2024\"").size(13).style(iced::theme::Text::Color(style::COLOR_PRIMARY)),
                        ].spacing(10),
                        row![
                            text("• Boolean OR:").size(13),
                            text("cat OR dog      ext:png OR ext:jpg").size(13).style(iced::theme::Text::Color(style::COLOR_PRIMARY)),
                        ].spacing(10),
                        row![
                            text("• Combined:").size(13),
                            text("budget 2024 ext:xlsx,csv path:finance -draft").size(13).style(iced::theme::Text::Color(style::COLOR_PRIMARY)),
                        ].spacing(10),
                        Space::with_height(Length::Fixed(12.0)),
                        button(text("Back to Overview Dashboard").size(12))
                            .on_press(Message::SelectTab(Tab::Overview))
                            .style(iced::theme::Button::Custom(Box::new(style::PrimaryPillBtn)))
                            .padding([6, 14]),
                    ]
                    .spacing(8),
                )
                .padding(24)
                .width(Length::Fill)
                .style(iced::theme::Container::Custom(Box::new(style::CardStyle)));

                column![guide_card].spacing(14).height(Length::Fill).into()
            }

            Tab::Drives => {
                let drives_card = container(
                    column![
                        row![
                            win_icon(icons::DRIVES, 18).style(iced::theme::Text::Color(style::COLOR_PRIMARY)),
                            text("Monitored Drives & Storage").size(18),
                        ]
                        .spacing(8)
                        .align_items(Alignment::Center),
                        text("OmniFind monitors your system storage and user files:").size(13).style(iced::theme::Text::Color(style::COLOR_TEXT_MUTED)),
                        Space::with_height(Length::Fixed(8.0)),
                        row![
                            text("• Total Files in RAM:").size(13),
                            text(format_number(file_count)).size(13).style(iced::theme::Text::Color(style::COLOR_PRIMARY)),
                        ].spacing(10),
                        row![
                            text("• Monitored Drives:").size(13),
                            text(format!("{} active Windows drives", self.active_drives)).size(13).style(iced::theme::Text::Color(style::COLOR_PRIMARY)),
                        ].spacing(10),
                        row![
                            text("• Live Watcher:").size(13),
                            text("Active (watches C:\\Users and data drive roots in real-time)").size(13).style(iced::theme::Text::Color(iced::Color::from_rgb(0.2, 0.75, 0.4))),
                        ].spacing(10),
                        Space::with_height(Length::Fixed(12.0)),
                        row![
                            button(
                                row![
                                    win_icon(icons::SAVE, 12),
                                    text("Save RAM Index to Disk").size(12),
                                ]
                                .spacing(6)
                                .align_items(Alignment::Center),
                            )
                            .on_press(Message::StartSaveIndex)
                            .style(iced::theme::Button::Custom(Box::new(style::PrimaryPillBtn)))
                            .padding([6, 14]),
                            button(
                                row![
                                    win_icon(icons::SYNC, 12),
                                    text("Run Quick Sync (1-2s)").size(12),
                                ]
                                .spacing(6)
                                .align_items(Alignment::Center),
                            )
                            .on_press(Message::StartQuickSync)
                            .style(iced::theme::Button::Custom(Box::new(style::SecondaryPillBtn)))
                            .padding([6, 14]),
                            button(
                                row![
                                    win_icon(icons::RESCAN, 12),
                                    text("Run Full Rescan").size(12),
                                ]
                                .spacing(6)
                                .align_items(Alignment::Center),
                            )
                            .on_press(Message::StartFullRescan)
                            .style(iced::theme::Button::Custom(Box::new(style::SecondaryPillBtn)))
                            .padding([6, 14]),
                        ]
                        .spacing(10),
                    ]
                    .spacing(8),
                )
                .padding(24)
                .width(Length::Fill)
                .style(iced::theme::Container::Custom(Box::new(style::CardStyle)));

                column![drives_card].spacing(14).height(Length::Fill).into()
            }
        };

        // Bottom Status Row (Matching screenshot bottom bar)
        let bottom_status_row = row![
            row![
                win_icon(icons::DRIVES, 11).style(iced::theme::Text::Color(style::COLOR_TEXT_MUTED)),
                text(format!("{} drives indexed", self.active_drives)).size(11).style(iced::theme::Text::Color(style::COLOR_TEXT_MUTED)),
            ]
            .spacing(5)
            .align_items(Alignment::Center),
            text("•").size(11).style(iced::theme::Text::Color(style::COLOR_TEXT_MUTED)),
            text(format!("{} selected", self.selected_files.len())).size(11).style(iced::theme::Text::Color(style::COLOR_TEXT_MUTED)),
            text("•").size(11).style(iced::theme::Text::Color(style::COLOR_TEXT_MUTED)),
            row![
                win_icon(icons::CHECK, 11).style(iced::theme::Text::Color(style::COLOR_TEXT_MUTED)),
                text("Live watcher active").size(11).style(iced::theme::Text::Color(style::COLOR_TEXT_MUTED)),
            ]
            .spacing(5)
            .align_items(Alignment::Center),
            Space::with_width(Length::Fill),
            text(format!("OmniFind v{}", CURRENT_VERSION)).size(11).style(iced::theme::Text::Color(style::COLOR_TEXT_MUTED)),
        ]
        .spacing(8)
        .align_items(Alignment::Center);

        // Conditional layout construction: only pushes progress section when indexing to prevent layout jump (#22)
        let mut content_column = column![main_header];
        if self.is_indexing {
            let progress_section = column![
                text(&self.status_msg).size(12).style(iced::theme::Text::Color(style::COLOR_PRIMARY)),
                progress_bar(0.0..=1.0, self.progress).height(Length::Fixed(6.0)),
            ]
            .spacing(4);
            content_column = content_column.push(progress_section);
        }

        content_column = content_column
            .push(stats_row)
            .push(main_body)
            .push(bottom_status_row)
            .spacing(14)
            .padding([18, 24])
            .width(Length::Fill)
            .height(Length::Fill);

        // Entire Layout: Sidebar + Main Content Area
        let main_layout = row![
            sidebar,
            content_column,
        ]
        .width(Length::Fill)
        .height(Length::Fill);

        container(main_layout)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(iced::theme::Container::Custom(Box::new(style::WindowBgStyle)))
            .into()
    }

    fn theme(&self) -> Theme {
        Theme::Light
    }
}

fn format_number(n: usize) -> String {
    let s = n.to_string();
    let mut result = String::new();
    let chars: Vec<char> = s.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if i > 0 && (chars.len() - i) % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result
}

fn count_active_drives() -> usize {
    let mut count = 0;
    for letter in b'C'..=b'Z' {
        let path = format!("{}:\\", char::from(letter));
        if Path::new(&path).exists() {
            count += 1;
        }
    }
    count.max(1)
}

/// Reveals the target file in Windows File Explorer, selecting it and scrolling it into view.
#[cfg(windows)]
mod shell_reveal {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    type HRESULT = i32;
    const S_OK: HRESULT = 0;

    #[link(name = "shell32")]
    unsafe extern "system" {
        fn ILCreateFromPathW(pszPath: *const u16) -> *mut c_void;
        fn ILFree(pidl: *mut c_void);
        fn SHOpenFolderAndSelectItems(
            pidlFolder: *const c_void,
            cidl: u32,
            apidl: *const *const c_void,
            dwFlags: u32,
        ) -> HRESULT;
    }

    #[link(name = "ole32")]
    unsafe extern "system" {
        fn CoInitializeEx(pvReserved: *mut c_void, dwCoInit: u32) -> HRESULT;
    }

    const COINIT_APARTMENTTHREADED: u32 = 0x2;

    pub fn reveal(path: &str) -> bool {
        let wide: Vec<u16> = Path::new(path)
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        unsafe {
            let _ = CoInitializeEx(std::ptr::null_mut(), COINIT_APARTMENTTHREADED);

            let pidl = ILCreateFromPathW(wide.as_ptr());
            if !pidl.is_null() {
                // When cidl == 0 and apidl is null, pidlFolder points to the single item to select and scroll to.
                let hr = SHOpenFolderAndSelectItems(pidl, 0, std::ptr::null(), 0);
                ILFree(pidl);
                if hr == S_OK {
                    return true;
                }
            }
        }
        false
    }
}

pub fn reveal_in_explorer(path: &str) {
    #[cfg(windows)]
    {
        // 1. Primary method: Windows Shell API (SHOpenFolderAndSelectItems)
        // This natively reveals the file, selects it, and scrolls it directly into view.
        if shell_reveal::reveal(path) {
            return;
        }

        // 2. Fallback: explorer.exe /select,"<path>"
        // Using raw_arg ensures Rust does not wrap /select, in quotes, which explorer.exe rejects.
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("explorer")
            .raw_arg(format!("/select,\"{}\"", path))
            .spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = open::that(path);
    }
}
