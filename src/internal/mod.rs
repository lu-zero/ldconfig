//! Internal implementation modules.
//!
//! These modules contain the low-level implementation details and are not part
//! of the public API. They are accessible within the crate using `pub(crate)`.

pub(crate) mod cache_format;
pub(crate) mod elf;
pub(crate) mod hwcap;
pub(crate) mod scanner;
pub(crate) mod symlinks;
