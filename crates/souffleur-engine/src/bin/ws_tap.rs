//! `ws-tap` — a minimal Coach Protocol surface: connect to the core's WebSocket
//! and print every event frame. The smallest possible "surface", useful for
//! verifying the core and as the skeleton the real surfaces (phone, glasses,
//! overlay) grow from.
//!
//! Usage: ws-tap [URL] [MAX_FRAMES]
//! Defaults: ws://127.0.0.1:8123  (unlimited)

use anyhow::{Context, Result};
use futures_util::StreamExt;
use tokio_tungstenite::tungstenite::Message;

#[tokio::main]
async fn main() -> Result<()> {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "ws://127.0.0.1:8123".to_string());
    let max: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(usize::MAX);

    let (ws, _resp) = tokio_tungstenite::connect_async(&url)
        .await
        .with_context(|| format!("connect {url}"))?;
    eprintln!("[ws-tap] connected to {url}");
    let (_write, mut read) = ws.split();

    // Once connected, any read outcome (clean close or an abrupt reset when the
    // core exits) is end-of-stream, not failure — a surface must tolerate the
    // core going away. Connect failures (above) are the only thing that errors.
    let mut n = 0usize;
    while let Some(msg) = read.next().await {
        match msg {
            Ok(Message::Text(t)) => {
                println!("{}", t.as_str());
                n += 1;
                if n >= max {
                    break;
                }
            }
            Ok(Message::Close(_)) | Err(_) => break,
            Ok(_) => {}
        }
    }
    eprintln!("[ws-tap] received {n} frame(s)");
    Ok(())
}
