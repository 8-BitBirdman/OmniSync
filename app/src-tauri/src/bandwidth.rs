use std::sync::Mutex;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use chrono::Local;
use tauri::Manager;

#[derive(Debug, Clone, Copy)]
pub enum TransferKind {
    Up,
    Down,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BandwidthStats {
    pub date: String,
    pub up_bytes: u64,
    pub down_bytes: u64,
}

impl Default for BandwidthStats {
    fn default() -> Self {
        Self {
            date: Local::now().format("%Y-%m-%d").to_string(),
            up_bytes: 0,
            down_bytes: 0,
        }
    }
}

pub struct BandwidthManager {
    pub file_path: PathBuf,
    pub stats: Mutex<BandwidthStats>,
    pub limit: u64, // Daily limit in bytes
}

impl BandwidthManager {
    pub fn new(app_handle: &tauri::AppHandle) -> Self {
        // Resolve a writable data dir. Try app_data_dir first; fall back to the
        // OS temp dir (always writable) rather than CWD which is often read-only
        // inside packaged macOS/Windows bundles.
        let app_data_dir = app_handle
            .path()
            .app_data_dir()
            .unwrap_or_else(|_| std::env::temp_dir().join("omnisync"));

        if !app_data_dir.exists() {
            let _ = std::fs::create_dir_all(&app_data_dir);
        }
        let file_path = app_data_dir.join("bandwidth.json");

        let stats = if file_path.exists() {
            let content = fs::read_to_string(&file_path).unwrap_or_default();
            match serde_json::from_str(&content) {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("[Bandwidth] bandwidth.json parse failed ({}); resetting counters", e);
                    BandwidthStats::default()
                }
            }
        } else {
            BandwidthStats::default()
        };

        Self {
            file_path,
            stats: Mutex::new(stats),
            limit: 250 * 1024 * 1024 * 1024, // 250 GB
        }
    }

    pub fn check_and_reset(&self) {
        let today = Local::now().format("%Y-%m-%d").to_string();
        let snapshot = {
            let mut stats = self.stats.lock().unwrap_or_else(|e| e.into_inner());
            if stats.date == today {
                return;
            }
            log::info!("[Bandwidth] New day detected. Resetting. Old: {}, New: {}", stats.date, today);
            stats.date = today;
            stats.up_bytes = 0;
            stats.down_bytes = 0;
            stats.clone()
        };
        self.save_locked(&snapshot);
    }

    pub fn can_transfer(&self, bytes: u64) -> Result<(), String> {
        self.check_and_reset();
        let stats = self.stats.lock().unwrap_or_else(|e| e.into_inner());
        let total = stats.up_bytes.saturating_add(stats.down_bytes).saturating_add(bytes);
        if total > self.limit {
            return Err(format!("Daily bandwidth limit ({}) exceeded! Used: {}", self.format_bytes(self.limit), self.format_bytes(total)));
        }
        Ok(())
    }

    /// Atomic check-and-increment. Returns Err(...) if reserving `bytes` would
    /// exceed the daily cap, otherwise increments the counter for `kind` and
    /// returns Ok. Use [`refund`] on transfer failure.
    pub fn try_reserve(&self, bytes: u64, kind: TransferKind) -> Result<(), String> {
        self.check_and_reset();
        let snapshot = {
            let mut stats = self.stats.lock().unwrap_or_else(|e| e.into_inner());
            let total = stats.up_bytes.saturating_add(stats.down_bytes).saturating_add(bytes);
            if total > self.limit {
                return Err(format!(
                    "Daily bandwidth limit ({}) exceeded! Used: {}",
                    self.format_bytes(self.limit),
                    self.format_bytes(total)
                ));
            }
            match kind {
                TransferKind::Up   => stats.up_bytes   = stats.up_bytes.saturating_add(bytes),
                TransferKind::Down => stats.down_bytes = stats.down_bytes.saturating_add(bytes),
            }
            stats.clone()
        };
        self.save_locked(&snapshot);
        Ok(())
    }

    /// Refund previously-reserved bytes (call on partial/failed transfer).
    pub fn refund(&self, bytes: u64, kind: TransferKind) {
        if bytes == 0 { return; }
        let snapshot = {
            let mut stats = self.stats.lock().unwrap_or_else(|e| e.into_inner());
            match kind {
                TransferKind::Up   => stats.up_bytes   = stats.up_bytes.saturating_sub(bytes),
                TransferKind::Down => stats.down_bytes = stats.down_bytes.saturating_sub(bytes),
            }
            stats.clone()
        };
        self.save_locked(&snapshot);
    }

    pub fn add_up(&self, bytes: u64) {
        self.check_and_reset();
        let snapshot = {
            let mut stats = self.stats.lock().unwrap_or_else(|e| e.into_inner());
            stats.up_bytes = stats.up_bytes.saturating_add(bytes);
            stats.clone()
        };
        self.save_locked(&snapshot);
    }

    pub fn add_down(&self, bytes: u64) {
        self.check_and_reset();
        let snapshot = {
            let mut stats = self.stats.lock().unwrap_or_else(|e| e.into_inner());
            stats.down_bytes = stats.down_bytes.saturating_add(bytes);
            stats.clone()
        };
        self.save_locked(&snapshot);
    }

    fn save_locked(&self, stats: &BandwidthStats) {
        if let Ok(json) = serde_json::to_string(stats) {
            let _ = fs::write(&self.file_path, json);
        }
    }

    pub fn get_stats(&self) -> BandwidthStats {
        self.check_and_reset();
        self.stats.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn format_bytes(&self, bytes: u64) -> String {
        const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
        let mut v = bytes as f64;
        let mut i = 0;
        while v >= 1024.0 && i < UNITS.len() - 1 {
            v /= 1024.0;
            i += 1;
        }
        format!("{:.2} {}", v, UNITS[i])
    }
}
