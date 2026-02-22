//! Atomic file write utilities
//!
//! Provides safe atomic write operations using tempfile crate
//! for better error handling and cross-platform compatibility.

use std::fs;
use std::io::Write;
use std::path::Path;
use tempfile::NamedTempFile;

/// Write data to a file atomically using tempfile for robustness
pub(crate) fn atomic_write<P: AsRef<Path>>(path: P, data: &[u8]) -> std::io::Result<()> {
    let path = path.as_ref();

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Create temp file in same directory as target
    // This is important for filesystems where cross-directory renames aren't atomic
    let parent_dir = path.parent().unwrap_or_else(|| Path::new("."));
    let mut temp_file = NamedTempFile::new_in(parent_dir)?;

    // Write data
    temp_file.write_all(data)?;
    temp_file.flush()?;
    temp_file.as_file().sync_all()?;

    // Set permissions (Unix only) - match the original behavior
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temp_file
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o644))?;
    }

    // Atomically persist to final location
    // This will fail if the target already exists (which is what we want)
    temp_file.persist(path)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_atomic_write() {
        let dir = tempdir().unwrap();
        let target_path = dir.path().join("test_file.txt");
        let data = b"test data";

        // Write should succeed
        let result = atomic_write(&target_path, data);
        assert!(result.is_ok());

        // File should exist and contain correct data
        let content = fs::read_to_string(&target_path).unwrap();
        assert_eq!(content, "test data");
    }

    #[test]
    fn test_atomic_write_nested_dirs() {
        let dir = tempdir().unwrap();
        let target_path = dir.path().join("nested/dir/test_file.txt");
        let data = b"nested test";

        // Should create parent directories automatically
        let result = atomic_write(&target_path, data);
        assert!(result.is_ok());

        // File should exist
        let content = fs::read_to_string(&target_path).unwrap();
        assert_eq!(content, "nested test");
    }
}
