//! History sessions query — list import batches.

use crate::db::Db;
use crate::reporting::{HistorySessionsQuery, HistorySessionsReporter, HistorySessionRow, HistorySessionsSummary};

/// Maximum events to fetch for filtering.
/// This is a safety limit to prevent unbounded queries.
const MAX_EVENTS_TO_SCAN: usize = 10000;

/// Query import sessions (batches) from event log.
/// 
/// Pagination semantics:
/// - All matching events are scanned (up to MAX_EVENTS_TO_SCAN)
/// - Source filter is applied to all events
/// - `summary.total` = total matching events (before pagination)
/// - `summary.returned` = events in current page
/// - `summary.has_more` = true if there are more matching events beyond this page
pub fn query_sessions<R: HistorySessionsReporter>(
    db: &Db,
    query: &HistorySessionsQuery,
    reporter: &R,
) -> anyhow::Result<HistorySessionsSummary> {
    reporter.started(query);

    // Query a large batch of events for accurate filtering and pagination
    // We fetch more than limit+offset to ensure has_more is accurate
    let fetch_limit = if query.offset + query.limit > MAX_EVENTS_TO_SCAN {
        MAX_EVENTS_TO_SCAN
    } else {
        (query.offset + query.limit).saturating_mul(2).min(MAX_EVENTS_TO_SCAN)
    };
    
    let events = db.get_events(
        fetch_limit,
        Some("batch.imported"),
        query.from_ms,
        query.to_ms,
        None,
    )?;

    // Collect all matching rows (apply source filter)
    let mut all_matching_rows: Vec<HistorySessionRow> = Vec::new();
    
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
        
        all_matching_rows.push(row);
    }

    // Calculate total matching (before pagination)
    let total_matching = all_matching_rows.len();
    
    // Apply offset and limit for current page
    let page_rows: Vec<_> = all_matching_rows
        .into_iter()
        .skip(query.offset)
        .take(query.limit)
        .collect();
    let returned = page_rows.len();

    // Report rows for current page
    for row in page_rows {
        reporter.item(&row);
    }

    // has_more is true if we couldn't return all matching rows
    // OR if we hit the fetch limit (indicating there might be more in DB)
    let has_more = total_matching > query.offset + returned 
        || (total_matching == fetch_limit && query.offset + returned >= fetch_limit);
    
    let summary = HistorySessionsSummary { 
        total: total_matching, 
        returned, 
        has_more 
    };
    reporter.finish(&summary);
    
    Ok(summary)
}
