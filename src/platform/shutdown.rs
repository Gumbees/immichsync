//! Hidden top-level window that catches Windows shutdown messages.
//!
//! Task Manager → "End task" (and `taskkill` without `/F`) sends `WM_CLOSE`
//! to a process's top-level windows and waits ~5 seconds before escalating
//! to `TerminateProcess`. A tray-only app has no visible top-level window,
//! so without this hook Task Manager finds nothing to message and goes
//! straight to `TerminateProcess` — leaving in-flight uploads orphaned.
//!
//! We register a hidden (no `WS_VISIBLE`) top-level window whose WndProc
//! translates close-related messages into `PostQuitMessage(0)`. The main
//! thread's existing Win32 message pump in `App::run` then sees `WM_QUIT`
//! and runs the normal shutdown path.
//!
//! Messages handled:
//! - `WM_CLOSE`           — Task Manager "End task", taskkill (no /F)
//! - `WM_QUERYENDSESSION` — Logoff/shutdown asking for permission
//! - `WM_ENDSESSION`      — Logoff/shutdown actually proceeding
//!
//! True kill via `TerminateProcess` (Task Manager → "End process tree" or
//! `taskkill /F`) cannot be caught. The crash-recovery path in
//! `App::init` resets stale `"uploading"` queue rows back to `"pending"`
//! on next startup so partial uploads aren't lost — just retried.

use thiserror::Error;
use tracing::{debug, info};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, PostQuitMessage, RegisterClassW, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_CLOSE, WM_ENDSESSION, WM_QUERYENDSESSION, WNDCLASSW,
};

#[derive(Debug, Error)]
pub enum ShutdownWindowError {
    #[error("CreateWindowExW failed: {0}")]
    Create(windows::core::Error),
}

/// Register the shutdown window on the current thread.
///
/// Must be called from the same thread that runs the main Win32 message
/// pump — window messages are dispatched only to the thread that owns the
/// window. In ImmichSync that's the thread running `App::run`.
pub fn install() -> Result<(), ShutdownWindowError> {
    // Null-terminated wide strings kept alive across the FFI calls.
    let class_wide: Vec<u16> = "ImmichSyncShutdown\0".encode_utf16().collect();
    let title_wide: Vec<u16> = "ImmichSync\0".encode_utf16().collect();

    let wc = WNDCLASSW {
        lpfnWndProc: Some(shutdown_wnd_proc),
        lpszClassName: PCWSTR(class_wide.as_ptr()),
        ..Default::default()
    };

    // Re-registering the same class would error; install() is called once
    // at startup so we ignore the return value.
    unsafe {
        RegisterClassW(&wc);
    }

    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            PCWSTR(class_wide.as_ptr()),
            PCWSTR(title_wide.as_ptr()),
            // WS_OVERLAPPED with no WS_VISIBLE → top-level, never shown.
            // The lack of HWND_MESSAGE parent is what makes it top-level
            // (and therefore enumerable by Task Manager) instead of
            // message-only like watch::device's WM_DEVICECHANGE window.
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            None, // no parent — top-level
            None,
            None,
            None,
        )
    }
    .map_err(ShutdownWindowError::Create)?;

    info!(?hwnd, "Shutdown window registered");
    Ok(())
}

unsafe extern "system" fn shutdown_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CLOSE => {
            info!("WM_CLOSE received — initiating graceful shutdown");
            PostQuitMessage(0);
            LRESULT(0)
        }
        WM_QUERYENDSESSION => {
            // TRUE = we agree to end. Windows will then deliver WM_ENDSESSION.
            debug!("WM_QUERYENDSESSION received");
            LRESULT(1)
        }
        WM_ENDSESSION => {
            // wParam != 0 means the session is actually ending (not cancelled).
            if wparam.0 != 0 {
                info!("WM_ENDSESSION received — initiating graceful shutdown");
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
