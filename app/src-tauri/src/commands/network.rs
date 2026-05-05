use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

/// Ultra-lightweight network check
///
/// Tries to TCP-connect to Telegram's DC2 without invoking grammers.
/// Avoids the stack-overflow path in grammers reconnection logic.
#[tauri::command]
pub async fn cmd_is_network_available() -> Result<bool, String> {
    tokio::task::spawn_blocking(|| {
        let addr: SocketAddr = "149.154.167.50:443"
            .parse()
            .expect("hardcoded Telegram DC2 address must parse");
        Ok(TcpStream::connect_timeout(&addr, Duration::from_secs(2)).is_ok())
    })
    .await
    .map_err(|e| e.to_string())?
}
