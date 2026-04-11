//! CLI command implementations.

pub mod add;
pub mod clone;
pub mod db;
#[cfg(debug_assertions)]
pub mod debug_reporter;
pub mod history;
pub mod import;
pub mod init;
pub mod recheck;
pub mod scan;
pub mod status;
pub mod sync;
pub mod update;
pub mod verify;

/// Parse a datetime string (RFC 3339 or YYYY-MM-DD) into Unix milliseconds.
pub fn parse_datetime_to_ms(s: &str) -> Option<i64> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        let dt_utc: chrono::DateTime<chrono::Utc> = dt.into();
        return Some(dt_utc.timestamp_millis());
    }
    if let Ok(date) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let dt: chrono::NaiveDateTime = date.and_hms_opt(0, 0, 0)?;
        return Some(dt.and_utc().timestamp_millis());
    }
    None
}

/// Format bytes to human readable string
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    if bytes == 0 {
        return "0 B".to_string();
    }
    let exp = (bytes as f64).log(1024.0).min(4.0) as usize;
    let value = bytes as f64 / 1024f64.powi(exp as i32);
    if exp == 0 {
        format!("{} {}", bytes, UNITS[0])
    } else {
        format!("{:.1} {}", value, UNITS[exp])
    }
}

/// Format byte size to human-readable string.
pub fn format_size(size: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    if size == 0 {
        return "0 B".to_string();
    }
    let exp = (size as f64).log(1024.0).min(UNITS.len() as f64 - 1.0) as usize;
    let value = size as f64 / 1024_f64.powi(exp as i32);
    if exp == 0 {
        format!("{} {}", size, UNITS[0])
    } else {
        format!("{:.2} {}", value, UNITS[exp])
    }
}

/// Global flag for shutdown signal
static SHUTDOWN_REQUESTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Check if shutdown has been requested (for periodic checks in long operations)
pub fn is_shutdown_requested() -> bool {
    SHUTDOWN_REQUESTED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Setup signal handler for graceful shutdown on Ctrl-C
pub fn setup_signal_handler() {
    ctrlc::set_handler(move || {
        eprintln!("\n⚠️  Received interrupt signal (Ctrl-C)");
        eprintln!("   Shutting down, please wait...");
        SHUTDOWN_REQUESTED.store(true, std::sync::atomic::Ordering::Relaxed);
        // Give the program a moment to clean up
        std::thread::sleep(std::time::Duration::from_millis(800));
        std::process::exit(130); // 128 + SIGINT(2)
    })
    .expect("Error setting Ctrl-C handler");
}
