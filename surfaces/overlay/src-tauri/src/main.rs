//! Souffleur desktop overlay (Tauri v2).
//!
//! A frameless, transparent, always-on-top, click-through window that renders the
//! Coach Protocol stream and is excluded from screen capture where the OS supports
//! it. The exclusion is REAL on Windows (`WDA_EXCLUDEFROMCAPTURE`), a NO-OP on
//! macOS 15+ (ScreenCaptureKit ignores `NSWindowSharingNone`) and on Linux (no
//! per-window API). `capture_status` reports the truth so the UI never claims
//! invisibility it doesn't have.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::Serialize;
use tauri::Manager;

#[derive(Serialize, Clone)]
struct CaptureStatus {
    os: String,
    hidden: bool,
    note: String,
}

fn capture_status_impl() -> CaptureStatus {
    #[cfg(target_os = "windows")]
    {
        CaptureStatus {
            os: "windows".into(),
            hidden: true,
            note: "excluded via SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)".into(),
        }
    }
    #[cfg(target_os = "macos")]
    {
        CaptureStatus {
            os: "macos".into(),
            hidden: false,
            note: "ScreenCaptureKit ignores window exclusion on macOS 15+; overlay is captured".into(),
        }
    }
    #[cfg(target_os = "linux")]
    {
        CaptureStatus {
            os: "linux".into(),
            hidden: false,
            note: "no per-window screen-capture exclusion API on X11/Wayland".into(),
        }
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        CaptureStatus {
            os: "other".into(),
            hidden: false,
            note: "unknown platform; assume visible".into(),
        }
    }
}

#[tauri::command]
fn capture_status() -> CaptureStatus {
    capture_status_impl()
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![capture_status])
        .setup(|app| {
            if let Some(win) = app.get_webview_window("overlay") {
                // Request capture exclusion (real on Windows; no-op elsewhere) and
                // make the overlay click-through so it never steals interaction.
                let _ = win.set_content_protected(true);
                let _ = win.set_ignore_cursor_events(true);
            }
            let s = capture_status_impl();
            if s.hidden {
                println!("[overlay] content protection ON — hidden from screen capture ({})", s.os);
            } else {
                println!(
                    "[overlay] WARNING: this OS ({}) does NOT hide windows from screen capture — the overlay WILL appear on screen-share. {}",
                    s.os, s.note
                );
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running souffleur overlay");
}
