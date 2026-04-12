//! History items query — list files in a specific import session.

use std::path::Path;

use crate::db::Db;
use crate::reporting::{HistoryItemsQuery, HistoryItemsReporter, HistoryItemRow, HistoryItemsSummary};
use crate::verify::manifest::{ManifestManager, ItemStatus};

/// Parse status string to ItemStatus.
fn parse_status_filter(filter: &str) -> Option<ItemStatus> {
    match filter.to_lowercase().as_str() {
        "added" => Some(ItemStatus::Added),
        "duplicate" | "dup" => Some(ItemStatus::Duplicate),
        "failed" | "fail" => Some(ItemStatus::Failed),
        "skipped" | "skip" => Some(ItemStatus::Skipped),
        "missing" => Some(ItemStatus::Missing),
        "moved" | "move" => Some(ItemStatus::Moved),
        "relinked" => Some(ItemStatus::Relinked),
        "unchanged" => Some(ItemStatus::Unchanged),
        _ => None,
    }
}

/// Query items (files) in a specific import session.
pub fn query_items<R: HistoryItemsReporter>(
    _db: &Db,
    vault_root: &Path,
    session_id: &str,
    query: &HistoryItemsQuery,
    reporter: &R,
) -> anyhow::Result<HistoryItemsSummary> {
    reporter.started(session_id, query);

    // Try to load manifest for this session
    let manager = ManifestManager::new(vault_root);
    
    let items: Vec<HistoryItemRow> = match manager.load(session_id) {
        Ok(manifest) => {
            // Parse status filter if provided
            let status_filter = query.status.as_ref().and_then(|s| parse_status_filter(s));
            
            // If user specified a status but it's invalid, return empty
            if query.status.is_some() && status_filter.is_none() {
                Vec::new()
            } else {
                manifest.files.into_iter()
                    .filter(|f| {
                        // Apply status filter if specified
                        if let Some(filter) = status_filter {
                            f.status == filter
                        } else {
                            true
                        }
                    })
                    .map(|f| {
                        HistoryItemRow {
                            source_path: f.src_path.to_string_lossy().to_string(),
                            vault_path: f.dest_path.as_ref()
                                .map(|p| p.to_string_lossy().to_string())
                                .unwrap_or_default(),
                            status: f.status.to_string(),
                            size: f.size,
                            mtime_ms: f.mtime_ms,
                        }
                    }).collect()
            }
        }
        Err(_) => {
            // Manifest not found, return empty
            Vec::new()
        }
    };

    // Apply offset and limit
    let total = items.len();
    let offset_items: Vec<_> = items.into_iter().skip(query.offset).take(query.limit).collect();
    let returned = offset_items.len();

    // Report rows
    for item in offset_items {
        reporter.item(&item);
    }

    let has_more = total > query.offset + returned;
    let summary = HistoryItemsSummary { total, returned, has_more };
    reporter.finish(&summary);
    
    Ok(summary)
}