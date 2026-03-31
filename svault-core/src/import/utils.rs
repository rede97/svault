//! Utility functions for import pipeline.

use std::time::{SystemTime, UNIX_EPOCH};

/// Current Unix timestamp in milliseconds.
pub fn unix_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// Generate a session ID based on current timestamp.
/// Format: Unix seconds (sufficient for session uniqueness).
pub fn session_id_now() -> String {
    let ms = unix_now_ms();
    let secs = ms / 1000;
    format!("{secs}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unix_now_ms_increases() {
        let t1 = unix_now_ms();
        std::thread::sleep(std::time::Duration::from_millis(10));
        let t2 = unix_now_ms();
        assert!(t2 >= t1);
    }

    #[test]
    fn test_session_id_format() {
        let id = session_id_now();
        // Should be numeric (Unix seconds)
        assert!(id.parse::<u64>().is_ok());
    }
}
