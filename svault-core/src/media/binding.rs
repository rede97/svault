//! Media binding detection for related files.
//!
//! Detects:
//! - Live Photo: .heic/.jpg + .mov pairs
//! - RAW+JPG: .dng/.arw/etc + .jpg pairs
//! - Burst sequences: numbered files

use super::{MediaInfo, Path, Result};
use crate::media::formats::{FormatCategory, MediaFormat};
use regex::Regex;
use std::sync::OnceLock;

/// Types of media bindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BindingKind {
    /// Live Photo pair (HEIF/JPG + MOV)
    LivePhoto,
    /// RAW+JPG pair
    RawPlusJpg,
    /// Burst sequence
    Burst,
    /// Unknown/unsupported binding
    Unknown,
}

/// Detected media binding between files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaBinding {
    /// Type of binding
    pub kind: BindingKind,
    /// Primary file (the one usually shown to users)
    pub primary: std::path::PathBuf,
    /// Secondary files (RAW component, video component, etc.)
    pub secondaries: Vec<std::path::PathBuf>,
    /// Group identifier (timestamp, sequence, etc.)
    pub group_id: String,
}

impl MediaBinding {
    /// Create a new binding.
    pub fn new(
        kind: BindingKind,
        primary: std::path::PathBuf,
        secondaries: Vec<std::path::PathBuf>,
        group_id: String,
    ) -> Self {
        Self {
            kind,
            primary,
            secondaries,
            group_id,
        }
    }

    /// Check if a path is part of this binding.
    pub fn contains(&self, path: &Path) -> bool {
        self.primary == path || self.secondaries.iter().any(|p| p == path)
    }

    /// Get all paths in this binding.
    pub fn all_paths(&self) -> Vec<&Path> {
        let mut paths = vec![self.primary.as_path()];
        paths.extend(self.secondaries.iter().map(|p| p.as_path()));
        paths
    }

    /// Get total size of all files in binding.
    pub fn total_size(&self) -> u64 {
        self.all_paths()
            .iter()
            .filter_map(|p| std::fs::metadata(p).ok().map(|m| m.len()))
            .sum()
    }
}

/// Detector for finding related media files.
pub struct BindingDetector {
    /// Maximum time difference for files to be considered related (seconds)
    time_tolerance: i64,
    /// Whether to detect Live Photos
    detect_live_photo: bool,
    /// Whether to detect RAW+JPG pairs
    detect_raw_plus_jpg: bool,
    /// Whether to detect burst sequences
    detect_burst: bool,
}

impl Default for BindingDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl BindingDetector {
    /// Create a new detector with default settings.
    pub fn new() -> Self {
        Self {
            time_tolerance: 2, // 2 seconds
            detect_live_photo: true,
            detect_raw_plus_jpg: true,
            detect_burst: true,
        }
    }

    /// Set time tolerance in seconds.
    pub fn time_tolerance(mut self, seconds: i64) -> Self {
        self.time_tolerance = seconds;
        self
    }

    /// Disable Live Photo detection.
    pub fn disable_live_photo(mut self) -> Self {
        self.detect_live_photo = false;
        self
    }

    /// Disable RAW+JPG detection.
    pub fn disable_raw_plus_jpg(mut self) -> Self {
        self.detect_raw_plus_jpg = false;
        self
    }

    /// Disable burst detection.
    pub fn disable_burst(mut self) -> Self {
        self.detect_burst = false;
        self
    }

    /// Find all bindings in a directory.
    pub fn find_bindings(&self, dir: &Path) -> Result<Vec<MediaBinding>> {
        let entries = std::fs::read_dir(dir)?;
        let mut files: Vec<MediaInfo> = Vec::new();

        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.is_file()
                && let Ok(info) = MediaInfo::from_path(&path)
            {
                files.push(info);
            }
        }

        self.group_files(files)
    }

    /// Group files into bindings.
    fn group_files(&self, files: Vec<MediaInfo>) -> Result<Vec<MediaBinding>> {
        let mut bindings: Vec<MediaBinding> = Vec::new();
        let mut processed: Vec<bool> = vec![false; files.len()];

        for i in 0..files.len() {
            if processed[i] {
                continue;
            }

            let file = &files[i];
            let mut group = vec![i];

            // Find related files
            for j in (i + 1)..files.len() {
                if processed[j] {
                    continue;
                }

                if self.are_related(file, &files[j]) {
                    group.push(j);
                }
            }

            // Mark as processed
            for &idx in &group {
                processed[idx] = true;
            }

            // Create binding if we found a group
            if (group.len() > 1 || self.is_binding_candidate(&files[group[0]]))
                && let Some(binding) = self.create_binding(&files, &group)
            {
                bindings.push(binding);
            }
        }

        Ok(bindings)
    }

    /// Check if two files are related.
    fn are_related(&self, a: &MediaInfo, b: &MediaInfo) -> bool {
        // Check Live Photo relationship
        if self.detect_live_photo && self.is_live_photo_pair(a, b) {
            return true;
        }

        // Check RAW+JPG relationship
        if self.detect_raw_plus_jpg && self.is_raw_plus_jpg_pair(a, b) {
            return true;
        }

        // Check burst sequence
        if self.detect_burst && self.is_burst_pair(a, b) {
            return true;
        }

        false
    }

    /// Check if files form a Live Photo pair.
    fn is_live_photo_pair(&self, a: &MediaInfo, b: &MediaInfo) -> bool {
        // One must be image, one must be video
        let (img, vid) = match (a.format.category(), b.format.category()) {
            (FormatCategory::Image, FormatCategory::Video) => (a, b),
            (FormatCategory::Video, FormatCategory::Image) => (b, a),
            _ => return false,
        };

        // Image must be HEIC or JPEG
        if !matches!(
            img.format,
            MediaFormat::Heic | MediaFormat::Heif | MediaFormat::Jpeg
        ) {
            return false;
        }

        // Video must be MOV or MP4
        if !matches!(vid.format, MediaFormat::Mov | MediaFormat::Mp4) {
            return false;
        }

        // Check if base names match (ignoring extensions)
        self.base_names_match(&img.path, &vid.path)
    }

    /// Check if files form a RAW+JPG pair.
    fn is_raw_plus_jpg_pair(&self, a: &MediaInfo, b: &MediaInfo) -> bool {
        // One must be RAW, one must be JPG/HEIF
        let (raw, jpg) = if a.format.is_raw() && b.format.is_raw_plus_jpg_primary() {
            (a, b)
        } else if b.format.is_raw() && a.format.is_raw_plus_jpg_primary() {
            (b, a)
        } else {
            return false;
        };

        // Check if base names match
        self.base_names_match(&raw.path, &jpg.path)
    }

    /// Check if files are part of a burst sequence.
    fn is_burst_pair(&self, a: &MediaInfo, b: &MediaInfo) -> bool {
        // Both must be images
        if a.format.category() != FormatCategory::Image
            || b.format.category() != FormatCategory::Image
        {
            return false;
        }

        // Check if base names are burst sequence
        let name_a = a.path.file_stem().and_then(|s| s.to_str());
        let name_b = b.path.file_stem().and_then(|s| s.to_str());

        match (name_a, name_b) {
            (Some(na), Some(nb)) => is_burst_sequence(na, nb),
            _ => false,
        }
    }

    /// Check if base names match (ignoring extensions).
    fn base_names_match(&self, a: &Path, b: &Path) -> bool {
        let stem_a = a.file_stem().and_then(|s| s.to_str());
        let stem_b = b.file_stem().and_then(|s| s.to_str());

        match (stem_a, stem_b) {
            (Some(sa), Some(sb)) => sa == sb,
            _ => false,
        }
    }

    /// Check if a file is a binding candidate (might be part of a pair).
    fn is_binding_candidate(&self, file: &MediaInfo) -> bool {
        if self.detect_live_photo && file.format.is_live_photo_component() {
            return true;
        }
        if self.detect_raw_plus_jpg
            && (file.format.is_raw() || file.format.is_raw_plus_jpg_primary())
        {
            return true;
        }
        false
    }

    /// Create a binding from a group of file indices.
    fn create_binding(&self, files: &[MediaInfo], group: &[usize]) -> Option<MediaBinding> {
        if group.is_empty() {
            return None;
        }

        // Determine binding kind
        let kind = self.determine_binding_kind(files, group);

        // Find primary file
        let primary_idx = self.find_primary(files, group, kind)?;
        let primary = files[primary_idx].path.clone();

        // Collect secondaries
        let mut secondaries: Vec<std::path::PathBuf> = group
            .iter()
            .filter(|&&idx| idx != primary_idx)
            .map(|&idx| files[idx].path.clone())
            .collect();

        // Generate group ID
        let group_id = self.generate_group_id(files, group);

        // Sort secondaries for consistency
        secondaries.sort();

        Some(MediaBinding::new(kind, primary, secondaries, group_id))
    }

    /// Determine the binding kind from file group.
    fn determine_binding_kind(&self, files: &[MediaInfo], group: &[usize]) -> BindingKind {
        let formats: Vec<_> = group.iter().map(|&i| files[i].format).collect();

        // Check for Live Photo
        if self.detect_live_photo {
            let has_img = formats
                .iter()
                .any(|f| f.category() == FormatCategory::Image);
            let has_vid = formats
                .iter()
                .any(|f| f.category() == FormatCategory::Video);
            if has_img && has_vid {
                return BindingKind::LivePhoto;
            }
        }

        // Check for RAW+JPG
        if self.detect_raw_plus_jpg {
            let has_raw = formats.iter().any(|f| f.is_raw());
            let has_jpg = formats.iter().any(|f| f.is_raw_plus_jpg_primary());
            if has_raw && has_jpg {
                return BindingKind::RawPlusJpg;
            }
        }

        // Check for burst
        if self.detect_burst && group.len() > 2 {
            return BindingKind::Burst;
        }

        BindingKind::Unknown
    }

    /// Find the primary file in a group.
    fn find_primary(
        &self,
        files: &[MediaInfo],
        group: &[usize],
        kind: BindingKind,
    ) -> Option<usize> {
        match kind {
            BindingKind::LivePhoto => {
                // Prefer HEIC/JPEG over MOV
                group
                    .iter()
                    .find(|&&idx| {
                        matches!(
                            files[idx].format,
                            MediaFormat::Heic | MediaFormat::Heif | MediaFormat::Jpeg
                        )
                    })
                    .copied()
                    .or_else(|| group.first().copied())
            }
            BindingKind::RawPlusJpg => {
                // Prefer JPEG/HEIF over RAW
                group
                    .iter()
                    .find(|&&idx| files[idx].format.is_raw_plus_jpg_primary())
                    .copied()
                    .or_else(|| group.first().copied())
            }
            BindingKind::Burst => {
                // First file in sequence
                group.first().copied()
            }
            BindingKind::Unknown => group.first().copied(),
        }
    }

    /// Generate a group identifier.
    fn generate_group_id(&self, files: &[MediaInfo], group: &[usize]) -> String {
        // Use the first file's base name as group ID
        if let Some(first) = group.first() {
            files[*first]
                .path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string()
        } else {
            "unknown".to_string()
        }
    }
}

/// Compute binding key for a single file.
pub fn compute_binding_key(info: &MediaInfo) -> Option<super::BindingKey> {
    // Get base name (without extension)
    let base = info.path.file_stem()?.to_str()?.to_string();

    // Determine binding kind from format
    let kind = if info.format.is_live_photo_component() {
        BindingKind::LivePhoto
    } else if info.format.is_raw() || info.format.is_raw_plus_jpg_primary() {
        BindingKind::RawPlusJpg
    } else {
        return None;
    };

    Some(super::BindingKey { base, kind })
}

/// Check if two filenames are burst sequence.
fn is_burst_sequence(a: &str, b: &str) -> bool {
    // Common patterns:
    // IMG_0001.jpg, IMG_0002.jpg
    // DSC_1234.ARW, DSC_1235.ARW
    // _0001234.JPG, _0001235.JPG

    static BURST_RE: OnceLock<Regex> = OnceLock::new();
    let re = BURST_RE.get_or_init(|| Regex::new(r"^(.*?)(\d+)$").unwrap());

    let cap_a = re.captures(a);
    let cap_b = re.captures(b);

    match (cap_a, cap_b) {
        (Some(ca), Some(cb)) => {
            let prefix_a = ca.get(1).map(|m| m.as_str()).unwrap_or("");
            let prefix_b = cb.get(1).map(|m| m.as_str()).unwrap_or("");
            let num_a: u64 = ca[2].parse().unwrap_or(0);
            let num_b: u64 = cb[2].parse().unwrap_or(0);

            prefix_a == prefix_b && num_a.abs_diff(num_b) <= 1
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_burst_detection() {
        assert!(is_burst_sequence("IMG_0001", "IMG_0002"));
        assert!(is_burst_sequence("DSC_1234", "DSC_1235"));
        assert!(!is_burst_sequence("IMG_0001", "DSC_0002"));
        assert!(!is_burst_sequence("IMG_0001", "IMG_0003")); // gap > 1
    }

    #[test]
    fn test_binding_key() {
        // This would need actual files, so we just test the helper
        assert!(is_burst_sequence("photo_001", "photo_002"));
    }
}
