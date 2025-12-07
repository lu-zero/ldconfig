// ldconfig - Portable Rust implementation
// MIT OR Apache-2.0, 2025

pub mod cache;
pub mod cache_reader;
pub mod config;
pub mod elf;
pub mod error;
pub mod hwcap;
pub mod symlinks;

pub use cache::{build_cache, CacheEntry};
pub use cache_reader::{parse_cache_data, read_cache_file, CacheInfo};
pub use config::{expand_includes, parse_config_content, parse_config_file, Config};
pub use elf::{parse_elf_file, ElfArch, ElfLibrary};
pub use error::{ElfError, LdconfigError};
pub use hwcap::{detect_hwcap_dirs, scan_hwcap_libraries, HwCap};
pub use symlinks::{create_symlink, update_symlinks, SymlinkAction, SymlinkActionType};
