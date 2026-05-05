use actix_web::{get, web, App, HttpServer, HttpRequest, HttpResponse, Responder};
use actix_cors::Cors;
use crate::commands::TelegramState;
use crate::commands::utils::resolve_peer;
use grammers_client::types::Media;

use std::sync::Arc;
use tokio::sync::RwLock;

const STREAM_CHUNK: i32 = 512 * 1024; // grammers MAX_CHUNK_SIZE

/// Holds the per-session streaming token for Actix validation.
/// Wrapped so callers can rotate the token at runtime; Actix sees the new
/// value lazily on the next request.
pub struct StreamTokenData {
    pub token: Arc<RwLock<String>>,
}

#[derive(serde::Deserialize)]
struct StreamQuery {
    token: Option<String>,
}

/// Parse a single-range `Range: bytes=start-end?` header into (start, end_inclusive_opt).
fn parse_range(h: &str, total: u64) -> Option<(u64, Option<u64>)> {
    let v = h.strip_prefix("bytes=")?;
    let (s, e) = v.split_once('-')?;
    let start: u64 = s.trim().parse().ok()?;
    let end = e.trim();
    let end_opt = if end.is_empty() { None } else { end.parse::<u64>().ok() };
    if start >= total {
        return None;
    }
    Some((start, end_opt))
}

#[get("/stream/{folder_id}/{message_id}")]
async fn stream_media(
    req: HttpRequest,
    path: web::Path<(String, i32)>,
    query: web::Query<StreamQuery>,
    data: web::Data<Arc<TelegramState>>,
    token_data: web::Data<StreamTokenData>,
) -> impl Responder {
    let (folder_id_str, message_id) = path.into_inner();

    // Validate session token (also accept Authorization: Bearer ...)
    let header_token = req
        .headers()
        .get(actix_web::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_owned);
    let provided = query.token.clone().or(header_token);
    let expected = token_data.token.read().await.clone();
    match provided {
        Some(t) if t == expected => {}
        _ => {
            log::error!("Stream request failed: Invalid or missing stream token for msg {}", message_id);
            return HttpResponse::Forbidden().body("Invalid or missing stream token");
        }
    }

    // Parse folder ID
    let folder_id = if folder_id_str == "me" || folder_id_str == "home" || folder_id_str == "null" {
        None
    } else {
        match folder_id_str.parse::<i64>() {
            Ok(id) => Some(id),
            Err(_) => return HttpResponse::BadRequest().body("Invalid folder ID"),
        }
    };

    let client_opt = { data.client.lock().await.clone() };
    let Some(client) = client_opt else {
        return HttpResponse::ServiceUnavailable().body("Telegram client not connected");
    };

    let peer = match resolve_peer(&client, folder_id, &data.peer_cache).await {
        Ok(p) => p,
        Err(e) => return HttpResponse::BadRequest().body(format!("Peer resolution failed: {}", e)),
    };

    let messages = match client.get_messages_by_id(peer, &[message_id]).await {
        Ok(m) => m,
        Err(e) => return HttpResponse::InternalServerError().body(format!("Failed to fetch message: {}", e)),
    };

    let Some(Some(msg)) = messages.first() else {
        return HttpResponse::NotFound().body("Message not found");
    };
    let Some(media) = msg.media() else {
        return HttpResponse::NotFound().body("Media not found");
    };

    let total: u64 = match &media {
        Media::Document(d) => d.size().max(0) as u64,
        _ => 0,
    };
    let mime = mime_type_from_media(&media);

    // Parse Range
    let range_hdr = req
        .headers()
        .get(actix_web::http::header::RANGE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);

    let (status_code, content_range_hdr, start, length) = if total == 0 {
        // Photos / unknown size: stream whole thing, no ranges.
        (200u16, None, 0u64, None::<u64>)
    } else if let Some(h) = range_hdr.as_deref().and_then(|h| parse_range(h, total)) {
        let (s, e_opt) = h;
        let end = e_opt.unwrap_or(total - 1).min(total - 1);
        if end < s {
            return HttpResponse::RangeNotSatisfiable()
                .insert_header(("Content-Range", format!("bytes */{}", total)))
                .finish();
        }
        let len = end - s + 1;
        (
            206u16,
            Some(format!("bytes {}-{}/{}", s, end, total)),
            s,
            Some(len),
        )
    } else {
        (200u16, None, 0u64, Some(total))
    };

    // Compute chunk skipping.
    let chunk_size = STREAM_CHUNK as u64;
    let skip_chunks = (start / chunk_size) as i32;
    let intra_chunk_skip = (start % chunk_size) as usize;

    let mut download_iter = client
        .iter_download(&media)
        .chunk_size(STREAM_CHUNK)
        .skip_chunks(skip_chunks);

    let mut remaining: Option<u64> = length;
    let stream = async_stream::stream! {
        let mut first = true;
        while remaining.is_none_or(|r| r > 0) {
            match download_iter.next().await.transpose() {
                Some(Ok(mut bytes)) => {
                    if first && intra_chunk_skip > 0 && intra_chunk_skip <= bytes.len() {
                        bytes.drain(..intra_chunk_skip);
                    }
                    first = false;
                    if let Some(r) = remaining {
                        if (bytes.len() as u64) > r {
                            bytes.truncate(r as usize);
                        }
                        remaining = Some(r - bytes.len() as u64);
                    }
                    yield Ok::<_, actix_web::Error>(web::Bytes::from(bytes));
                }
                Some(Err(e)) => {
                    log::error!("Stream error on msg {}: {}", message_id, e);
                    break;
                }
                None => break,
            }
        }
    };

    let mut builder = HttpResponse::build(actix_web::http::StatusCode::from_u16(status_code).unwrap());
    builder
        .insert_header(("Content-Type", mime))
        .insert_header(("Accept-Ranges", "bytes"))
        .insert_header(("Cache-Control", "private, max-age=120"));
    if let Some(cr) = content_range_hdr {
        builder.insert_header(("Content-Range", cr));
    }
    if let Some(len) = length {
        builder.insert_header(("Content-Length", len.to_string()));
    }
    builder.streaming(stream)
}

fn mime_type_from_media(media: &Media) -> String {
    match media {
        Media::Document(d) => d.mime_type().unwrap_or("application/octet-stream").to_string(),
        _ => "application/octet-stream".to_string(),
    }
}

pub async fn start_server(
    state: Arc<TelegramState>,
    port: u16,
    token: Arc<RwLock<String>>,
) -> std::io::Result<(actix_web::dev::Server, u16)> {
    let state_data = web::Data::new(state);
    let token_data = web::Data::new(StreamTokenData { token });

    log::info!("Starting Streaming Server (preferred port {})", port);

    let factory = move || {
        let cors = Cors::default()
            .allowed_origin("tauri://localhost")
            .allowed_origin("http://localhost:1420")
            .allowed_origin("https://tauri.localhost")
            .allow_any_method()
            .allow_any_header();

        App::new()
            .wrap(cors)
            .app_data(state_data.clone())
            .app_data(token_data.clone())
            .service(stream_media)
    };

    let bind_result = HttpServer::new(factory.clone()).bind(("127.0.0.1", port));
    let bound = match bind_result {
        Ok(b) => b,
        Err(e) => {
            log::warn!(
                "Preferred port {} unavailable ({}); falling back to ephemeral port",
                port, e
            );
            HttpServer::new(factory).bind(("127.0.0.1", 0))?
        }
    };

    let actual_port = bound.addrs().first().map(|a| a.port()).unwrap_or(port);
    let server = bound.run();
    log::info!("Streaming Server bound on http://127.0.0.1:{}", actual_port);
    Ok((server, actual_port))
}
