// Error types for ldconfig
use std::io;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("Invalid cache file: {0}")]
    InvalidCache(&'static str),

    #[error("Invalid cache offset: {0}")]
    InvalidCacheOffset(u32),

    #[error("Invalid UTF-8 in cache string")]
    InvalidCacheUtf8,
}
