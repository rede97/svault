//! Utility functions for import pipeline.

use std::time::{SystemTime, UNIX_EPOCH};

/// Current Unix timestamp in milliseconds.
pub fn unix_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// Generate a session ID: local timestamp + unique suffix.
///
/// Format: `YYYYMMDDTHHMMSS-fffff` (hex suffix from sub-second microseconds
/// XORed with the PID). Second-resolution IDs collided when two sessions
/// started within the same second (formerly BUG-4); the suffix makes
/// collisions practically impossible, and the vault process lock serializes
/// writers anyway.
pub fn session_id_now() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let ts = chrono::Local::now().format("%Y%m%dT%H%M%S");
    let suffix = (now.subsec_micros() ^ std::process::id()) & 0xFFFFF;
    format!("{ts}-{suffix:05x}")
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
        // Format: YYYYMMDDTHHMMSS-fffff (timestamp + unique hex suffix)
        let (ts, suffix) = id.rsplit_once('-').expect("session id has suffix");
        assert_eq!(ts.len(), 15); // YYYYMMDD T HHMMSS
        assert_eq!(ts.chars().nth(8), Some('T'));
        assert_eq!(suffix.len(), 5);
        assert!(suffix.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_session_id_unique_within_same_second() {
        // BUG-4 regression: two sessions started back-to-back must differ.
        let a = session_id_now();
        let b = session_id_now();
        assert_ne!(a, b);
    }
}
