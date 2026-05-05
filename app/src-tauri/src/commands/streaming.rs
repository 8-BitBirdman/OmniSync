use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};
use tauri::State;
use tokio::sync::RwLock;

/// Holds the per-session streaming config. Port is atomic because the actual
/// listener may fall back to an OS-assigned ephemeral port if the preferred
/// one is taken — see `server::start_server`. Token is wrapped in
/// `Arc<RwLock<String>>` so it can be rotated per Telegram session and the
/// change is visible to the running Actix server (which holds the same Arc).
pub struct StreamConfig {
    pub token: Arc<RwLock<String>>,
    pub port: AtomicU16,
}

/// Returned to the frontend so it can construct stream URLs dynamically
#[derive(serde::Serialize)]
pub struct StreamInfo {
    pub token: String,
    pub base_url: String,
}

/// Returns the streaming server's session token and base URL to the frontend.
/// The frontend must use the returned base_url to construct stream URLs,
/// never hardcoding the port.
#[tauri::command]
pub async fn cmd_get_stream_info(config: State<'_, StreamConfig>) -> Result<StreamInfo, String> {
    let port = config.port.load(Ordering::Acquire);
    let token = config.token.read().await.clone();
    Ok(StreamInfo {
        token,
        base_url: format!("http://localhost:{}", port),
    })
}

/// Regenerate the stream token (call on connect/logout to invalidate prior URLs).
pub async fn rotate_stream_token(app_handle: &tauri::AppHandle) {
    use tauri::Manager;
    if let Some(cfg) = app_handle.try_state::<StreamConfig>() {
        let new_tok = crate::generate_stream_token();
        *cfg.token.write().await = new_tok;
        log::info!("Rotated stream token");
    }
}
