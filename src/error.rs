// Error types for ldconfig
use std::io;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("ELF parsing error: {0}")]
    Goblin(#[from] goblin::error::Error),

    #[error("Glob pattern error: {0}")]
    Glob(#[from] glob::PatternError),

    #[error("UTF-8 conversion error: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),

    #[error("Invalid cache offset: {0}")]
    InvalidCacheOffset(u32),

    #[error("Invalid UTF-8 in cache string")]
    InvalidCacheUtf8,

    #[error("Invalid UTF-8 in path")]
    InvalidPathUtf8,
}
