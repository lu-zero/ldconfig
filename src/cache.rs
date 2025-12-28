// Temporary wrapper around internal::cache_format
// This module will be replaced by writer.rs in a later phase

use crate::internal::cache_format;
use crate::internal::elf::ElfLibrary;
use camino::Utf8Path;

pub use crate::internal::cache_format::CacheEntry;

pub fn build_cache(libraries: &[ElfLibrary], prefix: Option<&Utf8Path>) -> Vec<u8> {
    cache_format::build_cache(libraries, prefix)
}
