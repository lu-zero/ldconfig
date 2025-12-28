//! Cache writing API.
//!
//! Provides high-level interface for writing cache files to disk.

use crate::Error;
use std::fs;
use std::io::Write as IoWrite;
use std::path::Path;

/// Cache data ready to be written
pub struct Cache {
    data: Vec<u8>,
}

impl Cache {
    /// Create cache from bytes
    pub(crate) fn from_bytes(data: Vec<u8>) -> Self {
        Self { data }
    }

    /// Write cache to file
    pub fn write_to_file<P: AsRef<Path>>(&self, path: P) -> Result<(), Error> {
        let path = path.as_ref();

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Write cache file
        let mut file = fs::File::create(path)?;
        file.write_all(&self.data)?;
        file.flush()?;
        file.sync_all()?;

        Ok(())
    }

    /// Get cache as bytes
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    /// Get cache size
    pub fn size(&self) -> usize {
        self.data.len()
    }
}
