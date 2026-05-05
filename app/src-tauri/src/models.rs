use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "status", content = "data")]
pub enum AuthState {
    LoggedOut,
    AwaitingCode { phone: String, phone_code_hash: String },
    AwaitingPassword { phone: String },
    LoggedIn,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AuthResult {
    pub success: bool,
    pub next_step: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FileMetadata {
    pub id: i64,
    pub folder_id: Option<i64>,
    pub name: String,
    pub size: u64,
    pub mime_type: Option<String>,
    pub file_ext: Option<String>,
    pub created_at: String,
    pub icon_type: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FolderMetadata {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Drive {
    pub chat_id: i64,
    pub name: String,
    pub icon: Option<String>,
}

// ── Google Drive Sync Models ─────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum SyncStatusKind {
    #[default]
    Disconnected,
    Connecting,
    Connected,
    Syncing,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub state: SyncStatusKind,
    pub message: String,
    pub last_synced: Option<String>,
    pub files_synced: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GDriveFile {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    pub size: Option<String>,
    pub modified_time: Option<String>,
    pub parents: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncEvent {
    pub event_type: String, // "file_synced" | "error" | "status_changed"
    pub file_name: Option<String>,
    pub message: String,
}
