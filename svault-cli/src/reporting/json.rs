//! JSON reporter for machine-readable output.

use std::path::Path;

use serde_json::json;
use svault_core::reporting::{
    AddSummaryReporter, CopyReporter, HashReporter, InsertReporter,
    ItemStatus, RecheckReporter, ReporterBuilder, ScanReporter,
    UpdateApplyReporter, VerifyReporter,
};
use svault_core::verify::{VerifyResult, VerifySummary};

/// JSON reporter builder that outputs structured JSON lines.
#[derive(Debug, Clone, Default)]
pub struct JsonReporterBuilder;

impl JsonReporterBuilder {
    pub fn new() -> Self {
        Self
    }
}

impl ReporterBuilder for JsonReporterBuilder {
    type Scan = JsonScanReporter;
    type Copy = JsonCopyReporter;
    type Hash = JsonHashReporter;
    type Insert = JsonInsertReporter;
    type AddSummary = JsonAddSummaryReporter;
    type Recheck = JsonRecheckReporter;
    type UpdateApply = JsonUpdateApplyReporter;
    type Verify = JsonVerifyReporter;

    fn scan_reporter(&self, _source: &Path) -> JsonScanReporter {
        JsonScanReporter::new()
    }

    fn copy_reporter(&self, _source: &Path, _vault_root: &Path, _total: u64) -> JsonCopyReporter {
        JsonCopyReporter::new()
    }

    fn hash_reporter(&self, _source: &Path, _total: u64) -> JsonHashReporter {
        JsonHashReporter::new()
    }

    fn insert_reporter(&self, _source: &Path, _total: u64) -> JsonInsertReporter {
        JsonInsertReporter::new()
    }

    fn add_summary_reporter(&self, _vault_root: &Path) -> JsonAddSummaryReporter {
        JsonAddSummaryReporter::new()
    }

    fn recheck_reporter(&self, total: u64) -> JsonRecheckReporter {
        JsonRecheckReporter::new(total)
    }

    fn update_hash_reporter(&self, _source: &Path, _total: u64) -> JsonHashReporter {
        JsonHashReporter::new()
    }

    fn update_apply_reporter(&self, total: u64) -> JsonUpdateApplyReporter {
        JsonUpdateApplyReporter::new(total)
    }

    fn verify_reporter(&self, total: u64) -> JsonVerifyReporter {
        JsonVerifyReporter::new(total)
    }
}

macro_rules! emit_json {
    ($obj:expr) => {
        println!("{}", serde_json::to_string(&$obj).unwrap());
    };
}

// ─────────────────────────────────────────────────────────────────────────────
// Scan reporter
// ─────────────────────────────────────────────────────────────────────────────

pub struct JsonScanReporter;

impl JsonScanReporter {
    fn new() -> Self {
        emit_json!(json!({"event": "scan_started"}));
        Self
    }
}

impl ScanReporter for JsonScanReporter {
    fn discovered(&self, path: &Path, size: u64, mtime_ms: i64) {
        emit_json!(json!({
            "event": "file_discovered",
            "path": path.display().to_string(),
            "size": size,
            "mtime_ms": mtime_ms
        }));
    }

    fn classified(&self, path: &Path, size: u64, status: ItemStatus, _detail: Option<&str>) {
        let status_str = match status {
            ItemStatus::New => "new",
            ItemStatus::Duplicate => "duplicate",
            ItemStatus::Recover => "recover",
            ItemStatus::MovedInVault => "moved",
            ItemStatus::Failed => "failed",
        };
        emit_json!(json!({
            "event": "file_classified",
            "path": path.display().to_string(),
            "size": size,
            "status": status_str
        }));
    }

    fn progress(&self, completed: u64) {
        emit_json!(json!({
            "event": "scan_progress",
            "completed": completed
        }));
    }

    fn warning(&self, message: &str, path: Option<&Path>) {
        emit_json!(json!({
            "event": "warning",
            "message": message,
            "path": path.map(|p| p.display().to_string())
        }));
    }

    fn error(&self, message: &str, path: Option<&Path>) {
        emit_json!(json!({
            "event": "error",
            "message": message,
            "path": path.map(|p| p.display().to_string())
        }));
    }

    fn preflight(
        &self,
        total_scanned: usize,
        new_count: usize,
        duplicate_count: usize,
        moved_count: usize,
        failed_count: usize,
        source: &Path,
    ) {
        // If nothing to import, emit special event
        if new_count == 0 {
            emit_json!(json!({
                "event": "nothing_to_import",
                "total": total_scanned,
                "duplicate": duplicate_count,
                "moved": moved_count
            }));
            return;
        }

        emit_json!(json!({
            "event": "preflight",
            "total_scanned": total_scanned,
            "new": new_count,
            "duplicate": duplicate_count,
            "moved": moved_count,
            "failed": failed_count,
            "source": source.display().to_string()
        }));
    }

    fn finish(&self) {
        emit_json!(json!({"event": "scan_finished"}));
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Copy reporter
// ─────────────────────────────────────────────────────────────────────────────

pub struct JsonCopyReporter;

impl JsonCopyReporter {
    fn new() -> Self {
        emit_json!(json!({"event": "copy_started"}));
        Self
    }
}

impl CopyReporter for JsonCopyReporter {
    fn item_started(&self, src_abs: &Path, dest_abs: &Path, bytes_total: u64) {
        emit_json!(json!({
            "event": "copy_item_started",
            "src": src_abs.display().to_string(),
            "dest": dest_abs.display().to_string(),
            "size": bytes_total
        }));
    }

    fn item_finished(&self, src_abs: &Path, dest_abs: &Path, _bytes_total: u64) {
        emit_json!(json!({
            "event": "copy_item_finished",
            "src": src_abs.display().to_string(),
            "dest": dest_abs.display().to_string()
        }));
    }

    fn error(&self, message: &str, path: Option<&Path>) {
        emit_json!(json!({
            "event": "copy_error",
            "message": message,
            "path": path.map(|p| p.display().to_string())
        }));
    }

    fn finish(&self) {
        emit_json!(json!({"event": "copy_finished"}));
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Hash reporter
// ─────────────────────────────────────────────────────────────────────────────

pub struct JsonHashReporter;

impl JsonHashReporter {
    fn new() -> Self {
        emit_json!(json!({"event": "hash_started"}));
        Self
    }
}

impl HashReporter for JsonHashReporter {
    fn item_started(&self, abs_path: &Path, bytes_total: u64) {
        emit_json!(json!({
            "event": "hash_item_started",
            "path": abs_path.display().to_string(),
            "size": bytes_total
        }));
    }

    fn item_finished(&self, abs_path: &Path, error: Option<&str>, _bytes_total: u64) {
        emit_json!(json!({
            "event": "hash_item_finished",
            "path": abs_path.display().to_string(),
            "error": error
        }));
    }

    fn finish(&self) {
        emit_json!(json!({"event": "hash_finished"}));
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Insert reporter
// ─────────────────────────────────────────────────────────────────────────────

pub struct JsonInsertReporter;

impl JsonInsertReporter {
    fn new() -> Self {
        emit_json!(json!({"event": "insert_started"}));
        Self
    }
}

impl InsertReporter for JsonInsertReporter {
    fn progress(&self, completed: u64, total: u64) {
        emit_json!(json!({
            "event": "insert_progress",
            "completed": completed,
            "total": total
        }));
    }

    fn finish(&self) {
        emit_json!(json!({"event": "insert_finished"}));
    }

    fn summary(
        &self,
        total: usize,
        imported: usize,
        duplicate: usize,
        failed: usize,
        manifest_path: Option<&Path>,
    ) {
        emit_json!(json!({
            "event": "import_summary",
            "total": total,
            "imported": imported,
            "duplicate": duplicate,
            "failed": failed,
            "manifest_path": manifest_path.map(|p| p.display().to_string())
        }));
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Add summary reporter
// ─────────────────────────────────────────────────────────────────────────────

pub struct JsonAddSummaryReporter;

impl JsonAddSummaryReporter {
    fn new() -> Self {
        Self
    }
}

impl AddSummaryReporter for JsonAddSummaryReporter {
    fn preflight(&self, new_count: usize, duplicate_count: usize, moved_count: usize) {
        emit_json!(json!({
            "event": "add_preflight",
            "new": new_count,
            "duplicate": duplicate_count,
            "moved": moved_count
        }));
    }

    fn only_moved(&self, moved_files: &[(std::path::PathBuf, String)], vault_root: &Path) {
        let files: Vec<_> = moved_files
            .iter()
            .map(|(p, old)| {
                json!({
                    "current": p.display().to_string(),
                    "previous": old,
                })
            })
            .collect();
        emit_json!(json!({
            "event": "add_only_moved",
            "vault_root": vault_root.display().to_string(),
            "moved_files": files
        }));
    }

    fn summary(&self, total: usize, added: usize, duplicate: usize, failed: usize) {
        emit_json!(json!({
            "event": "add_summary",
            "total": total,
            "added": added,
            "duplicate": duplicate,
            "failed": failed
        }));
    }

    fn moved_hint(&self, moved_files: &[(std::path::PathBuf, String)], vault_root: &Path) {
        let files: Vec<_> = moved_files
            .iter()
            .map(|(p, old)| {
                json!({
                    "current": p.display().to_string(),
                    "previous": old,
                })
            })
            .collect();
        emit_json!(json!({
            "event": "add_moved_hint",
            "vault_root": vault_root.display().to_string(),
            "moved_files": files
        }));
    }

    fn finish(&self) {
        emit_json!(json!({"event": "add_finished"}));
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Recheck reporter
// ─────────────────────────────────────────────────────────────────────────────

pub struct JsonRecheckReporter {
    total: u64,
}

impl JsonRecheckReporter {
    fn new(total: u64) -> Self {
        emit_json!(json!({"event": "recheck_started", "total": total}));
        Self { total }
    }
}

impl RecheckReporter for JsonRecheckReporter {
    fn started(&self, total: usize, session_id: &str, source: &Path) {
        emit_json!(json!({
            "event": "recheck_started",
            "total": total,
            "session_id": session_id,
            "source": source.display().to_string()
        }));
    }

    fn item_started(&self, src_path: &Path, vault_path: &Path) {
        emit_json!(json!({
            "event": "recheck_item_started",
            "src_path": src_path.display().to_string(),
            "vault_path": vault_path.display().to_string()
        }));
    }

    fn item_finished(&self, src_path: &Path, vault_path: &Path, status: &svault_core::import::RecheckStatus) {
        let status_str = match status {
            svault_core::import::RecheckStatus::Ok => "ok",
            svault_core::import::RecheckStatus::SourceModified => "source_modified",
            svault_core::import::RecheckStatus::VaultCorrupted => "vault_corrupted",
            svault_core::import::RecheckStatus::BothDiverged => "both_diverged",
            svault_core::import::RecheckStatus::SourceDeleted => "source_deleted",
            svault_core::import::RecheckStatus::VaultDeleted => "vault_deleted",
            svault_core::import::RecheckStatus::Error(_) => "error",
        };
        emit_json!(json!({
            "event": "recheck_item_finished",
            "src_path": src_path.display().to_string(),
            "vault_path": vault_path.display().to_string(),
            "status": status_str
        }));
    }

    fn finish(&self) {
        emit_json!(json!({"event": "recheck_finished"}));
    }

    fn summary(
        &self,
        ok: usize,
        source_modified: usize,
        vault_corrupted: usize,
        both_diverged: usize,
        source_deleted: usize,
        vault_deleted: usize,
        errors: usize,
        sha256_verified: usize,
        report_path: &Path,
    ) {
        emit_json!(json!({
            "event": "recheck_summary",
            "ok": ok,
            "source_modified": source_modified,
            "vault_corrupted": vault_corrupted,
            "both_diverged": both_diverged,
            "source_deleted": source_deleted,
            "vault_deleted": vault_deleted,
            "errors": errors,
            "sha256_verified": sha256_verified,
            "report_path": report_path.display().to_string()
        }));
    }
}


// ─────────────────────────────────────────────────────────────────────────────
// Update apply reporter
// ─────────────────────────────────────────────────────────────────────────────

pub struct JsonUpdateApplyReporter {
    total: u64,
}

impl JsonUpdateApplyReporter {
    fn new(total: u64) -> Self {
        emit_json!(json!({"event": "update_apply_started", "total": total}));
        Self { total }
    }
}

impl UpdateApplyReporter for JsonUpdateApplyReporter {
    fn progress(&self, completed: u64, _total: u64) {
        emit_json!(json!({
            "event": "update_apply_progress",
            "completed": completed,
            "total": self.total
        }));
    }

    fn error(&self, message: &str, path: &str) {
        emit_json!(json!({
            "event": "update_apply_error",
            "message": message,
            "path": path
        }));
    }

    fn finish(&self) {
        emit_json!(json!({"event": "update_apply_finished"}));
    }

    fn summary(
        &self,
        scanned: usize,
        missing: usize,
        matched: usize,
        unmatched: usize,
        updated: usize,
    ) {
        emit_json!(json!({
            "event": "update_summary",
            "scanned": scanned,
            "missing": missing,
            "matched": matched,
            "unmatched": unmatched,
            "updated": updated
        }));
    }

    fn nothing_to_update(&self) {
        emit_json!(json!({"event": "nothing_to_update"}));
    }

    fn dry_run_missing(&self, count: usize) {
        emit_json!(json!({
            "event": "dry_run_missing",
            "count": count
        }));
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Verify reporter
// ─────────────────────────────────────────────────────────────────────────────

pub struct JsonVerifyReporter {
    total: u64,
}

impl JsonVerifyReporter {
    fn new(total: u64) -> Self {
        emit_json!(json!({"event": "verify_started", "total": total}));
        Self { total }
    }
}

impl VerifyReporter for JsonVerifyReporter {
    fn started(&self, _total: u64) {}

    fn item_started(&self, path: &Path) {
        emit_json!(json!({
            "event": "verify_item_started",
            "path": path.display().to_string()
        }));
    }

    fn item_finished(&self, path: &Path, result: &VerifyResult) {
        let (status, details) = match result {
            VerifyResult::Ok => ("ok", None),
            VerifyResult::Missing => ("missing", None),
            VerifyResult::SizeMismatch { expected, actual } => (
                "size_mismatch",
                Some(json!({"expected": expected, "actual": actual}))
            ),
            VerifyResult::HashMismatch { algo } => (
                "hash_mismatch",
                Some(json!({"algorithm": format!("{:?}", algo)}))
            ),
            VerifyResult::IoError(e) => (
                "io_error",
                Some(json!({"error": e}))
            ),
            VerifyResult::HashNotAvailable => ("hash_not_available", None),
        };
        
        let mut event = json!({
            "event": "verify_item_finished",
            "path": path.display().to_string(),
            "status": status
        });
        
        if let Some(d) = details {
            if let Some(obj) = event.as_object_mut() {
                obj.insert("details".to_string(), d);
            }
        }
        
        emit_json!(event);
    }

    fn finish(&self) {
        emit_json!(json!({"event": "verify_finished"}));
    }

    fn summary(&self, summary: &VerifySummary) {
        emit_json!(json!({
            "event": "verify_summary",
            "total": summary.total,
            "ok": summary.ok,
            "missing": summary.missing,
            "size_mismatch": summary.size_mismatch,
            "hash_mismatch": summary.hash_mismatch,
            "io_error": summary.io_error,
            "hash_not_available": summary.hash_not_available
        }));
    }
}


