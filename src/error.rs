// Error types for ldconfig
use std::io;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum LdconfigError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("ELF parsing error: {0}")]
    Elf(#[from] goblin::error::Error),

    #[error("Invalid ELF file: {0}")]
    InvalidElf(String),

    #[error("Missing SONAME in ELF file: {0}")]
    MissingSoname(PathBuf),

    #[error("Cache write error: {0}")]
    CacheWrite(String),

    #[error("Cache read error: {0}")]
    CacheRead(String),

    #[error("Symlink error: {0}")]
    Symlink(String),

    #[error("Config parsing error: {0}")]
    Config(String),

    #[error("ELF validation error: {0}")]
    ElfValidation(#[from] ElfError),
}

#[derive(Debug, thiserror::Error)]
pub enum ElfError {
    #[error("Not a shared object (ET_DYN)")]
    NotSharedObject,

    #[error("Missing PT_DYNAMIC segment")]
    MissingDynamicSegment,

    #[error("Missing DT_SONAME entry")]
    MissingSoname,

    #[error("Empty SONAME")]
    EmptySoname,

    #[error("Unsupported ELF class")]
    UnsupportedClass,

    #[error("Unsupported endianness")]
    UnsupportedEndianness,

    #[error("Unsupported architecture")]
    UnsupportedArchitecture,
}
