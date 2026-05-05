use tauri::{State, Emitter, AppHandle};
use tauri_plugin_shell::ShellExt;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;
use crate::models::{SyncStatus, SyncStatusKind, SyncEvent, GDriveFile};

// ── State ────────────────────────────────────────────────────────

#[derive(Clone, Default)]
pub struct GDriveState {
    pub access_token:   Arc<Mutex<Option<String>>>,
    pub refresh_token:  Arc<Mutex<Option<String>>>,
    pub token_expiry:   Arc<Mutex<u64>>,
    pub client_id:      Arc<Mutex<Option<String>>>,
    pub client_secret:  Arc<Mutex<Option<String>>>,
    pub status:         Arc<Mutex<SyncStatus>>,
    pub changes_token:  Arc<Mutex<Option<String>>>,
    pub files_synced:   Arc<std::sync::atomic::AtomicU64>,
    /// Send () to stop the background sync loop
    pub sync_shutdown:  Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
}

// ── Google API response shapes ───────────────────────────────────

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
    refresh_token: Option<String>,
}

#[derive(Deserialize)]
struct DriveChangesResponse {
    #[serde(rename = "newStartPageToken")]
    new_start_page_token: Option<String>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
    changes: Option<Vec<DriveChange>>,
}

#[derive(Deserialize, Clone)]
struct DriveChange {
    #[serde(rename = "fileId")]
    file_id: Option<String>,
    file: Option<DriveChangeFile>,
    removed: Option<bool>,
}

#[derive(Deserialize, Clone)]
struct DriveChangeFile {
    id: String,
    name: String,
    #[serde(rename = "mimeType")]
    mime_type: String,
    size: Option<String>,
    #[serde(rename = "modifiedTime")]
    modified_time: Option<String>,
    parents: Option<Vec<String>>,
    trashed: Option<bool>,
}

#[derive(Deserialize)]
struct DriveFileListResponse {
    files: Option<Vec<DriveChangeFile>>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

// ── OAuth helpers ────────────────────────────────────────────────

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

async fn refresh_access_token(
    client: &reqwest::Client,
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
) -> Result<(String, u64), String> {
    let params = [
        ("client_id",     client_id),
        ("client_secret", client_secret),
        ("refresh_token", refresh_token),
        ("grant_type",    "refresh_token"),
    ];
    let resp: TokenResponse = client
        .post("https://oauth2.googleapis.com/token")
        .form(&params)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let expiry = unix_now() + resp.expires_in - 60;
    Ok((resp.access_token, expiry))
}

/// Returns a valid access token, refreshing if necessary.
async fn get_valid_token(state: &GDriveState) -> Result<String, String> {
    let now = unix_now();
    let expiry = *state.token_expiry.lock().await;
    if now < expiry {
        let tok = state.access_token.lock().await;
        if let Some(t) = tok.as_ref() {
            return Ok(t.clone());
        }
    }
    // Need to refresh
    let refresh_token = state.refresh_token.lock().await
        .clone()
        .ok_or("Not connected to Google Drive")?;
    let client_id = state.client_id.lock().await
        .clone()
        .ok_or("Google OAuth credentials not configured")?;
    let client_secret = state.client_secret.lock().await
        .clone()
        .ok_or("Google OAuth credentials not configured")?;

    let http = reqwest::Client::new();
    let (new_tok, new_expiry) = refresh_access_token(&http, &client_id, &client_secret, &refresh_token).await?;
    *state.access_token.lock().await  = Some(new_tok.clone());
    *state.token_expiry.lock().await  = new_expiry;
    Ok(new_tok)
}

// ── Tauri Commands ───────────────────────────────────────────────

/// Store OAuth client_id + client_secret (from user's Google Cloud project).
#[tauri::command]
pub async fn cmd_gdrive_set_credentials(
    client_id: String,
    client_secret: String,
    state: State<'_, GDriveState>,
) -> Result<bool, String> {
    *state.client_id.lock().await     = Some(client_id);
    *state.client_secret.lock().await = Some(client_secret);
    log::info!("[GDrive] OAuth credentials stored.");
    Ok(true)
}

/// Begin OAuth2 loopback flow. Opens the browser and waits for the auth code
/// via a temporary HTTP listener. Returns when tokens are exchanged.
#[tauri::command]
pub async fn cmd_gdrive_connect(
    app_handle: AppHandle,
    state: State<'_, GDriveState>,
) -> Result<bool, String> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;

    let client_id = state.client_id.lock().await
        .clone()
        .ok_or("Set OAuth credentials first via cmd_gdrive_set_credentials")?;
    let client_secret = state.client_secret.lock().await
        .clone()
        .ok_or("Set OAuth credentials first via cmd_gdrive_set_credentials")?;

    // Pick an ephemeral port for the loopback callback
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("Cannot bind OAuth listener: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect_uri = format!("http://127.0.0.1:{}/gdrive/callback", port);

    let auth_url = format!(
        "https://accounts.google.com/o/oauth2/v2/auth?\
         client_id={client_id}\
         &redirect_uri={redirect_uri}\
         &response_type=code\
         &scope=https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fdrive.readonly\
         &access_type=offline\
         &prompt=consent",
        client_id = urlencoding::encode(&client_id),
        redirect_uri = urlencoding::encode(&redirect_uri),
    );

    // Update status
    {
        let mut s = state.status.lock().await;
        s.state = SyncStatusKind::Connecting;
        s.message = "Waiting for Google sign-in…".to_string();
    }
    let _ = app_handle.emit("gdrive-status", state.status.lock().await.clone());

    // Open browser
    let _ = app_handle.shell().open(&auth_url, None);

    // Wait for callback (one connection only)
    let (mut socket, _) = listener.accept().await.map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(&mut socket);
    let mut request_line = String::new();
    reader.read_line(&mut request_line).await.map_err(|e| e.to_string())?;

    // Parse ?code= from "GET /gdrive/callback?code=XXXX HTTP/1.1"
    let code = request_line
        .split_whitespace()
        .nth(1)
        .and_then(|path| url::Url::parse(&format!("http://localhost{}", path)).ok())
        .and_then(|u| u.query_pairs().find(|(k, _)| k == "code").map(|(_, v)| v.into_owned()))
        .ok_or("OAuth callback did not contain a code")?;

    // Send HTTP response to browser
    let html = b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n\
        <html><body style='font-family:sans-serif;text-align:center;padding:60px'>\
        <h2>\u{2705} Connected to Google Drive</h2>\
        <p>You can close this tab and return to Antigravity Drive.</p></body></html>";
    let _ = socket.write_all(html).await;

    // Exchange code for tokens
    let http = reqwest::Client::new();
    let params = [
        ("code",          code.as_str()),
        ("client_id",     client_id.as_str()),
        ("client_secret", client_secret.as_str()),
        ("redirect_uri",  redirect_uri.as_str()),
        ("grant_type",    "authorization_code"),
    ];
    let token_resp: TokenResponse = http
        .post("https://oauth2.googleapis.com/token")
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Token exchange failed: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Token parse failed: {e}"))?;

    let expiry = unix_now() + token_resp.expires_in - 60;
    *state.access_token.lock().await  = Some(token_resp.access_token);
    *state.token_expiry.lock().await  = expiry;
    if let Some(rt) = token_resp.refresh_token {
        *state.refresh_token.lock().await = Some(rt);
    }

    // Fetch initial changes page token
    let access_token = get_valid_token(&state).await?;
    let start_token = http
        .get("https://www.googleapis.com/drive/v3/changes/startPageToken")
        .bearer_auth(&access_token)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json::<serde_json::Value>()
        .await
        .map_err(|e| e.to_string())?
        ["startPageToken"]
        .as_str()
        .map(|s| s.to_string());
    *state.changes_token.lock().await = start_token;

    // Mark connected
    {
        let mut s = state.status.lock().await;
        s.state   = SyncStatusKind::Connected;
        s.message = "Google Drive connected".to_string();
    }
    let _ = app_handle.emit("gdrive-status", state.status.lock().await.clone());
    log::info!("[GDrive] OAuth complete. Starting sync loop.");

    // Start background sync loop
    start_sync_loop(app_handle, state.inner().clone());

    Ok(true)
}

/// Disconnect from Google Drive and stop syncing.
#[tauri::command]
pub async fn cmd_gdrive_disconnect(
    app_handle: AppHandle,
    state: State<'_, GDriveState>,
) -> Result<bool, String> {
    // Stop background loop
    let tx = state.sync_shutdown.lock()
        .map_err(|e| e.to_string())?
        .take();
    if let Some(tx) = tx { let _ = tx.send(()); }

    *state.access_token.lock().await  = None;
    *state.refresh_token.lock().await = None;
    *state.token_expiry.lock().await  = 0;
    *state.changes_token.lock().await = None;

    let mut s = state.status.lock().await;
    s.state   = SyncStatusKind::Disconnected;
    s.message = "Disconnected".to_string();
    s.last_synced = None;
    s.files_synced = 0;
    drop(s);

    let _ = app_handle.emit("gdrive-status", state.status.lock().await.clone());
    log::info!("[GDrive] Disconnected.");
    Ok(true)
}

/// Returns the current sync status (for initial UI render and polling).
#[tauri::command]
pub async fn cmd_gdrive_sync_status(
    state: State<'_, GDriveState>,
) -> Result<SyncStatus, String> {
    Ok(state.status.lock().await.clone())
}

/// List files in a Google Drive folder (or My Drive root).
#[tauri::command]
pub async fn cmd_gdrive_list_files(
    folder_id: Option<String>,
    state: State<'_, GDriveState>,
) -> Result<Vec<GDriveFile>, String> {
    let token = get_valid_token(&state).await?;
    let http   = reqwest::Client::new();

    let parent = folder_id.as_deref().unwrap_or("root");
    let query  = format!("'{}' in parents and trashed = false", parent);

    let resp: DriveFileListResponse = http
        .get("https://www.googleapis.com/drive/v3/files")
        .bearer_auth(&token)
        .query(&[
            ("q",        query.as_str()),
            ("fields",   "nextPageToken,files(id,name,mimeType,size,modifiedTime,parents)"),
            ("pageSize", "200"),
        ])
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;

    let files = resp.files.unwrap_or_default().into_iter().map(|f| GDriveFile {
        id:            f.id,
        name:          f.name,
        mime_type:     f.mime_type,
        size:          f.size,
        modified_time: f.modified_time,
        parents:       f.parents,
    }).collect();

    Ok(files)
}

// ── Background Sync Loop ─────────────────────────────────────────

fn start_sync_loop(app_handle: AppHandle, state: GDriveState) {
    // Stop any existing loop first
    {
        let mut guard = state.sync_shutdown.lock().unwrap();
        if let Some(tx) = guard.take() { let _ = tx.send(()); }
    }

    let (tx, mut rx) = tokio::sync::oneshot::channel::<()>();
    *state.sync_shutdown.lock().unwrap() = Some(tx);

    tokio::spawn(async move {
        log::info!("[GDrive] Sync loop started.");
        loop {
            tokio::select! {
                _ = &mut rx => {
                    log::info!("[GDrive] Sync loop stopped.");
                    break;
                }
                _ = tokio::time::sleep(tokio::time::Duration::from_secs(10)) => {
                    if let Err(e) = run_sync_tick(&app_handle, &state).await {
                        log::error!("[GDrive] Sync tick error: {}", e);
                        let mut s = state.status.lock().await;
                        s.state   = SyncStatusKind::Error;
                        s.message = e.clone();
                        drop(s);
                        let _ = app_handle.emit("gdrive-status", state.status.lock().await.clone());
                        let _ = app_handle.emit("gdrive-sync-event", SyncEvent {
                            event_type: "error".to_string(),
                            file_name:  None,
                            message:    e,
                        });
                    }
                }
            }
        }
    });
}

async fn run_sync_tick(app_handle: &AppHandle, state: &GDriveState) -> Result<(), String> {
    let token = get_valid_token(state).await?;
    let page_token = {
        let t = state.changes_token.lock().await;
        t.clone().ok_or("No changes page token")?
    };

    let http = reqwest::Client::new();
    {
        let mut s = state.status.lock().await;
        s.state   = SyncStatusKind::Syncing;
        s.message = "Checking for changes…".to_string();
    }
    let _ = app_handle.emit("gdrive-status", state.status.lock().await.clone());

    let resp: DriveChangesResponse = http
        .get("https://www.googleapis.com/drive/v3/changes")
        .bearer_auth(&token)
        .query(&[
            ("pageToken",       page_token.as_str()),
            ("fields",          "newStartPageToken,nextPageToken,changes(fileId,removed,file(id,name,mimeType,size,modifiedTime,parents,trashed))"),
            ("includeRemoved",  "false"),
            ("spaces",          "drive"),
        ])
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json::<DriveChangesResponse>()
        .await
        .map_err(|e| e.to_string())?;

    // Update page token for next tick
    if let Some(nt) = resp.new_start_page_token {
        *state.changes_token.lock().await = Some(nt);
    } else if let Some(nt) = resp.next_page_token {
        *state.changes_token.lock().await = Some(nt);
    }

    let changes = resp.changes.unwrap_or_default();
    let new_files: Vec<DriveChangeFile> = changes
        .into_iter()
        .filter(|c| c.removed != Some(true))
        .filter_map(|c| c.file)
        .filter(|f| f.trashed != Some(true))
        // Skip Google Docs formats (not downloadable as raw binary)
        .filter(|f| !f.mime_type.starts_with("application/vnd.google-apps"))
        .collect();

    for file in &new_files {
        log::info!("[GDrive] Change detected: {} ({})", file.name, file.id);
        
        let _ = app_handle.emit("gdrive-status", SyncStatus {
            state: SyncStatusKind::Syncing,
            message: format!("Downloading '{}'…", file.name),
            last_synced: None,
            files_synced: state.files_synced.load(std::sync::atomic::Ordering::Relaxed),
        });

        // Actually sync the file
        match sync_file_to_telegram(app_handle, state, file).await {
            Ok(_) => {
                let _ = app_handle.emit("gdrive-sync-event", SyncEvent {
                    event_type: "file_synced".to_string(),
                    file_name:  Some(file.name.clone()),
                    message:    format!("Successfully synced '{}'", file.name),
                });
                state.files_synced.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            Err(e) => {
                log::error!("[GDrive] Failed to sync '{}': {}", file.name, e);
                let _ = app_handle.emit("gdrive-sync-event", SyncEvent {
                    event_type: "error".to_string(),
                    file_name:  Some(file.name.clone()),
                    message:    format!("Failed to sync '{}': {}", file.name, e),
                });
            }
        }
    }

    // Update status back to Connected
    let now = chrono::Local::now().format("%H:%M:%S").to_string();
    {
        let mut s = state.status.lock().await;
        s.state       = SyncStatusKind::Connected;
        s.message     = if new_files.is_empty() {
            "Up to date".to_string()
        } else {
            format!("{} file(s) synced", new_files.len())
        };
        s.last_synced = Some(now);
        s.files_synced = state.files_synced.load(std::sync::atomic::Ordering::Relaxed);
    }
    let _ = app_handle.emit("gdrive-status", state.status.lock().await.clone());

    Ok(())
}

use tauri::Manager;
use crate::TelegramState;
use grammers_client::InputMessage;

async fn sync_file_to_telegram(app_handle: &AppHandle, gdrive_state: &GDriveState, file: &DriveChangeFile) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;

    // 1. Download from Google Drive
    // Fetch a fresh token for EACH file in case a previous file took hours to upload.
    let gdrive_token = get_valid_token(gdrive_state).await?;
    
    let http = reqwest::Client::new();
    let download_url = format!("https://www.googleapis.com/drive/v3/files/{}?alt=media", file.id);
    let mut resp = http.get(&download_url)
        .bearer_auth(&gdrive_token)
        .send()
        .await
        .map_err(|e| format!("Download request failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("Google Drive returned {}", resp.status()));
    }

    // Use a temp file
    let temp_dir = std::env::temp_dir();
    let safe_name = file.name.replace('/', "_");
    let temp_path = temp_dir.join(format!("gdrive_sync_{}_{}", file.id, safe_name));
    
    {
        // FIX: Use tokio::fs::File to prevent blocking the async executor
        let mut temp_file = tokio::fs::File::create(&temp_path).await.map_err(|e| e.to_string())?;
        while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
            temp_file.write_all(&chunk).await.map_err(|e| e.to_string())?;
        }
    }

    let path_str = temp_path.to_str().unwrap().to_string();

    // 2. Upload to Telegram
    let tg_state = app_handle.state::<TelegramState>();
    let client_opt = tg_state.inner().client.lock().await.clone();
    
    if let Some(client) = client_opt {
        log::info!("[GDrive] Uploading '{}' to Telegram...", file.name);
        
        let client_clone = client.clone();
        let path_clone = path_str.clone();
        
        let uploaded_file = tauri::async_runtime::spawn(async move {
            client_clone.upload_file(&path_clone).await
        }).await.map_err(|e| format!("Task join error: {}", e))?
          .map_err(|e| e.to_string())?;
            
        let message = InputMessage::new().text("").file(uploaded_file);

        // Send to "Saved Messages" (Me)
        let peer = grammers_client::types::Peer::User(grammers_client::types::User::from_raw(
            grammers_tl_types::enums::User::User(grammers_tl_types::types::User {
                is_self: true,
                contact: true,
                mutual_contact: true,
                deleted: false,
                bot: false,
                bot_chat_history: false,
                bot_nochats: false,
                verified: false,
                restricted: false,
                min: false,
                bot_inline_geo: false,
                support: false,
                scam: false,
                fake: false,
                premium: false,
                attach_menu_bot: false,
                close_friend: false,
                stories_hidden: false,
                stories_unavailable: false,
                id: 0, // grammers automatically resolves 'me' for self uploads if we use self peer, but let's use the explicit saved messages logic
                access_hash: None,
                first_name: None,
                last_name: None,
                username: None,
                phone: None,
                photo: None,
                status: None,
                bot_info_version: None,
                restriction_reason: None,
                bot_inline_placeholder: None,
                lang_code: None,
                emoji_status: None,
                usernames: None,
                stories_max_id: None,
                color: None,
                profile_color: None,
            })
        ));

        // Wait, grammers has an easier way to get the "Me" peer:
        // Actually, we should just use `resolve_peer(None)`
        let peer = crate::commands::utils::resolve_peer(&client, None, &tg_state.peer_cache).await?;
        
        client.send_message(&peer, message).await.map_err(|e| e.to_string())?;
        log::info!("[GDrive] Successfully synced '{}' to Telegram.", file.name);
    } else {
        log::info!("[GDrive] [MOCK] Synced file {} to mock Telegram", file.name);
    }

    // 3. Cleanup temp file
    let _ = std::fs::remove_file(temp_path);

    Ok(())
}
