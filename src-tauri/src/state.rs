//! Shared application state.

use fillerncut_core::{EncoderSupport, HistoryStore, LibraryStore, Settings};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use tokio::sync::OnceCell;
use tokio_util::sync::CancellationToken;

pub struct AppState {
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub settings_path: PathBuf,
    pub settings: Mutex<Settings>,
    pub library: LibraryStore,
    pub history: HistoryStore,
    pub encoders: OnceCell<EncoderSupport>,
    jobs: Mutex<HashMap<String, CancellationToken>>,
}

impl AppState {
    pub fn new(data_dir: PathBuf, cache_dir: PathBuf) -> AppState {
        let settings_path = data_dir.join("settings.json");
        AppState {
            settings: Mutex::new(Settings::load(&settings_path)),
            library: LibraryStore::new(data_dir.join("watermarks")),
            history: HistoryStore::new(data_dir.join("history")),
            settings_path,
            data_dir,
            cache_dir,
            encoders: OnceCell::new(),
            jobs: Mutex::new(HashMap::new()),
        }
    }

    pub fn settings(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }

    /// Register a long-running job. Only one job per name may run at a time.
    pub fn begin_job(&self, name: &str) -> Result<JobGuard<'_>, String> {
        let mut jobs = self.jobs.lock().unwrap();
        if jobs.contains_key(name) {
            return Err(format!("A {name} is already running"));
        }
        let token = CancellationToken::new();
        jobs.insert(name.to_string(), token.clone());
        Ok(JobGuard {
            state: self,
            name: name.to_string(),
            token,
        })
    }

    pub fn cancel_job(&self, name: &str) {
        if let Some(t) = self.jobs.lock().unwrap().get(name) {
            t.cancel();
        }
    }
}

/// Unregisters the job when dropped, so a panic or early return can't leave
/// the app stuck thinking a download is still running.
pub struct JobGuard<'a> {
    state: &'a AppState,
    name: String,
    pub token: CancellationToken,
}

impl Drop for JobGuard<'_> {
    fn drop(&mut self) {
        self.state.jobs.lock().unwrap().remove(&self.name);
    }
}
