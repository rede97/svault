//! Reporter types and traits for the `history` command.

/// Query parameters for history sessions query.
#[derive(Debug, Clone, Default)]
pub struct HistorySessionsQuery {
    pub limit: usize,
    pub offset: usize,
    pub source: Option<String>,
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
}

/// Query parameters for history items query.
#[derive(Debug, Clone, Default)]
pub struct HistoryItemsQuery {
    pub limit: usize,
    pub offset: usize,
    pub status: Option<String>,
}

/// Row data for a history session.
#[derive(Debug, Clone)]
pub struct HistorySessionRow {
    pub session_id: String,
    pub session_type: String,
    pub source: String,
    pub started_at_ms: i64,
    pub total_files: usize,
    pub added: usize,
    pub duplicate: usize,
    pub failed: usize,
    pub skipped: usize,
}

/// Row data for a history item.
#[derive(Debug, Clone)]
pub struct HistoryItemRow {
    pub source_path: String,
    pub vault_path: String,
    pub status: String,
    pub size: u64,
    pub mtime_ms: i64,
}

/// Summary for history sessions query.
#[derive(Debug, Clone)]
pub struct HistorySessionsSummary {
    pub total: usize,
    pub returned: usize,
    pub has_more: bool,
}

/// Summary for history items query.
#[derive(Debug, Clone)]
pub struct HistoryItemsSummary {
    pub total: usize,
    pub returned: usize,
    pub has_more: bool,
}

/// Reporter for history sessions query.
pub trait HistorySessionsReporter: Send + Sync {
    fn started(&self, query: &HistorySessionsQuery);
    fn item(&self, row: &HistorySessionRow);
    fn finish(&self, summary: &HistorySessionsSummary);
}

/// Reporter for history items query.
pub trait HistoryItemsReporter: Send + Sync {
    fn started(&self, session_id: &str, query: &HistoryItemsQuery);
    fn item(&self, row: &HistoryItemRow);
    fn finish(&self, summary: &HistoryItemsSummary);
}
