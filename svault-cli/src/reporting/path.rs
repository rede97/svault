//! Shared path helpers for CLI reporting.

use std::path::Path;

#[cfg(target_os = "windows")]
use std::path::{Component, PathBuf};

/// Compute a display-friendly path relative to `base`.
///
/// On Unix this is a thin wrapper around [`Path::strip_prefix`]. On Windows
/// we first try `strip_prefix` and then fall back to case-insensitive
/// component matching so paths that differ only by case still render as
/// relative paths.
pub fn relative_display_path(abs_path: &Path, base: &Path) -> String {
    if let Ok(rel) = abs_path.strip_prefix(base) {
        return rel.display().to_string();
    }

    #[cfg(target_os = "windows")]
    {
        relative_display_path_windows(abs_path, base)
    }

    #[cfg(not(target_os = "windows"))]
    {
        abs_path.display().to_string()
    }
}

#[cfg(target_os = "windows")]
fn relative_display_path_windows(abs_path: &Path, base: &Path) -> String {
    let abs_components: Vec<_> = abs_path.components().collect();
    let base_components: Vec<_> = base.components().collect();

    if base_components.len() > abs_components.len() {
        return abs_path.display().to_string();
    }

    for (base_comp, abs_comp) in base_components.iter().zip(&abs_components) {
        if !components_equal_windows(base_comp, abs_comp) {
            return abs_path.display().to_string();
        }
    }

    let rel_components = &abs_components[base_components.len()..];
    if rel_components.is_empty() {
        return abs_path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| abs_path.display().to_string());
    }

    let mut result = PathBuf::new();
    for component in rel_components {
        result.push(component.as_os_str());
    }
    result.display().to_string()
}

#[cfg(target_os = "windows")]
fn components_equal_windows(left: &Component<'_>, right: &Component<'_>) -> bool {
    match (left, right) {
        (Component::Prefix(prefix_left), Component::Prefix(prefix_right)) => prefix_left
            .as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&prefix_right.as_os_str().to_string_lossy()),
        (Component::RootDir, Component::RootDir) => true,
        _ => left
            .as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&right.as_os_str().to_string_lossy()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_os = "windows")]
    fn computes_relative_path_case_insensitively_on_windows() {
        let result = relative_display_path(
            Path::new(r"C:\USERS\REDE\PICTURES\NIKON TRANSFER 2\subdir\file.jpg"),
            Path::new(r"c:\Users\rede\Pictures\Nikon Transfer 2"),
        );
        assert_eq!(result, r"subdir\file.jpg");
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn falls_back_to_absolute_path_when_base_is_not_a_prefix() {
        let path = Path::new(r"C:\Users\rede\Pictures\file.jpg");
        let result = relative_display_path(path, Path::new(r"D:\Vault"));
        assert_eq!(result, path.display().to_string());
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn computes_relative_path_on_unix() {
        let result = relative_display_path(
            Path::new("/home/user/Pictures/2024-01-01/file.jpg"),
            Path::new("/home/user/Pictures"),
        );
        assert_eq!(result, "2024-01-01/file.jpg");
    }
}
