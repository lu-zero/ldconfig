//! Low-level cache binary format implementation.
//!
//! This module handles the binary format of ld.so.cache files, including:
//! - Architecture-specific flags
//! - Cache header and entry structures
//! - Binary serialization and deserialization
//! - Extension section handling

use crate::elf::{ElfArch, ElfLibrary};
use crate::error::Error;
use camino::{Utf8Path, Utf8PathBuf};
use std::cmp::Ordering;
use std::collections::HashMap;
use tracing::{trace, warn};

pub(crate) const CACHE_MAGIC: [u8; 20] = *b"glibc-ld.so.cache1.1";

// Flag constants from glibc sysdeps/generic/ldconfig.h
// https://sourceware.org/git/?p=glibc.git;a=blob;f=sysdeps/generic/ldconfig.h
#[allow(dead_code)]
pub(crate) const FLAG_TYPE_MASK: u32 = 0x00ff;
pub(crate) const FLAG_ELF_LIBC6: u32 = 0x0003;
#[allow(dead_code)]
pub(crate) const FLAG_SPARC_LIB64: u32 = 0x0100;
pub(crate) const FLAG_X8664_LIB64: u32 = 0x0300;
#[allow(dead_code)]
pub(crate) const FLAG_S390_LIB64: u32 = 0x0400;
pub(crate) const FLAG_POWERPC_LIB64: u32 = 0x0500;
#[allow(dead_code)]
pub(crate) const FLAG_MIPS64_LIBN32: u32 = 0x0600;
#[allow(dead_code)]
pub(crate) const FLAG_MIPS64_LIBN64: u32 = 0x0700;
#[allow(dead_code)]
pub(crate) const FLAG_X8664_LIBX32: u32 = 0x0800;
pub(crate) const FLAG_ARM_LIBHF: u32 = 0x0900;
pub(crate) const FLAG_AARCH64_LIB64: u32 = 0x0a00;
#[allow(dead_code)]
pub(crate) const FLAG_ARM_LIBSF: u32 = 0x0b00;
#[allow(dead_code)]
pub(crate) const FLAG_MIPS_LIB32_NAN2008: u32 = 0x0c00;
#[allow(dead_code)]
pub(crate) const FLAG_MIPS64_LIBN32_NAN2008: u32 = 0x0d00;
#[allow(dead_code)]
pub(crate) const FLAG_MIPS64_LIBN64_NAN2008: u32 = 0x0e00;
#[allow(dead_code)]
pub(crate) const FLAG_RISCV_FLOAT_ABI_SOFT: u32 = 0x0f00;
pub(crate) const FLAG_RISCV_FLOAT_ABI_DOUBLE: u32 = 0x1000; // RISC-V lp64d (double-precision FP)
#[allow(dead_code)]
pub(crate) const FLAG_LARCH_FLOAT_ABI_SOFT: u32 = 0x1100;
#[allow(dead_code)]
pub(crate) const FLAG_LARCH_FLOAT_ABI_DOUBLE: u32 = 0x1200;

const EXTENSION_MAGIC: u32 = 0xEAA42174;

#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub flags: u32,
    pub key_offset: u32,
    pub value_offset: u32,
    pub osversion: u32,
    pub hwcap: u64,
}

#[derive(Debug, Clone)]
pub struct CacheInfo {
    pub entries: Vec<CacheEntry>,
    pub generator: Option<String>,
}

/// Build cache binary data from library list
pub(crate) fn build_cache(libraries: &[ElfLibrary], prefix: &Utf8Path) -> Vec<u8> {
    let has_prefix = prefix != "/";
    let mut cache = Vec::new();

    // Header: magic (20 bytes)
    cache.extend_from_slice(&CACHE_MAGIC);

    // Header: nlibs (4 bytes) - placeholder
    let nlibs_pos = cache.len();
    cache.extend_from_slice(&0u32.to_ne_bytes());

    // Header: len_strings (4 bytes) - placeholder
    let len_strings_pos = cache.len();
    cache.extend_from_slice(&0u32.to_ne_bytes());

    // Header: flags (1 byte) - endianness flag
    // Values: 0 = unset, 1 = invalid, 2 = little endian, 3 = big endian
    let flags: u8 = if cfg!(target_endian = "little") { 2 } else { 3 };
    cache.push(flags);

    // Header: padding (3 bytes) - alignment
    cache.extend_from_slice(&[0u8; 3]);

    // Header: extension_offset (4 bytes) - offset to extension section (0 = no extensions)
    cache.extend_from_slice(&0u32.to_ne_bytes());

    // Header: unused[3] (12 bytes) - actual unused padding
    cache.extend_from_slice(&[0u8; 12]);

    // Sort matching glibc's compare() in cache.c:
    // Primary: reverse dl_cache_libcmp (swapped args)
    // Secondary: flags descending
    let mut sorted_libs = libraries.to_vec();
    sorted_libs.sort_by(|a, b| {
        let filename_a = a.path.file_name().unwrap_or(a.path.as_str());
        let filename_b = b.path.file_name().unwrap_or(b.path.as_str());
        let res = dl_cache_libcmp(filename_b, filename_a);
        if res != Ordering::Equal {
            return res;
        }
        let flags_a = arch_to_flags(a.arch, a.is_64bit, a.is_hardfloat);
        let flags_b = arch_to_flags(b.arch, b.is_64bit, b.is_hardfloat);
        flags_b.cmp(&flags_a)
    });

    // Pre-compute cache paths for each library (filename, absolute_path)
    let lib_paths: Vec<(&str, String)> = sorted_libs
        .iter()
        .map(|lib| {
            let filename = lib.path.file_name().unwrap_or(lib.path.as_str());
            let cache_path = cache_path_for_lib(&lib.path, prefix, has_prefix);
            (filename, cache_path)
        })
        .collect();

    // Build string table
    let mut string_table = Vec::new();
    let mut string_offsets = HashMap::new();

    for (filename, cache_path) in &lib_paths {
        add_string(&mut string_table, &mut string_offsets, filename);
        add_string(&mut string_table, &mut string_offsets, cache_path);
    }

    // Calculate where string table will be in the final file
    // Header = 48 bytes, entries = nlibs * 24 bytes
    let string_table_file_offset = 48 + (sorted_libs.len() * 24);

    // Build entries
    for (i, lib) in sorted_libs.iter().enumerate() {
        let (filename, cache_path) = &lib_paths[i];

        let key_relative_offset = *string_offsets.get(*filename).unwrap_or_else(|| {
            warn!("Filename '{}' not found in string offsets map!", filename);
            &0u32
        });
        let value_relative_offset = *string_offsets.get(cache_path).unwrap_or_else(|| {
            warn!("Path '{}' not found in string offsets map!", cache_path);
            &0u32
        });

        let key_offset = (string_table_file_offset as u32) + key_relative_offset;
        let value_offset = (string_table_file_offset as u32) + value_relative_offset;
        let flags = arch_to_flags(lib.arch, lib.is_64bit, lib.is_hardfloat);

        cache.extend_from_slice(&flags.to_ne_bytes());
        cache.extend_from_slice(&key_offset.to_ne_bytes());
        cache.extend_from_slice(&value_offset.to_ne_bytes());
        cache.extend_from_slice(&lib.osversion.to_ne_bytes());
        cache.extend_from_slice(&lib.hwcap.unwrap_or(0).to_ne_bytes());
    }

    // Append string table
    cache.extend_from_slice(&string_table);

    // Add padding to align extension section to 4 bytes
    while cache.len() % 4 != 0 {
        cache.push(0);
    }

    // Add extension section with generator information
    let extension_offset = cache.len() as u32;

    cache.extend_from_slice(&EXTENSION_MAGIC.to_ne_bytes());
    cache.extend_from_slice(&1u32.to_ne_bytes()); // 1 extension

    // Calculate where the generator data will be
    let generator_data_offset = extension_offset + 4 + 4 + 16;

    // Generator string (include version from Cargo.toml)
    let generator = format!("ldconfig-rs {}", env!("CARGO_PKG_VERSION"));
    let generator_bytes = generator.as_bytes();

    // Extension section descriptor
    cache.extend_from_slice(&0u32.to_ne_bytes()); // tag: 0 (generator)
    cache.extend_from_slice(&0u32.to_ne_bytes()); // flags: 0
    cache.extend_from_slice(&generator_data_offset.to_ne_bytes()); // offset to data
    cache.extend_from_slice(&(generator_bytes.len() as u32).to_ne_bytes()); // size

    // Append the actual generator string (null-terminated)
    cache.extend_from_slice(generator_bytes);
    cache.push(0);

    // Update placeholders in header
    let nlibs = sorted_libs.len() as u32;
    let len_strings = string_table.len() as u32;

    cache[nlibs_pos..nlibs_pos + 4].copy_from_slice(&nlibs.to_ne_bytes());
    cache[len_strings_pos..len_strings_pos + 4].copy_from_slice(&len_strings.to_ne_bytes());
    cache[32..36].copy_from_slice(&extension_offset.to_ne_bytes());

    cache
}

/// Parse cache binary data
pub(crate) fn parse_cache(data: &[u8]) -> Result<CacheInfo, Error> {
    // Parse header
    let magic = String::from_utf8_lossy(&data[..20]).to_string();
    let nlibs = u32::from_ne_bytes([data[20], data[21], data[22], data[23]]);
    let len_strings = u32::from_ne_bytes([data[24], data[25], data[26], data[27]]);

    trace!("magic {magic}, nlibs {nlibs}");

    // Parse entries
    let mut entries = Vec::new();
    let header_size = 48;
    let entry_size = 24;

    for i in 0..nlibs {
        let offset = header_size + (i as usize * entry_size);
        let flags = u32::from_ne_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]);
        let key_offset = u32::from_ne_bytes([
            data[offset + 4],
            data[offset + 5],
            data[offset + 6],
            data[offset + 7],
        ]);
        let value_offset = u32::from_ne_bytes([
            data[offset + 8],
            data[offset + 9],
            data[offset + 10],
            data[offset + 11],
        ]);
        let osversion = u32::from_ne_bytes([
            data[offset + 12],
            data[offset + 13],
            data[offset + 14],
            data[offset + 15],
        ]);
        let hwcap = u64::from_ne_bytes([
            data[offset + 16],
            data[offset + 17],
            data[offset + 18],
            data[offset + 19],
            data[offset + 20],
            data[offset + 21],
            data[offset + 22],
            data[offset + 23],
        ]);

        entries.push(CacheEntry {
            flags,
            key_offset,
            value_offset,
            osversion,
            hwcap,
        });
    }

    // Parse string table
    let string_table_start = header_size + (nlibs as usize * entry_size);
    let string_table_end = std::cmp::min(string_table_start + len_strings as usize, data.len());
    let string_data = &data[string_table_start..string_table_end];

    let mut strings = Vec::new();
    let mut start = 0;
    for i in 0..string_data.len() {
        if string_data[i] == 0 {
            if i > start {
                let s = String::from_utf8_lossy(&string_data[start..i]).to_string();
                strings.push(s);
            }
            start = i + 1;
        }
    }

    // Parse extension section
    let extension_offset = u32::from_ne_bytes([data[32], data[33], data[34], data[35]]) as usize;
    let mut generator = None;

    if extension_offset > 0 && extension_offset + 24 <= data.len() {
        let ext_magic = u32::from_ne_bytes([
            data[extension_offset],
            data[extension_offset + 1],
            data[extension_offset + 2],
            data[extension_offset + 3],
        ]);

        if ext_magic == EXTENSION_MAGIC {
            let ext_count = u32::from_ne_bytes([
                data[extension_offset + 4],
                data[extension_offset + 5],
                data[extension_offset + 6],
                data[extension_offset + 7],
            ]);

            for i in 0..ext_count as usize {
                let section_offset = extension_offset + 8 + (i * 16);
                if section_offset + 16 <= data.len() {
                    let tag = u32::from_ne_bytes([
                        data[section_offset],
                        data[section_offset + 1],
                        data[section_offset + 2],
                        data[section_offset + 3],
                    ]);
                    let data_offset = u32::from_ne_bytes([
                        data[section_offset + 8],
                        data[section_offset + 9],
                        data[section_offset + 10],
                        data[section_offset + 11],
                    ]) as usize;
                    let data_size = u32::from_ne_bytes([
                        data[section_offset + 12],
                        data[section_offset + 13],
                        data[section_offset + 14],
                        data[section_offset + 15],
                    ]) as usize;

                    // Tag 0 = generator
                    if tag == 0 && data_offset + data_size <= data.len() {
                        generator = Some(
                            String::from_utf8_lossy(&data[data_offset..data_offset + data_size])
                                .to_string(),
                        );
                    }
                }
            }
        }
    }

    let info = CacheInfo { entries, generator };

    Ok(info)
}

/// Convert architecture to cache flags
pub(crate) fn arch_to_flags(arch: ElfArch, is_64bit: bool, is_hardfloat: bool) -> u32 {
    match arch {
        ElfArch::X86_64 => {
            if is_64bit {
                FLAG_X8664_LIB64 | FLAG_ELF_LIBC6
            } else {
                FLAG_ELF_LIBC6
            }
        }
        ElfArch::AArch64 => FLAG_AARCH64_LIB64 | FLAG_ELF_LIBC6,
        ElfArch::RiscV64 => FLAG_RISCV_FLOAT_ABI_DOUBLE | FLAG_ELF_LIBC6,
        ElfArch::PowerPC64 => FLAG_POWERPC_LIB64 | FLAG_ELF_LIBC6,
        ElfArch::I686 => FLAG_ELF_LIBC6,
        ElfArch::ARM => {
            if is_hardfloat {
                FLAG_ARM_LIBHF | FLAG_ELF_LIBC6
            } else {
                FLAG_ELF_LIBC6
            }
        }
    }
}

/// Compute the cache path for a library: canonicalize directory, strip prefix.
fn cache_path_for_lib(lib_path: &Utf8Path, prefix: &Utf8Path, has_prefix: bool) -> String {
    let dir = lib_path.parent().unwrap_or_else(|| Utf8Path::new(""));
    let filename = lib_path.file_name().unwrap_or(lib_path.as_str());

    let canonical_dir = dir
        .as_std_path()
        .canonicalize()
        .ok()
        .and_then(|p| Utf8PathBuf::try_from(p).ok())
        .unwrap_or_else(|| dir.to_path_buf());

    let canonical_path = canonical_dir.join(filename);

    if has_prefix {
        let canonical_prefix = prefix
            .as_std_path()
            .canonicalize()
            .ok()
            .and_then(|p| Utf8PathBuf::try_from(p).ok())
            .unwrap_or_else(|| prefix.to_path_buf());

        if let Ok(stripped) = canonical_path.strip_prefix(&canonical_prefix) {
            format!("/{}", stripped)
        } else {
            canonical_path.to_string()
        }
    } else {
        canonical_path.to_string()
    }
}

/// Numeric-aware string comparison matching glibc's `_dl_cache_libcmp`.
/// Digits sort after non-digits; runs of digits compare numerically.
fn dl_cache_libcmp(p1: &str, p2: &str) -> Ordering {
    let b1 = p1.as_bytes();
    let b2 = p2.as_bytes();
    let mut i = 0;
    let mut j = 0;

    while i < b1.len() {
        if b1[i].is_ascii_digit() {
            if j < b2.len() && b2[j].is_ascii_digit() {
                // Both digits: compare numerically
                let mut val1: i32 = 0;
                let mut val2: i32 = 0;
                while i < b1.len() && b1[i].is_ascii_digit() {
                    val1 = val1 * 10 + (b1[i] - b'0') as i32;
                    i += 1;
                }
                while j < b2.len() && b2[j].is_ascii_digit() {
                    val2 = val2 * 10 + (b2[j] - b'0') as i32;
                    j += 1;
                }
                if val1 != val2 {
                    return val1.cmp(&val2);
                }
            } else {
                // p1 digit, p2 non-digit: digits sort after non-digits
                return Ordering::Greater;
            }
        } else if j < b2.len() && b2[j].is_ascii_digit() {
            return Ordering::Less;
        } else if j >= b2.len() || b1[i] != b2[j] {
            return b1.get(i).unwrap_or(&0).cmp(b2.get(j).unwrap_or(&0));
        } else {
            i += 1;
            j += 1;
        }
    }
    // p1 ended: compare NUL (0) vs p2's current char
    0u8.cmp(b2.get(j).unwrap_or(&0))
}

fn add_string(table: &mut Vec<u8>, offsets: &mut HashMap<String, u32>, string: &str) {
    if !offsets.contains_key(string) {
        let offset = table.len() as u32;
        offsets.insert(string.to_string(), offset);
        table.extend_from_slice(string.as_bytes());
        table.push(0); // NUL terminator
    }
}
