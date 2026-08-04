//! JSON event sink — writes one JSON object per event to stdout.
//!
//! Because [`Event`] derives `Serialize`, this sink is a thin serializer —
//! no hand-written field mapping. Each line is a self-contained JSON object:
//!
//! ```json
//! {"event":"scan_item","path":"/src/IMG_0001.jpg","size":1024,"mtime_ms":0,"status":"new","error":null}
//! {"event":"summary","kind":"import","total":3,"imported":2,"duplicate":1,"failed":0,"manifest_path":null,"all_cache_hit":false}
//! ```

use std::io::{Write, stdout};
use std::sync::Mutex;

use svault_core::event::{Event, EventSink};

/// Sink that serializes every event as one JSON line on stdout.
pub struct JsonSink {
    out: Mutex<std::io::Stdout>,
}

impl JsonSink {
    /// Create a new JSON sink writing to stdout.
    pub fn new() -> Self {
        Self {
            out: Mutex::new(stdout()),
        }
    }
}

impl EventSink for JsonSink {
    fn emit(&self, event: &Event) {
        if let Ok(line) = serde_json::to_string(event) {
            let mut out = self.out.lock().unwrap();
            let _ = writeln!(out, "{}", line);
            let _ = out.flush();
        }
    }
}

impl Default for JsonSink {
    fn default() -> Self {
        Self::new()
    }
}
