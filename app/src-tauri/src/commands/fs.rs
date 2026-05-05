use tauri::{State, Emitter};
use grammers_client::types::{Media, Peer};
use grammers_client::InputMessage;
use grammers_tl_types as tl;
use std::sync::atomic::{AtomicU64, Ordering};
use crate::TelegramState;
use crate::models::{FolderMetadata, FileMetadata};
use crate::bandwidth::{BandwidthManager, TransferKind};
use crate::commands::utils::{resolve_peer, map_error};

/// Monotonic counter ensures unique mock folder ids even when called within the same second.
static MOCK_FOLDER_SEQ: AtomicU64 = AtomicU64::new(0);

#[tauri::command]
pub async fn cmd_create_folder(
    name: String,
    state: State<'_, TelegramState>,
) -> Result<FolderMetadata, String> {
    let client_opt = {
        state.client.lock().await.clone()
    };

    // --- MOCK ---
    if client_opt.is_none() {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        // Combine timestamp with monotonic counter to avoid collisions.
        let seq = MOCK_FOLDER_SEQ.fetch_add(1, Ordering::Relaxed);
        let mock_id = ((secs as i64) << 16) | (seq as i64 & 0xFFFF);
        log::info!("[MOCK] Created folder '{}' with ID {}", name, mock_id);
        return Ok(FolderMetadata {
            id: mock_id,
            name,
            parent_id: None,
        });
    }
    // -----------
    let client = client_opt.unwrap();
    log::info!("Creating Telegram Channel: {}", name);
    
    let result = client.invoke(&tl::functions::channels::CreateChannel {
        broadcast: true,
        megagroup: false,
        title: format!("{} [AD]", name),
        about: "OmniSync Storage Folder\n[antigravity-drive-folder]".to_string(),
        geo_point: None,
        address: None,
        for_import: false,
        forum: false,
        ttl_period: None, // Initial creation TTL
    }).await.map_err(map_error)?;
    
    let (chat_id, access_hash) = match result {
        tl::enums::Updates::Updates(u) => {
             // Find the first Channel chat (CreateChannel may include other
             // bookkeeping chats in the response).
             let channel = u.chats.iter().find_map(|c| match c {
                 tl::enums::Chat::Channel(ch) => Some(ch),
                 _ => None,
             }).ok_or("No channel in updates")?;
             (channel.id, channel.access_hash.unwrap_or(0))
        },
        _ => return Err("Unexpected response (not Updates::Updates)".to_string()), 
    };

    // Explicitly Disable TTL
    let _input_channel = tl::enums::InputChannel::Channel(tl::types::InputChannel {
         channel_id: chat_id,
         access_hash,
    });

    let _ = client.invoke(&tl::functions::messages::SetHistoryTtl {
        peer: tl::enums::InputPeer::Channel(tl::types::InputPeerChannel { channel_id: chat_id, access_hash }),
        period: 0, 
    }).await;

    Ok(FolderMetadata {
        id: chat_id,
        name,
        parent_id: None,
    })
}

#[tauri::command]
pub async fn cmd_delete_folder(
    folder_id: i64,
    state: State<'_, TelegramState>,
) -> Result<bool, String> {
    let client_opt = {
        state.client.lock().await.clone()
    };
    
    if client_opt.is_none() {
        log::info!("[MOCK] Deleted folder ID {}", folder_id);
        return Ok(true);
    }
    let client = client_opt.unwrap();
    log::info!("Deleting folder/channel: {}", folder_id);

    let peer = resolve_peer(&client, Some(folder_id), &state.peer_cache).await?;
    
    let input_channel = match peer {
        Peer::Channel(c) => {
             let chan = &c.raw;
             tl::enums::InputChannel::Channel(tl::types::InputChannel {
                 channel_id: chan.id,
                 access_hash: chan.access_hash.ok_or("No access hash for channel")?,
             })
        },
        _ => return Err("Only channels (folders) can be deleted.".to_string()),
    };
    
    client.invoke(&tl::functions::channels::DeleteChannel {
        channel: input_channel,
    }).await.map_err(|e| format!("Failed to delete channel: {}", e))?;
    
    Ok(true)
}


#[derive(Clone, serde::Serialize)]
struct ProgressPayload {
    id: String,
    percent: u8,
}

#[tauri::command]
pub async fn cmd_upload_file(
    path: String,
    folder_id: Option<i64>,
    transfer_id: Option<String>,
    app_handle: tauri::AppHandle,
    state: State<'_, TelegramState>,
    bw_state: State<'_, BandwidthManager>,
) -> Result<String, String> {
    let size = tokio::fs::metadata(&path).await.map_err(|e| e.to_string())?.len();
    bw_state.try_reserve(size, TransferKind::Up)?;

    let tid = transfer_id.unwrap_or_default();

    let client_opt = { state.client.lock().await.clone() };
    if client_opt.is_none() {
        log::info!("[MOCK] Uploaded file {} to {:?}", path, folder_id);
        return Ok("Mock upload successful".to_string());
    }
    let client = client_opt.unwrap();
    
    // Emit start progress
    if !tid.is_empty() {
        let _ = app_handle.emit("upload-progress", ProgressPayload { id: tid.clone(), percent: 0 });
    }

    let path_clone = path.clone();
    let client_clone = client.clone();
    
    let uploaded_file = match tauri::async_runtime::spawn(async move {
        client_clone.upload_file(&path_clone).await
    }).await.map_err(|e| format!("Task join error: {}", e))? {
        Ok(f) => f,
        Err(e) => {
            bw_state.refund(size, TransferKind::Up);
            return Err(map_error(e));
        }
    };

    let message = InputMessage::new().text("").file(uploaded_file);

    let peer = match resolve_peer(&client, folder_id, &state.peer_cache).await {
        Ok(p) => p,
        Err(e) => { bw_state.refund(size, TransferKind::Up); return Err(e); }
    };

    if let Err(e) = client.send_message(&peer, message).await {
        bw_state.refund(size, TransferKind::Up);
        return Err(map_error(e));
    }

    // Emit completion (file >10MB also receives a 50% mid-event below for UX).
    // Mid-progress hint for large files (cheap fallback to true streaming progress).
    if !tid.is_empty() && size > 10 * 1024 * 1024 {
        let _ = app_handle.emit("upload-progress", ProgressPayload { id: tid.clone(), percent: 50 });
    }

    if !tid.is_empty() {
        let _ = app_handle.emit("upload-progress", ProgressPayload { id: tid, percent: 100 });
    }

    Ok("File uploaded successfully".to_string())
}

#[tauri::command]
pub async fn cmd_delete_file(
    message_id: i32,
    folder_id: Option<i64>,
    state: State<'_, TelegramState>,
) -> Result<bool, String> {
    let client_opt = { state.client.lock().await.clone() };
    if client_opt.is_none() { 
         log::info!("[MOCK] Deleted message {} from folder {:?}", message_id, folder_id);
        return Ok(true); 
    }
    let client = client_opt.unwrap();

    let peer = resolve_peer(&client, folder_id, &state.peer_cache).await?;
    client.delete_messages(&peer, &[message_id]).await.map_err(|e| e.to_string())?;
    Ok(true)
}

#[tauri::command]
pub async fn cmd_download_file(
    message_id: i32,
    save_path: String,
    folder_id: Option<i64>,
    transfer_id: Option<String>,
    app_handle: tauri::AppHandle,
    state: State<'_, TelegramState>,
    bw_state: State<'_, BandwidthManager>,
) -> Result<String, String> {
    let tid = transfer_id.unwrap_or_default();

    let client_opt = { state.client.lock().await.clone() };
    if client_opt.is_none() { 
        log::info!("[MOCK] Downloaded message {} from {:?} to {}", message_id, folder_id, save_path);
        if let Err(e) = std::fs::write(&save_path, b"Mock Content") { return Err(e.to_string()); }
        return Ok("Download successful".to_string());
    }
    let client = client_opt.unwrap();
    
    let peer = resolve_peer(&client, folder_id, &state.peer_cache).await?;

    // Use get_messages_by_id for efficient message lookup (same as server.rs)
    let messages = client.get_messages_by_id(&peer, &[message_id]).await.map_err(|e| e.to_string())?;
    
    let msg = messages.into_iter()
        .flatten()
        .next()
        .ok_or_else(|| "Message not found".to_string())?;

    let media = msg.media()
        .ok_or_else(|| "No media in message".to_string())?;

    let total_size = match &media {
        Media::Document(d) => d.size() as u64,
        Media::Photo(_) => 1024 * 1024,
        _ => 0,
    };
    
    bw_state.try_reserve(total_size, TransferKind::Down)?;

    // Emit start
    if !tid.is_empty() {
        let _ = app_handle.emit("download-progress", ProgressPayload { id: tid.clone(), percent: 0 });
    }

    // Stream download with per-chunk progress (async I/O — never block the runtime)
    use tokio::io::AsyncWriteExt;
    let mut download_iter = client.iter_download(&media);
    let mut file = match tokio::fs::File::create(&save_path).await {
        Ok(f) => f,
        Err(e) => { bw_state.refund(total_size, TransferKind::Down); return Err(e.to_string()); }
    };
    let mut downloaded: u64 = 0;
    let mut last_percent: u8 = 0;

    while let Some(chunk) = download_iter.next().await.transpose() {
        let bytes = match chunk {
            Ok(b) => b,
            Err(e) => {
                // Refund the bytes we never got.
                bw_state.refund(total_size.saturating_sub(downloaded), TransferKind::Down);
                return Err(format!("Download chunk error: {}", e));
            }
        };
        if let Err(e) = file.write_all(&bytes).await {
            bw_state.refund(total_size.saturating_sub(downloaded), TransferKind::Down);
            return Err(e.to_string());
        }
        downloaded += bytes.len() as u64;

        if !tid.is_empty() && total_size > 0 {
            let percent = ((downloaded as f64 / total_size as f64) * 100.0).min(100.0) as u8;
            if percent != last_percent {
                last_percent = percent;
                let _ = app_handle.emit("download-progress", ProgressPayload { id: tid.clone(), percent });
            }
        }
    }
    if let Err(e) = file.flush().await {
        bw_state.refund(total_size.saturating_sub(downloaded), TransferKind::Down);
        return Err(e.to_string());
    }

    // Refund any over-reserved bytes (declared size > actual).
    if downloaded < total_size {
        bw_state.refund(total_size - downloaded, TransferKind::Down);
    }

    // Emit completion
    if !tid.is_empty() {
        let _ = app_handle.emit("download-progress", ProgressPayload { id: tid, percent: 100 });
    }

    Ok("Download successful".to_string())
}

/// Retry the given async fn up to `max_attempts` times if Telegram returns
/// FLOOD_WAIT_<n>, sleeping n+1 seconds between attempts.
async fn retry_flood_wait<T, F, Fut>(max_attempts: u32, mut op: F) -> Result<T, String>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, String>>,
{
    let mut last_err = String::new();
    for _ in 0..max_attempts {
        match op().await {
            Ok(v) => return Ok(v),
            Err(e) => {
                if let Some(rest) = e.strip_prefix("FLOOD_WAIT_") {
                    if let Ok(n) = rest.parse::<u64>() {
                        log::warn!("FLOOD_WAIT detected ({}s); sleeping then retrying", n);
                        tokio::time::sleep(tokio::time::Duration::from_secs(n.saturating_add(1))).await;
                        last_err = e;
                        continue;
                    }
                }
                return Err(e);
            }
        }
    }
    Err(last_err)
}

#[tauri::command]
pub async fn cmd_move_files(
    message_ids: Vec<i32>,
    source_folder_id: Option<i64>,
    target_folder_id: Option<i64>,
    state: State<'_, TelegramState>,
) -> Result<u32, String> {
    if source_folder_id == target_folder_id { return Ok(message_ids.len() as u32); }
    let client_opt = { state.client.lock().await.clone() };
    if client_opt.is_none() {
        log::info!("[MOCK] Moved msgs {:?} from {:?} to {:?}", message_ids, source_folder_id, target_folder_id);
        return Ok(message_ids.len() as u32);
    }
    let client = client_opt.unwrap();

    let source_peer = resolve_peer(&client, source_folder_id, &state.peer_cache).await?;
    let target_peer = resolve_peer(&client, target_folder_id, &state.peer_cache).await?;

    let ids = message_ids.clone();
    retry_flood_wait(3, || {
        let client = client.clone();
        let target_peer = target_peer.clone();
        let source_peer = source_peer.clone();
        let ids = ids.clone();
        async move {
            client.forward_messages(&target_peer, &ids, &source_peer).await
                .map(|_| ())
                .map_err(map_error)
        }
    }).await.map_err(|e| format!("Forward failed: {}", e))?;

    let forwarded = message_ids.len() as u32;

    // Soft-success: deletion is best-effort. Report forwarded count even if delete fails.
    let ids2 = message_ids.clone();
    if let Err(e) = retry_flood_wait(3, || {
        let client = client.clone();
        let source_peer = source_peer.clone();
        let ids = ids2.clone();
        async move {
            client.delete_messages(&source_peer, &ids).await
                .map(|_| ())
                .map_err(map_error)
        }
    }).await {
        log::warn!("Move: forwarded {} message(s) but delete from source failed: {}", forwarded, e);
    }

    Ok(forwarded)
}

#[tauri::command]
pub async fn cmd_get_files(
    folder_id: Option<i64>,
    state: State<'_, TelegramState>,
) -> Result<Vec<FileMetadata>, String> {
    let client_opt = { state.client.lock().await.clone() };
    if client_opt.is_none() { 
        log::info!("[MOCK] Returning mock files for folder {:?}", folder_id);
        return Ok(Vec::new()); // No mock files for now
    }
    let client = client_opt.unwrap();
    let mut files = Vec::new();
    
    let peer = resolve_peer(&client, folder_id, &state.peer_cache).await?;

    let mut msgs = client.iter_messages(&peer).limit(500); // TODO: paginate for folders >500 files
    while let Some(msg) = msgs.next().await.map_err(|e| e.to_string())? {
        if let Some(doc) = msg.media() {
            let (name, size, mime, ext) = match doc {
                Media::Document(d) => {
                    let n = d.name().to_string();
                    let s = d.size();
                    let m = d.mime_type().map(|s| s.to_string());
                    let e = std::path::Path::new(&n).extension().map(|os| os.to_str().unwrap_or("").to_string());
                    (n, s, m, e)
                },
                Media::Photo(_) => ("Photo.jpg".to_string(), 0, Some("image/jpeg".into()), Some("jpg".into())),
                _ => ("Unknown".to_string(), 0, None, None),
            };
            files.push(FileMetadata {
                id: msg.id() as i64, folder_id, name, size: size as u64, mime_type: mime, file_ext: ext, created_at: msg.date().to_string(), icon_type: "file".into()
            });
        }
    }

    Ok(files)
}

#[tauri::command]
pub async fn cmd_search_global(
    query: String,
    state: State<'_, TelegramState>,
) -> Result<Vec<FileMetadata>, String> {
    let client_opt = { state.client.lock().await.clone() };
    if client_opt.is_none() { 
        return Ok(Vec::new());
    }
    let client = client_opt.unwrap();
    let mut files = Vec::new();
    
    log::info!("Searching global for: {}", query);

    let result = client.invoke(&tl::functions::messages::SearchGlobal {
        q: query,
        filter: tl::enums::MessagesFilter::InputMessagesFilterDocument,
        min_date: 0,
        max_date: 0,
        offset_rate: 0,
        offset_peer: tl::enums::InputPeer::Empty,
        offset_id: 0,
        limit: 50,
        folder_id: None,
        broadcasts_only: false,
        groups_only: false,
        users_only: false,
    }).await.map_err(map_error)?;

    let messages_vec: Vec<tl::enums::Message> = match result {
        tl::enums::messages::Messages::Messages(m) => m.messages,
        tl::enums::messages::Messages::Slice(m)    => m.messages,
        tl::enums::messages::Messages::ChannelMessages(m) => m.messages,
        tl::enums::messages::Messages::NotModified(_) => Vec::new(),
    };

    for msg in messages_vec {
        let tl::enums::Message::Message(m) = msg else { continue };
        let Some(tl::enums::MessageMedia::Document(d)) = m.media else { continue };
        let Some(tl::enums::Document::Document(doc)) = d.document else { continue };

        let name = doc.attributes.iter().find_map(|a| match a {
            tl::enums::DocumentAttribute::Filename(f) => Some(f.file_name.clone()),
            _ => None,
        }).unwrap_or_else(|| "Unknown".to_string());
        let size = doc.size as u64;
        let mime = doc.mime_type.clone();
        let ext  = std::path::Path::new(&name)
            .extension()
            .map(|os| os.to_string_lossy().to_string());
        let folder_id = match m.peer_id {
            tl::enums::Peer::Channel(c) => Some(c.channel_id),
            tl::enums::Peer::User(u)    => Some(u.user_id),
            tl::enums::Peer::Chat(c)    => Some(c.chat_id),
        };
        files.push(FileMetadata {
            id: m.id as i64,
            folder_id,
            name,
            size,
            mime_type: Some(mime),
            file_ext: ext,
            created_at: m.date.to_string(),
            icon_type: "file".into(),
        });
    }

    Ok(files)
}

#[tauri::command]
pub async fn cmd_scan_folders(
    state: State<'_, TelegramState>,
) -> Result<Vec<FolderMetadata>, String> {
    let client_opt = { state.client.lock().await.clone() };
    if client_opt.is_none() {
        return Ok(Vec::new());
    }
    let client = client_opt.unwrap();

    let mut folders = Vec::new();
    let mut dialogs = client.iter_dialogs();

    log::info!("Starting Folder Scan...");

    // Harvest peers locally without holding the cache lock across awaits.
    let mut harvested: Vec<(i64, Peer)> = Vec::new();

    while let Some(dialog) = dialogs.next().await.map_err(|e| e.to_string())? {
        match &dialog.peer {
            Peer::Channel(c) => {
                let id = c.raw.id;
                harvested.push((id, dialog.peer.clone()));

                let name = c.raw.title.clone();
                let access_hash = c.raw.access_hash.unwrap_or(0);

                log::debug!("[SCAN] Processing Channel: '{}' (ID: {})", name, id);

                // Detect folders solely via the `[antigravity-drive-folder]` marker
                // in the channel "about" text. Title heuristics produce false
                // positives for unrelated chats containing "[ad]".
                let input_chan = tl::enums::InputChannel::Channel(tl::types::InputChannel {
                    channel_id: c.raw.id,
                    access_hash,
                });

                match client.invoke(&tl::functions::channels::GetFullChannel {
                    channel: input_chan,
                }).await {
                    Ok(tl::enums::messages::ChatFull::Full(f)) => {
                        if let tl::enums::ChatFull::Full(cf) = f.full_chat {
                            if cf.about.contains("[antigravity-drive-folder]") {
                                log::info!(" -> MATCH via About: {}", name);
                                let display_name = name
                                    .replace(" [AD]", "")
                                    .replace(" [ad]", "")
                                    .replace("[AD]", "")
                                    .replace("[ad]", "")
                                    .trim()
                                    .to_string();
                                folders.push(FolderMetadata { id, name: display_name, parent_id: None });
                            }
                        }
                    },
                    Err(e) => log::warn!(" -> Failed to get full info: {}", e),
                }
            },
            Peer::User(u) => {
                harvested.push((u.raw.id(), dialog.peer.clone()));
                log::debug!("[SCAN] Cached User Peer: {}", u.raw.id());
            },
            peer => {
                log::debug!("[SCAN] Skipped Peer: {:?}", peer);
            }
        }
    }

    // Bulk insert into the peer cache after iteration completes.
    let cache_size = {
        let mut peer_cache = state.peer_cache.write().await;
        for (id, peer) in harvested {
            peer_cache.entry(id).or_insert(peer);
        }
        peer_cache.len()
    };

    log::info!("Scan complete. Found {} folders. Peer cache size: {}.", folders.len(), cache_size);
    Ok(folders)
}
