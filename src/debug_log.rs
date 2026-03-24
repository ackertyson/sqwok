use std::io::Write;
use std::sync::Mutex;

static LOG_FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);

/// Initialise the debug log. Call once at startup.
/// Writes to `$HOME/.sqwok/debug.log` (truncated each run).
pub fn init() {
    let path = dirs_path();
    if let Ok(file) = std::fs::File::create(&path) {
        *LOG_FILE.lock().unwrap() = Some(file);
    }
}

/// Append a timestamped line to the debug log.
pub fn log(msg: &str) {
    if let Ok(mut guard) = LOG_FILE.lock() {
        if let Some(ref mut f) = *guard {
            let ts = chrono::Utc::now().format("%H:%M:%S%.3f");
            let _ = writeln!(f, "[{ts}] {msg}");
            let _ = f.flush();
        }
    }
}

fn dirs_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    std::path::PathBuf::from(home)
        .join(".sqwok")
        .join("debug.log")
}

/// Convenience macro so call-sites look like `dlog!("foo {}", bar)`.
#[macro_export]
macro_rules! dlog {
    ($($arg:tt)*) => {
        $crate::debug_log::log(&format!($($arg)*))
    };
}
