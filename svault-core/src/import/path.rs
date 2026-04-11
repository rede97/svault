//! Path template resolution for import destinations.

use std::path::Path;
use std::path::PathBuf;

use crate::import::exif::secs_to_ymd;

/// Resolve the destination path from the template and file metadata.
/// Supported tokens: `$year`, `$mon`, `$day`, `$device`, `$filename`, `$stem`, `$ext`
pub fn resolve_dest_path(template: &str, rel: &Path, taken_ms: i64, device: &str) -> PathBuf {
    let secs = taken_ms / 1000;
    let (year, month, day) = secs_to_ymd(secs);
    let filename = rel
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = rel
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = rel
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();

    let rendered = template
        .replace("$year", &format!("{year:04}"))
        .replace("$mon", &format!("{month:02}"))
        .replace("$day", &format!("{day:02}"))
        .replace("$device", device)
        .replace("$filename", &filename)
        .replace("$stem", &stem)
        .replace("$ext", &ext);

    PathBuf::from(rendered)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_dest_path() {
        let template = "$year/$mon-$day/$device/$filename";
        let rel = Path::new("IMG_001.jpg");
        let taken_ms = 1714552800000; // 2024-05-01 10:00:00
        let device = "Apple iPhone 15";

        let result = resolve_dest_path(template, rel, taken_ms, device);
        assert_eq!(
            result,
            PathBuf::from("2024/05-01/Apple iPhone 15/IMG_001.jpg")
        );
    }

    #[test]
    fn test_resolve_dest_path_no_device() {
        let template = "$year/$mon-$day/$device/$filename";
        let rel = Path::new("photo.jpg");
        let taken_ms = 1714552800000;
        let device = "Unknown";

        let result = resolve_dest_path(template, rel, taken_ms, device);
        assert_eq!(result, PathBuf::from("2024/05-01/Unknown/photo.jpg"));
    }
}
