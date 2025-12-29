// Error types for ldconfig
use std::io;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("ELF parsing error: {0}")]
    Goblin(#[from] goblin::error::Error),

    #[error("ELF error: {0}")]
    Elf(#[from] crate::elf::Error),

    #[error("Cache write error: {0}")]
    CacheWrite(String),

    #[error("Cache read error: {0}")]
    CacheRead(String),

    #[error("Symlink error: {0}")]
    Symlink(String),

    #[error("Config parsing error: {0}")]
    Config(String),
}
