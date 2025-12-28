// Temporary wrapper around internal::cache_format
// This module will be replaced by reader.rs in a later phase

use crate::internal::cache_format;
use crate::Error;
use std::fs;
use std::path::Path;

pub use crate::internal::cache_format::{CacheEntry, CacheHeader, CacheInfo};

pub fn read_cache_file(path: &Path) -> Result<CacheInfo, Error> {
    let data =
        fs::read(path).map_err(|e| Error::CacheWrite(format!("Failed to read cache: {}", e)))?;

    parse_cache_data(&data)
}

pub fn parse_cache_data(data: &[u8]) -> Result<CacheInfo, Error> {
    cache_format::parse_cache(data)
}
