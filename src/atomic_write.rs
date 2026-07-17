//! Atomic file write via tempfile + rename.

use std::fs;
use std::io::Write;
use std::path::Path;
use tempfile::NamedTempFile;

/// Write data to a file atomically. The parent directory must exist,
/// like glibc's ldconfig which errors out on a missing cache directory.
pub(crate) fn atomic_write<P: AsRef<Path>>(path: P, data: &[u8]) -> std::io::Result<()> {
    let path = path.as_ref();

    // The temp file must live in the target's directory so the final
    // rename stays on one filesystem.
    let parent_dir = path.parent().unwrap_or_else(|| Path::new("."));
    let mut temp_file = NamedTempFile::new_in(parent_dir)?;

    temp_file.write_all(data)?;
    temp_file.flush()?;
    temp_file.as_file().sync_all()?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temp_file
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o644))?;
    }

    // Atomically replace the target via rename(2).
    temp_file.persist(path)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn writes_atomically() {
        let dir = tempdir().unwrap();
        let target_path = dir.path().join("test_file.txt");

        atomic_write(&target_path, b"test data").unwrap();
        assert_eq!(fs::read_to_string(&target_path).unwrap(), "test data");
    }

    #[test]
    fn missing_directory_errors() {
        let dir = tempdir().unwrap();
        let target_path = dir.path().join("nested/dir/test_file.txt");

        assert!(atomic_write(&target_path, b"x").is_err());
        assert!(!dir.path().join("nested").exists());
    }
}
