//! Vault-to-vault comparison engine — pure functions, no IO.
//!
//! This module is the heart of `svault sync`. It compares the file records
//! of two vaults **by database identity** (SHA-256 preferred, XXH3-128 as
//! fallback) without touching the filesystem, which makes it both fast
//! (no hashing IO) and exhaustively unit-testable.
//!
//! See `docs/ARCHITECTURE.md` §6.2 for the classification semantics.

use std::collections::HashMap;

/// A file record normalized for comparison.
///
/// Constructed from `db::FileRow`; paths are Unix-style vault-relative.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRecord {
    /// Unix-style vault-relative path.
    pub path: String,
    /// File size in bytes.
    pub size: i64,
    /// Modification time (Unix timestamp ms).
    pub mtime: i64,
    /// XXH3-128 as raw 16 bytes.
    pub xxh3_128: Option<Vec<u8>>,
    /// SHA-256 as raw 32 bytes.
    pub sha256: Option<Vec<u8>>,
    /// Format-specific CRC32C (carried for cache fidelity on copy).
    pub fingerprint: Option<Vec<u8>>,
    /// RAW unique ID (carried for dedup fidelity on copy).
    pub raw_unique_id: Option<String>,
}

impl FileRecord {
    /// Content identity: SHA-256 when available, otherwise XXH3-128.
    ///
    /// Mirrors the identity rule documented on the `files` table schema.
    pub fn identity(&self) -> Option<&[u8]> {
        self.sha256.as_deref().or(self.xxh3_128.as_deref())
    }
}

impl From<&crate::db::FileRow> for FileRecord {
    fn from(row: &crate::db::FileRow) -> Self {
        Self {
            path: row.path.clone(),
            size: row.size,
            mtime: row.mtime,
            xxh3_128: row.xxh3_128.clone(),
            sha256: row.sha256.clone(),
            fingerprint: row.fingerprint.clone(),
            raw_unique_id: row.raw_unique_id.clone(),
        }
    }
}

/// Classification of one file in a vault comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffEntry {
    /// Same identity at the same path in both vaults — nothing to do.
    Identical { path: String },
    /// Present in source, absent in dest — candidate for copying.
    OnlySource { record: FileRecord },
    /// Present in dest, absent in source — reported, **never deleted**.
    OnlyDest { record: FileRecord },
    /// Same identity at different paths — reported (v1 does not fix paths).
    Moved {
        source_path: String,
        dest_path: String,
    },
    /// Same path but different identities — dest is kept, copy is skipped.
    Conflict {
        path: String,
        source_identity: Vec<u8>,
        dest_identity: Vec<u8>,
    },
}

/// The outcome of comparing two vaults.
#[derive(Debug, Default)]
pub struct DiffPlan {
    /// All classified entries (for reporting / debugging).
    pub entries: Vec<DiffEntry>,
    /// Source records to copy into dest (subset of `OnlySource` that has a
    /// usable content identity).
    pub to_copy: Vec<FileRecord>,
    /// `OnlySource` records skipped because they have no hash at all.
    pub skipped_hashless: usize,
}

impl DiffPlan {
    /// Count entries of a given classification.
    fn count(&self, f: impl Fn(&DiffEntry) -> bool) -> usize {
        self.entries.iter().filter(|e| f(e)).count()
    }

    /// Files present in both vaults.
    pub fn identical(&self) -> usize {
        self.count(|e| matches!(e, DiffEntry::Identical { .. }))
    }

    /// Files only in the dest vault.
    pub fn only_dest(&self) -> usize {
        self.count(|e| matches!(e, DiffEntry::OnlyDest { .. }))
    }

    /// Same content at different paths.
    pub fn moved(&self) -> usize {
        self.count(|e| matches!(e, DiffEntry::Moved { .. }))
    }

    /// Same path, different content.
    pub fn conflicts(&self) -> usize {
        self.count(|e| matches!(e, DiffEntry::Conflict { .. }))
    }

    /// Conflict paths for reporting.
    pub fn conflict_paths(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter_map(|e| match e {
                DiffEntry::Conflict { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect()
    }

    /// Total bytes to copy.
    pub fn copy_bytes(&self) -> u64 {
        self.to_copy.iter().map(|r| r.size.max(0) as u64).sum()
    }
}

/// Compare two vaults' file records by content identity.
///
/// Both inputs should contain only records whose files exist on disk
/// (`status = 'imported'`); callers filter before calling.
///
/// Algorithm:
/// 1. Index dest records by identity and by path.
/// 2. For each source record:
///    - same path + same identity → `Identical`
///    - same path + different identity → `Conflict`
///    - identity found at a different dest path → `Moved`
///    - otherwise → `OnlySource` (copy candidate if it has an identity)
/// 3. Dest records whose identity matched nothing → `OnlyDest`.
pub fn diff_vaults(source: &[FileRecord], dest: &[FileRecord]) -> DiffPlan {
    let mut plan = DiffPlan::default();

    // Index dest by identity and by path
    let mut dest_by_identity: HashMap<&[u8], &FileRecord> = HashMap::new();
    let mut dest_by_path: HashMap<&str, &FileRecord> = HashMap::new();
    for rec in dest {
        if let Some(id) = rec.identity() {
            dest_by_identity.insert(id, rec);
        }
        dest_by_path.insert(rec.path.as_str(), rec);
    }

    // Track which dest records got matched (by path — every record has one)
    let mut matched_dest_paths: std::collections::HashSet<&str> = std::collections::HashSet::new();

    for src in source {
        let src_id = src.identity();

        // Case 1: same path exists in dest
        if let Some(dst) = dest_by_path.get(src.path.as_str()) {
            matched_dest_paths.insert(dst.path.as_str());
            match (src_id, dst.identity()) {
                (Some(s), Some(d)) if s == d => {
                    plan.entries.push(DiffEntry::Identical {
                        path: src.path.clone(),
                    });
                }
                // Both hashless: fall back to path + size equality.
                (None, None) if src.size == dst.size => {
                    plan.entries.push(DiffEntry::Identical {
                        path: src.path.clone(),
                    });
                }
                _ => {
                    plan.entries.push(DiffEntry::Conflict {
                        path: src.path.clone(),
                        source_identity: src_id.unwrap_or_default().to_vec(),
                        dest_identity: dst.identity().unwrap_or_default().to_vec(),
                    });
                }
            }
            continue;
        }

        // Case 2: identity exists elsewhere in dest → moved
        if let Some(id) = src_id
            && let Some(dst) = dest_by_identity.get(id)
        {
            matched_dest_paths.insert(dst.path.as_str());
            plan.entries.push(DiffEntry::Moved {
                source_path: src.path.clone(),
                dest_path: dst.path.clone(),
            });
            continue;
        }

        // Case 3: only in source
        plan.entries.push(DiffEntry::OnlySource {
            record: src.clone(),
        });
        if src_id.is_some() {
            plan.to_copy.push(src.clone());
        } else {
            plan.skipped_hashless += 1;
        }
    }

    // Dest records that matched nothing
    for rec in dest {
        if !matched_dest_paths.contains(rec.path.as_str()) {
            plan.entries.push(DiffEntry::OnlyDest {
                record: rec.clone(),
            });
        }
    }

    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(path: &str, xxh3: Option<u8>, sha: Option<u8>) -> FileRecord {
        FileRecord {
            path: path.to_string(),
            size: 100,
            mtime: 1000,
            xxh3_128: xxh3.map(|b| vec![b; 16]),
            sha256: sha.map(|b| vec![b; 32]),
            fingerprint: None,
            raw_unique_id: None,
        }
    }

    fn paths_of_copy(plan: &DiffPlan) -> Vec<&str> {
        plan.to_copy.iter().map(|r| r.path.as_str()).collect()
    }

    #[test]
    fn identical_vaults_copy_nothing() {
        let src = vec![rec("a/1.jpg", Some(1), None), rec("a/2.jpg", None, Some(2))];
        let dst = src.clone();
        let plan = diff_vaults(&src, &dst);
        assert_eq!(plan.identical(), 2);
        assert!(plan.to_copy.is_empty());
        assert_eq!(plan.only_dest(), 0);
        assert_eq!(plan.copy_bytes(), 0);
    }

    #[test]
    fn only_source_files_are_copy_candidates() {
        let src = vec![rec("a/1.jpg", Some(1), None), rec("b/2.jpg", Some(2), None)];
        let dst = vec![rec("a/1.jpg", Some(1), None)];
        let plan = diff_vaults(&src, &dst);
        assert_eq!(plan.identical(), 1);
        assert_eq!(paths_of_copy(&plan), vec!["b/2.jpg"]);
        assert_eq!(plan.copy_bytes(), 100);
    }

    #[test]
    fn only_dest_files_are_reported_not_deleted() {
        let src = vec![];
        let dst = vec![rec("x/9.jpg", Some(9), None)];
        let plan = diff_vaults(&src, &dst);
        assert_eq!(plan.only_dest(), 1);
        assert!(plan.to_copy.is_empty());
        assert!(matches!(
            &plan.entries[0],
            DiffEntry::OnlyDest { record } if record.path == "x/9.jpg"
        ));
    }

    #[test]
    fn same_identity_different_path_is_moved() {
        let src = vec![rec("2024/a.jpg", Some(1), None)];
        let dst = vec![rec("archive/a.jpg", Some(1), None)];
        let plan = diff_vaults(&src, &dst);
        assert_eq!(plan.moved(), 1);
        assert!(plan.to_copy.is_empty());
        assert_eq!(plan.only_dest(), 0);
        assert!(matches!(
            &plan.entries[0],
            DiffEntry::Moved { source_path, dest_path }
                if source_path == "2024/a.jpg" && dest_path == "archive/a.jpg"
        ));
    }

    #[test]
    fn same_path_different_identity_is_conflict() {
        let src = vec![rec("a/1.jpg", Some(1), None)];
        let dst = vec![rec("a/1.jpg", Some(2), None)];
        let plan = diff_vaults(&src, &dst);
        assert_eq!(plan.conflicts(), 1);
        assert_eq!(plan.conflict_paths(), vec!["a/1.jpg".to_string()]);
        assert!(plan.to_copy.is_empty());
    }

    #[test]
    fn sha256_takes_precedence_as_identity() {
        // Same sha256, different xxh3 (should be Identical — sha wins)
        let src = vec![rec("a/1.jpg", Some(1), Some(7))];
        let dst = vec![rec("a/1.jpg", Some(2), Some(7))];
        let plan = diff_vaults(&src, &dst);
        assert_eq!(plan.identical(), 1);
    }

    #[test]
    fn xxh3_matches_sha_when_other_side_lacks_sha() {
        // src has only xxh3=1; dst has xxh3=1 AND sha=9.
        // dst identity = sha=9 ≠ src identity xxh3=1 → not identical by identity,
        // but same path with differing identities → conflict (conservative).
        let src = vec![rec("a/1.jpg", Some(1), None)];
        let dst = vec![rec("a/1.jpg", Some(1), Some(9))];
        let plan = diff_vaults(&src, &dst);
        assert_eq!(plan.conflicts(), 1);
    }

    #[test]
    fn hashless_source_file_is_skipped_not_copied() {
        let src = vec![rec("a/1.jpg", None, None)];
        let dst = vec![];
        let plan = diff_vaults(&src, &dst);
        assert_eq!(plan.skipped_hashless, 1);
        assert!(plan.to_copy.is_empty());
    }

    #[test]
    fn hashless_pair_at_same_path_matches_by_size() {
        let src = vec![rec("a/1.jpg", None, None)];
        let mut same = rec("a/1.jpg", None, None);
        same.size = 100;
        let plan = diff_vaults(&src, &[same]);
        assert_eq!(plan.identical(), 1);

        let mut different = rec("a/1.jpg", None, None);
        different.size = 200;
        let plan = diff_vaults(&src, &[different]);
        assert_eq!(plan.conflicts(), 1);
    }

    #[test]
    fn empty_source_reports_everything_as_only_dest() {
        let dst = vec![rec("a.jpg", Some(1), None), rec("b.jpg", Some(2), None)];
        let plan = diff_vaults(&[], &dst);
        assert_eq!(plan.only_dest(), 2);
        assert!(plan.to_copy.is_empty());
    }

    #[test]
    fn mixed_scenario() {
        let src = vec![
            rec("same.jpg", Some(1), None),  // identical
            rec("new.jpg", Some(2), None),   // only source → copy
            rec("moved.jpg", Some(3), None), // moved (dst has it elsewhere)
            rec("clash.jpg", Some(4), None), // conflict
            rec("no_hash.jpg", None, None),  // hashless → skipped
        ];
        let dst = vec![
            rec("same.jpg", Some(1), None),
            rec("elsewhere/moved.jpg", Some(3), None),
            rec("clash.jpg", Some(5), None),
            rec("dest_only.jpg", Some(6), None),
        ];
        let plan = diff_vaults(&src, &dst);
        assert_eq!(plan.identical(), 1);
        assert_eq!(paths_of_copy(&plan), vec!["new.jpg"]);
        assert_eq!(plan.moved(), 1);
        assert_eq!(plan.conflicts(), 1);
        assert_eq!(plan.only_dest(), 1);
        assert_eq!(plan.skipped_hashless, 1);
        assert_eq!(plan.entries.len(), 6);
    }
}
