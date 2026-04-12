//! History sessions query — list import batches.

use crate::db::Db;
use crate::reporting::{HistorySessionsQuery, HistorySessionsReporter, HistorySessionRow, HistorySessionsSummary};

/// Query import sessions (batches) from event log.
pub fn query_sessions<R: HistorySessionsReporter>(
    db: &Db,
    query: &HistorySessionsQuery,
    reporter: &R,
) -> anyhow::Result<HistorySessionsSummary> {
    reporter.started(query);

    // Query batch.imported events from the events table
    let events = db.get_events(
        query.limit + query.offset,
        Some("batch.imported"),
        query.from_ms,
        query.to_ms,
        None,
    )?;

    let mut rows = Vec::new();
    
    for event in events {
        // Parse payload
        let payload: serde_json::Value = serde_json::from_str(&event.payload).unwrap_or_default();
        
        let session_id = payload["session_id"]
            .as_str()
            .unwrap_or("unknown")
            .to_string();
        
        // Skip if source filter doesn't match
        if let Some(ref source_filter) = query.source {
            let source = payload["source"].as_str().unwrap_or("");
            if !source.contains(source_filter) {
                continue;
            }
        }

        // Determine session type from event context or payload
        let session_type = payload["session_type"]
            .as_str()
            .unwrap_or("import")
            .to_string();

        let row = HistorySessionRow {
            session_id,
            session_type,
            source: payload["source"].as_str().unwrap_or("unknown").to_string(),
            started_at_ms: event.occurred_at,
            total_files: payload["total_files"].as_u64().unwrap_or(0) as usize,
            added: payload["added"].as_u64().unwrap_or(0) as usize,
            duplicate: payload["duplicate"].as_u64().unwrap_or(0) as usize,
            failed: payload["failed"].as_u64().unwrap_or(0) as usize,
            skipped: payload["skipped"].as_u64().unwrap_or(0) as usize,
        };
        
        rows.push(row);
    }

    // Apply offset and limit
    let total = rows.len();
    let offset_rows: Vec<_> = rows.into_iter().skip(query.offset).take(query.limit).collect();
    let returned = offset_rows.len();

    // Report rows
    for row in offset_rows {
        reporter.item(&row);
    }

    let has_more = total > query.offset + returned;
    let summary = HistorySessionsSummary { total, returned, has_more };
    reporter.finish(&summary);
    
    Ok(summary)
}