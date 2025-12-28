// ldconfig - Portable Rust implementation
// MIT OR Apache-2.0, 2025

//! A portable Rust implementation of ldconfig for managing dynamic linker cache files.
//!
//! This library provides high-level APIs for:
//! - Reading and exploring ld.so.cache files
//! - Parsing ld.so.conf configuration files
//! - Building cache files by scanning library directories
//! - Writing cache files to disk
//!
//! # Example: Read a cache file
//!
//! ```no_run
//! use ldconfig::Cache;
//!
//! let cache = Cache::from_file("/etc/ld.so.cache")?;
//! cache.print(&mut std::io::stdout())?;
//! # Ok::<(), ldconfig::Error>(())
//! ```
//!
//! # Example: Build and write a cache
//!
//! ```no_run
//! use ldconfig::{LibraryConfig, CacheBuilder, ScanOptions};
//!
//! let config = LibraryConfig::from_file("/etc/ld.so.conf", None)?;
//! let cache = CacheBuilder::new()
//!     .scan_directories(&config, &ScanOptions::default())?
//!     .build()?;
//! cache.write_to_file("/etc/ld.so.cache")?;
//! # Ok::<(), ldconfig::Error>(())
//! ```

mod internal;

pub mod builder;
pub mod cache;
pub mod config;
pub mod error;

// Backwards compatibility - old modules (will be removed in future version)
#[doc(hidden)]
pub mod cache_reader;
#[doc(hidden)]
pub mod reader;
#[doc(hidden)]
pub mod writer;

// Main public API exports
pub use builder::{CacheBuilder, ScanOptions};
pub use cache::{Cache, CacheEntry, CacheInfo};
pub use config::LibraryConfig;
pub use error::Error;

// Backwards compatibility exports (will be removed in future version)
#[doc(hidden)]
pub use cache::build_cache;
#[doc(hidden)]
pub use cache_reader::{parse_cache_data, read_cache_file};
#[doc(hidden)]
pub use config::{
    expand_includes_compat as expand_includes, parse_config_content_compat as parse_config_content,
    parse_config_file_compat as parse_config_file, Config,
};
#[doc(hidden)]
pub use internal::elf::{parse_elf_file, ElfArch, ElfLibrary};
#[doc(hidden)]
pub use internal::hwcap::{detect_hwcap_dirs, scan_hwcap_libraries, HwCap};
#[doc(hidden)]
pub use internal::scanner::{
    deduplicate_libraries, deduplicate_scan_directories, is_dso, scan_all_libraries,
    should_include_symlink, should_scan_library,
};
#[doc(hidden)]
pub use internal::symlinks::{create_symlink, update_symlinks, SymlinkAction, SymlinkActionType};

// Backwards compat aliases
#[doc(hidden)]
pub use cache::Cache as CacheReader;
