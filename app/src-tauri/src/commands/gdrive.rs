use tauri::{State, Emitter, AppHandle};
use tauri_plugin_opener::OpenerExt;
use serde::Deserialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::path::PathBuf;
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
    /// Re-entrancy guard for cmd_gdrive_connect
    pub connecting:     Arc<AtomicBool>,
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
    // Kept for completeness; we use `file.id` instead.
    #[allow(dead_code)]
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

// ── Token persistence helpers ────────────────────────────────────

const STORE_FILE: &str = "config.json";
const KEY_ACCESS:  &str = "gdrive_access_token";
const KEY_REFRESH: &str = "gdrive_refresh_token";
const KEY_EXPIRY:  &str = "gdrive_token_expiry";
const KEY_CHANGES: &str = "gdrive_changes_token";
const KEY_CLIENT_ID:     &str = "gdrive_client_id";
const KEY_CLIENT_SECRET: &str = "gdrive_client_secret";

async fn save_tokens(app_handle: &AppHandle, state: &GDriveState) -> Result<(), String> {
    use tauri_plugin_store::StoreExt;
    let store = app_handle.store(STORE_FILE).map_err(|e| e.to_string())?;
    if let Some(t) = state.access_token.lock().await.as_ref()  { store.set(KEY_ACCESS,  serde_json::Value::String(t.clone())); }
    if let Some(t) = state.refresh_token.lock().await.as_ref() { store.set(KEY_REFRESH, serde_json::Value::String(t.clone())); }
    let expiry = *state.token_expiry.lock().await;
    store.set(KEY_EXPIRY, serde_json::Value::from(expiry));
    if let Some(t) = state.changes_token.lock().await.as_ref() { store.set(KEY_CHANGES, serde_json::Value::String(t.clone())); }
    if let Some(t) = state.client_id.lock().await.as_ref()     { store.set(KEY_CLIENT_ID,     serde_json::Value::String(t.clone())); }
    if let Some(t) = state.client_secret.lock().await.as_ref() { store.set(KEY_CLIENT_SECRET, serde_json::Value::String(t.clone())); }
    store.save().map_err(|e| e.to_string())?;
    Ok(())
}

async fn delete_tokens(app_handle: &AppHandle) -> Result<(), String> {
    use tauri_plugin_store::StoreExt;
    let store = app_handle.store(STORE_FILE).map_err(|e| e.to_string())?;
    for k in [KEY_ACCESS, KEY_REFRESH, KEY_EXPIRY, KEY_CHANGES] {
        store.delete(k);
    }
    store.save().map_err(|e| e.to_string())?;
    Ok(())
}

/// Restore persisted tokens into in-memory state. If a refresh token is
/// available, also start the background sync loop.
#[tauri::command]
pub async fn cmd_gdrive_restore_tokens(
    app_handle: AppHandle,
    state: State<'_, GDriveState>,
) -> Result<bool, String> {
    use tauri_plugin_store::StoreExt;
    let store = app_handle.store(STORE_FILE).map_err(|e| e.to_string())?;

    let str_get = |k: &str| -> Option<String> {
        store.get(k).and_then(|v| v.as_str().map(|s| s.to_string()))
    };
    let u64_get = |k: &str| -> u64 {
        store.get(k).and_then(|v| v.as_u64()).unwrap_or(0)
    };

    let access  = str_get(KEY_ACCESS);
    let refresh = str_get(KEY_REFRESH);
    let changes = str_get(KEY_CHANGES);
    let cid     = str_get(KEY_CLIENT_ID);
    let csec    = str_get(KEY_CLIENT_SECRET);
    let expiry  = u64_get(KEY_EXPIRY);

    *state.access_token.lock().await   = access;
    *state.refresh_token.lock().await  = refresh.clone();
    *state.token_expiry.lock().await   = expiry;
    *state.changes_token.lock().await  = changes;
    if cid.is_some()  { *state.client_id.lock().await     = cid; }
    if csec.is_some() { *state.client_secret.lock().await = csec; }

    if refresh.is_some() {
        {
            let mut s = state.status.lock().await;
            s.state   = SyncStatusKind::Connected;
            s.message = "Restored from saved session".to_string();
        }
        let _ = app_handle.emit("gdrive-status", state.status.lock().await.clone());
        start_sync_loop(app_handle, state.inner().clone());
        Ok(true)
    } else {
        Ok(false)
    }
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
    let expiry = unix_now().saturating_add(resp.expires_in).saturating_sub(60);
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

    // Re-entrancy guard — bail if a previous flow is still running.
    if state.connecting.swap(true, Ordering::SeqCst) {
        return Err("Connection already in progress".to_string());
    }
    struct Reset(Arc<AtomicBool>);
    impl Drop for Reset { fn drop(&mut self) { self.0.store(false, Ordering::SeqCst); } }
    let _guard = Reset(state.connecting.clone());

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

    // CSRF: generate a 32-byte URL-safe random state nonce
    let state_nonce: String = {
        use rand::Rng;
        let mut rng = rand::thread_rng();
        (0..32).map(|_| rng.gen::<u8>()).map(|b| format!("{:02x}", b)).collect()
    };

    let auth_url = format!(
        "https://accounts.google.com/o/oauth2/v2/auth?\
         client_id={client_id}\
         &redirect_uri={redirect_uri}\
         &response_type=code\
         &scope=https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fdrive.readonly\
         &access_type=offline\
         &prompt=consent\
         &state={state_nonce}",
        client_id = urlencoding::encode(&client_id),
        redirect_uri = urlencoding::encode(&redirect_uri),
        state_nonce = urlencoding::encode(&state_nonce),
    );

    // Update status
    {
        let mut s = state.status.lock().await;
        s.state = SyncStatusKind::Connecting;
        s.message = "Waiting for Google sign-in…".to_string();
    }
    let _ = app_handle.emit("gdrive-status", state.status.lock().await.clone());

    // Open browser via opener plugin (shell.open is deprecated).
    let _ = app_handle.opener().open_url(&auth_url, None::<&str>);

    // Loop accept() until we see ?code= or ?error= in the request line.
    use std::time::Duration;
    let (code, _returned_state) = tokio::time::timeout(Duration::from_secs(300), async {
        loop {
            let (mut socket, _) = listener.accept().await
                .map_err(|e| format!("OAuth listener accept failed: {e}"))?;
            let mut reader = BufReader::new(&mut socket);
            let mut request_line = String::new();
            if reader.read_line(&mut request_line).await.is_err() {
                continue;
            }

            let path = match request_line.split_whitespace().nth(1) {
                Some(p) => p,
                None => continue,
            };
            let url = match url::Url::parse(&format!("http://localhost{}", path)) {
                Ok(u) => u,
                Err(_) => continue,
            };

            let mut code_opt: Option<String> = None;
            let mut error_opt: Option<String> = None;
            let mut state_opt: Option<String> = None;
            for (k, v) in url.query_pairs() {
                match k.as_ref() {
                    "code"  => code_opt  = Some(v.into_owned()),
                    "error" => error_opt = Some(v.into_owned()),
                    "state" => state_opt = Some(v.into_owned()),
                    _ => {}
                }
            }

            if let Some(err) = error_opt {
                let body = "HTTP/1.1 400 Bad Request\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\
                     <html><body style='font-family:sans-serif;text-align:center;padding:60px'>\
                     <h2>\u{274C} Google sign-in cancelled</h2>\
                     <p>You can close this tab and return to OmniSync.</p></body></html>";
                let _ = socket.write_all(body.as_bytes()).await;
                return Err::<(String, String), String>(format!("Google OAuth error: {err}"));
            }

            if let Some(c) = code_opt {
                let body = "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\
                     <html><body style='font-family:sans-serif;text-align:center;padding:60px'>\
                     <h2>\u{2705} Connected to Google Drive</h2>\
                     <p>You can close this tab and return to OmniSync.</p></body></html>";
                let _ = socket.write_all(body.as_bytes()).await;
                return Ok::<(String, String), String>((c, state_opt.unwrap_or_default()));
            }

            // Stray request (favicon, prefetch, …) — keep waiting.
            let _ = socket.write_all(b"HTTP/1.1 404 Not Found\r\n\r\n").await;
        }
    })
    .await
    .map_err(|_| "OAuth timed out after 5 minutes".to_string())??;

    if _returned_state != state_nonce {
        return Err("OAuth state mismatch — possible CSRF".to_string());
    }

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

    let expiry = unix_now().saturating_add(token_resp.expires_in).saturating_sub(60);
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

    // Persist for next launch (best-effort).
    if let Err(e) = save_tokens(&app_handle, &state).await {
        log::warn!("[GDrive] Failed to persist tokens: {}", e);
    }

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
        .unwrap_or_else(|e| e.into_inner())
        .take();
    if let Some(tx) = tx { let _ = tx.send(()); }

    // Best-effort revoke at Google before discarding the token locally.
    let access_for_revoke = state.access_token.lock().await.clone();
    if let Some(tok) = access_for_revoke {
        let http = reqwest::Client::new();
        let _ = http
            .post(format!("https://oauth2.googleapis.com/revoke?token={}", urlencoding::encode(&tok)))
            .send()
            .await;
    }

    *state.access_token.lock().await  = None;
    *state.refresh_token.lock().await = None;
    *state.token_expiry.lock().await  = 0;
    *state.changes_token.lock().await = None;

    // Clear persisted copies.
    let _ = delete_tokens(&app_handle).await;

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
    if parent != "root" && !parent.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err("Invalid folder_id".to_string());
    }
    let query  = format!("'{}' in parents and trashed = false", parent);

    const MAX_TOTAL: usize = 5000;
    let mut all: Vec<GDriveFile> = Vec::new();
    let mut page_token: Option<String> = None;

    loop {
        let mut q = vec![
            ("q",        query.as_str()),
            ("fields",   "nextPageToken,files(id,name,mimeType,size,modifiedTime,parents)"),
            ("pageSize", "200"),
        ];
        if let Some(pt) = page_token.as_deref() {
            q.push(("pageToken", pt));
        }
        let resp: DriveFileListResponse = http
            .get("https://www.googleapis.com/drive/v3/files")
            .bearer_auth(&token)
            .query(&q)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;

        for f in resp.files.unwrap_or_default() {
            all.push(GDriveFile {
                id:            f.id,
                name:          f.name,
                mime_type:     f.mime_type,
                size:          f.size,
                modified_time: f.modified_time,
                parents:       f.parents,
            });
            if all.len() >= MAX_TOTAL { return Ok(all); }
        }

        match resp.next_page_token {
            Some(nt) if !nt.is_empty() => page_token = Some(nt),
            _ => break,
        }
    }

    Ok(all)
}

// ── Background Sync Loop ─────────────────────────────────────────

fn start_sync_loop(app_handle: AppHandle, state: GDriveState) {
    // Stop any existing loop first
    {
        let mut guard = state.sync_shutdown.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(tx) = guard.take() { let _ = tx.send(()); }
    }

    let (tx, mut rx) = tokio::sync::oneshot::channel::<()>();
    *state.sync_shutdown.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);

    tokio::spawn(async move {
        log::info!("[GDrive] Sync loop started.");
        // Exponential backoff state. Resets to base on success.
        const BASE_DELAY_SECS: u64 = 10;
        const MAX_DELAY_SECS: u64 = 300; // 5 min cap
        const KILL_AFTER_FAILURES: u32 = 30; // ~give up after sustained failure
        let mut delay_secs = BASE_DELAY_SECS;
        let mut consecutive_failures: u32 = 0;

        loop {
            tokio::select! {
                _ = &mut rx => {
                    log::info!("[GDrive] Sync loop stopped.");
                    break;
                }
                _ = tokio::time::sleep(tokio::time::Duration::from_secs(delay_secs)) => {
                    match run_sync_tick(&app_handle, &state).await {
                        Ok(()) => {
                            // Success — reset backoff.
                            delay_secs = BASE_DELAY_SECS;
                            consecutive_failures = 0;
                        }
                        Err(e) => {
                            consecutive_failures += 1;
                            log::error!("[GDrive] Sync tick error ({}): {}", consecutive_failures, e);
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

                            if consecutive_failures >= KILL_AFTER_FAILURES {
                                log::error!("[GDrive] Too many consecutive failures ({}). Stopping sync loop.", consecutive_failures);
                                let final_msg = "Sync stopped after repeated failures. Please reconnect to retry.".to_string();
                                {
                                    let mut s = state.status.lock().await;
                                    s.state   = SyncStatusKind::Error;
                                    s.message = final_msg.clone();
                                }
                                let _ = app_handle.emit("gdrive-status", state.status.lock().await.clone());
                                let _ = app_handle.emit("gdrive-sync-event", SyncEvent {
                                    event_type: "error".to_string(),
                                    file_name:  None,
                                    message:    final_msg,
                                });
                                break;
                            }

                            // Exponential backoff: 10s, 20s, 40s, 80s, 160s, 300s...
                            delay_secs = (delay_secs.saturating_mul(2)).min(MAX_DELAY_SECS);
                        }
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

    // Defer page-token advancement until AFTER per-file syncs in this batch
    // complete. Otherwise a crash mid-batch loses files we never re-fetch.
    let pending_next_token: Option<String> = resp
        .new_start_page_token
        .or(resp.next_page_token);

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

        // Preserve the previously-shown last_synced timestamp so the UI doesn't
        // blank it out mid-sync.
        let prev_last_synced = state.status.lock().await.last_synced.clone();
        let _ = app_handle.emit("gdrive-status", SyncStatus {
            state: SyncStatusKind::Syncing,
            message: format!("Downloading '{}'…", file.name),
            last_synced: prev_last_synced,
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

    // All files in this batch processed — now safe to advance the page token.
    if let Some(nt) = pending_next_token {
        *state.changes_token.lock().await = Some(nt);
        // Persist so a crash before the next tick doesn't replay the batch.
        if let Err(e) = save_tokens(app_handle, state).await {
            log::warn!("[GDrive] Failed to persist changes_token: {}", e);
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

    /// RAII guard — removes the temp file on drop, even on early-return errors.
    struct TempFileGuard(PathBuf);
    impl Drop for TempFileGuard {
        fn drop(&mut self) {
            let p = self.0.clone();
            // Best-effort async cleanup; ignore if runtime is gone.
            tokio::spawn(async move { let _ = tokio::fs::remove_file(&p).await; });
        }
    }

    // 1. Download from Google Drive
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

    // Use a temp file. Suffix with monotonic timestamp so concurrent revisions
    // of the same file.id don't clobber each other.
    let temp_dir = std::env::temp_dir();
    let safe_name = file.name.replace('/', "_");
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let temp_path = temp_dir.join(format!("gdrive_sync_{}_{}_{}", file.id, stamp, safe_name));
    let _temp_guard = TempFileGuard(temp_path.clone());

    {
        let mut temp_file = tokio::fs::File::create(&temp_path).await.map_err(|e| e.to_string())?;
        while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
            temp_file.write_all(&chunk).await.map_err(|e| e.to_string())?;
        }
    }

    let path_str = temp_path.to_str()
        .ok_or_else(|| format!("Temp path is not valid UTF-8: {:?}", temp_path))?
        .to_string();

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

        let peer = crate::commands::utils::resolve_peer(&client, None, &tg_state.peer_cache).await?;

        client.send_message(&peer, message).await.map_err(|e| e.to_string())?;
        log::info!("[GDrive] Successfully synced '{}' to Telegram.", file.name);
    } else {
        log::info!("[GDrive] [MOCK] Synced file {} to mock Telegram", file.name);
    }

    // Temp file cleaned up by TempFileGuard drop.
    Ok(())
}
