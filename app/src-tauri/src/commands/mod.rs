use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::Mutex;
use grammers_client::{Client};
use grammers_client::types::{LoginToken, PasswordToken, Peer};

/// Tracks the lifecycle of the Telegram connection
#[derive(Clone)]
pub struct TelegramState {
    pub client: Arc<Mutex<Option<Client>>>,
    pub login_token: Arc<Mutex<Option<LoginToken>>>,
    pub password_token: Arc<Mutex<Option<PasswordToken>>>,
    pub api_id: Arc<Mutex<Option<i32>>>,
    pub runner_shutdown: Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    pub runner_count: Arc<std::sync::atomic::AtomicU32>,
    pub peer_cache: Arc<tokio::sync::RwLock<HashMap<i64, Peer>>>,
}

pub mod auth;
pub mod fs;
pub mod preview;
pub mod utils;
pub mod network;
pub mod streaming;
pub mod gdrive;

pub use auth::*;
pub use fs::*;
pub use preview::*;
pub use utils::*;
pub use network::*;
pub use streaming::*;
pub use gdrive::{
    GDriveState,
    cmd_gdrive_set_credentials,
    cmd_gdrive_connect,
    cmd_gdrive_disconnect,
    cmd_gdrive_sync_status,
    cmd_gdrive_list_files,
};
