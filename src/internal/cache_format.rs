//! Low-level cache binary format implementation.
//!
//! This module handles the binary format of ld.so.cache files, including:
//! - Architecture-specific flags
//! - Cache header and entry structures
//! - Binary serialization and deserialization
//! - Extension section handling

use crate::internal::elf::{ElfArch, ElfLibrary};
use crate::Error;
use camino::{Utf8Path, Utf8PathBuf};
use std::collections::HashMap;

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
pub struct CacheHeader {
    pub magic: String,
    pub nlibs: u32,
    pub len_strings: u32,
}

#[derive(Debug, Clone)]
pub struct CacheInfo {
    pub header: CacheHeader,
    pub entries: Vec<CacheEntry>,
    pub string_table: Vec<String>,
    pub generator: Option<String>,
}

/// Build cache binary data from library list
pub(crate) fn build_cache(libraries: &[ElfLibrary], prefix: Option<&Utf8Path>) -> Vec<u8> {
    let mut cache = Vec::new();

    // Header: magic (20 bytes)
    cache.extend_from_slice(&CACHE_MAGIC);

    // Header: nlibs (4 bytes) - placeholder
    let nlibs_pos = cache.len();
    cache.extend_from_slice(&0u32.to_le_bytes());

    // Header: len_strings (4 bytes) - placeholder
    let len_strings_pos = cache.len();
    cache.extend_from_slice(&0u32.to_le_bytes());

    // Header: flags (1 byte) - endianness flag
    // Values: 0 = unset, 1 = invalid, 2 = little endian, 3 = big endian
    let flags: u8 = if cfg!(target_endian = "little") { 2 } else { 3 };
    cache.push(flags);

    // Header: padding (3 bytes) - alignment
    cache.extend_from_slice(&[0u8; 3]);

    // Header: extension_offset (4 bytes) - offset to extension section (0 = no extensions)
    cache.extend_from_slice(&0u32.to_le_bytes());

    // Header: unused[3] (12 bytes) - actual unused padding
    cache.extend_from_slice(&[0u8; 12]);

    // Sort libraries FIRST before building anything
    // Primary: filename in REVERSE alphabetical order (glibc behavior)
    //          This puts libfoo.so.1 BEFORE libfoo.so
    // Secondary: hwcap priority (higher hwcap = more specialized, comes first)
    let mut sorted_libs = libraries.to_vec();
    sorted_libs.sort_by(|a, b| {
        let filename_a = a.path.file_name().unwrap_or(a.path.as_str());
        let filename_b = b.path.file_name().unwrap_or(b.path.as_str());
        match filename_b.cmp(filename_a) {
            // REVERSED: b.cmp(a) instead of a.cmp(b)
            std::cmp::Ordering::Equal => {
                // Higher hwcap comes first (more specialized)
                b.hwcap.unwrap_or(0).cmp(&a.hwcap.unwrap_or(0))
            }
            other => other,
        }
    });

    // Build string table from SORTED libraries
    let mut string_table = Vec::new();
    let mut string_offsets = HashMap::new();

    for lib in &sorted_libs {
        // Use filename as the cache key
        // This allows lookups by any symlink name (libfoo.so, libfoo.so.1, etc.)
        let filename = lib.path.file_name().unwrap_or(lib.path.as_str());
        add_string(&mut string_table, &mut string_offsets, filename);

        // Convert the path to an absolute path for the cache
        // The real ldconfig uses absolute paths in the cache
        // Canonicalize the DIRECTORY only (not the filename symlink)
        let dir = lib.path.parent().unwrap_or_else(|| Utf8Path::new(""));
        let filename_part = lib.path.file_name().unwrap_or(lib.path.as_str());

        let canonical_dir = dir
            .as_std_path()
            .canonicalize()
            .ok()
            .and_then(|p| Utf8PathBuf::try_from(p).ok())
            .unwrap_or_else(|| dir.to_path_buf());

        let canonical_path = canonical_dir.join(filename_part);

        let path_to_add = if let Some(prefix) = prefix {
            // Get canonical prefix for comparison
            let canonical_prefix = prefix
                .as_std_path()
                .canonicalize()
                .ok()
                .and_then(|p| Utf8PathBuf::try_from(p).ok())
                .unwrap_or_else(|| prefix.to_path_buf());

            if let Ok(stripped) = canonical_path.strip_prefix(&canonical_prefix) {
                // Convert to absolute path by prepending '/'
                format!("/{}", stripped)
            } else {
                canonical_path.to_string()
            }
        } else {
            canonical_path.to_string()
        };

        add_string(&mut string_table, &mut string_offsets, &path_to_add);
    }

    // Calculate where string table will be in the final file
    // Header = 48 bytes, entries = nlibs * 24 bytes
    let string_table_file_offset = 48 + (sorted_libs.len() * 24);

    // Build entries
    for lib in &sorted_libs {
        // Look up string offsets for filename and path (these are relative to string table start)
        let filename = lib.path.file_name().unwrap_or(lib.path.as_str());
        let key_relative_offset = *string_offsets.get(filename).unwrap_or_else(|| {
            eprintln!(
                "WARNING: Filename '{}' not found in string offsets map!",
                filename
            );
            &0u32
        });

        // Convert the path to an absolute path for the cache (same logic as above)
        let dir = lib.path.parent().unwrap_or_else(|| Utf8Path::new(""));
        let filename_part = lib.path.file_name().unwrap_or(lib.path.as_str());

        let canonical_dir = dir
            .as_std_path()
            .canonicalize()
            .ok()
            .and_then(|p| Utf8PathBuf::try_from(p).ok())
            .unwrap_or_else(|| dir.to_path_buf());

        let canonical_path = canonical_dir.join(filename_part);

        let path_to_add = if let Some(prefix) = prefix {
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
        };

        let value_relative_offset = *string_offsets.get(&path_to_add).unwrap_or_else(|| {
            eprintln!(
                "WARNING: PATH '{}' not found in string offsets map!",
                path_to_add
            );
            &0u32
        });

        // Convert to ABSOLUTE file offsets (glibc expects absolute offsets)
        let key_offset = (string_table_file_offset as u32) + key_relative_offset;
        let value_offset = (string_table_file_offset as u32) + value_relative_offset;

        // Calculate flags using glibc ldconfig.h constants
        let flags = arch_to_flags(lib.arch, lib.is_64bit, lib.is_hardfloat);

        let entry = CacheEntry {
            flags,
            key_offset,
            value_offset,
            osversion: lib.osversion,
            hwcap: lib.hwcap.unwrap_or(0),
        };

        // Write entry in little-endian format (24 bytes total)
        cache.extend_from_slice(&entry.flags.to_le_bytes());
        cache.extend_from_slice(&entry.key_offset.to_le_bytes());
        cache.extend_from_slice(&entry.value_offset.to_le_bytes());
        cache.extend_from_slice(&entry.osversion.to_le_bytes());
        cache.extend_from_slice(&entry.hwcap.to_le_bytes());
    }

    // Append string table
    cache.extend_from_slice(&string_table);

    // Add padding to align extension section to 4 bytes
    while cache.len() % 4 != 0 {
        cache.push(0);
    }

    // Add extension section with generator information
    let extension_offset = cache.len() as u32;

    cache.extend_from_slice(&EXTENSION_MAGIC.to_le_bytes());
    cache.extend_from_slice(&1u32.to_le_bytes()); // 1 extension

    // Calculate where the generator data will be
    let generator_data_offset = extension_offset + 4 + 4 + 16;

    // Generator string (include version from Cargo.toml)
    let generator = format!("ldconfig-rs {}", env!("CARGO_PKG_VERSION"));
    let generator_bytes = generator.as_bytes();

    // Extension section descriptor
    cache.extend_from_slice(&0u32.to_le_bytes()); // tag: 0 (generator)
    cache.extend_from_slice(&0u32.to_le_bytes()); // flags: 0
    cache.extend_from_slice(&generator_data_offset.to_le_bytes()); // offset to data
    cache.extend_from_slice(&(generator_bytes.len() as u32).to_le_bytes()); // size

    // Append the actual generator string (null-terminated)
    cache.extend_from_slice(generator_bytes);
    cache.push(0);

    // Update placeholders in header
    let nlibs = sorted_libs.len() as u32;
    let len_strings = string_table.len() as u32;

    cache[nlibs_pos..nlibs_pos + 4].copy_from_slice(&nlibs.to_le_bytes());
    cache[len_strings_pos..len_strings_pos + 4].copy_from_slice(&len_strings.to_le_bytes());
    cache[32..36].copy_from_slice(&extension_offset.to_le_bytes());

    cache
}

/// Parse cache binary data
pub(crate) fn parse_cache(data: &[u8]) -> Result<CacheInfo, Error> {
    // Parse header
    let magic = String::from_utf8_lossy(&data[..20]).to_string();
    let nlibs = u32::from_ne_bytes([data[20], data[21], data[22], data[23]]);
    let len_strings = u32::from_ne_bytes([data[24], data[25], data[26], data[27]]);

    let header = CacheHeader {
        magic,
        nlibs,
        len_strings,
    };

    // Parse entries
    let mut entries = Vec::new();
    let header_size = 48;
    let entry_size = 24;

    for i in 0..nlibs {
        let offset = header_size + (i as usize * entry_size);
        let flags = u32::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]);
        let key_offset = u32::from_le_bytes([
            data[offset + 4],
            data[offset + 5],
            data[offset + 6],
            data[offset + 7],
        ]);
        let value_offset = u32::from_le_bytes([
            data[offset + 8],
            data[offset + 9],
            data[offset + 10],
            data[offset + 11],
        ]);
        let osversion = u32::from_le_bytes([
            data[offset + 12],
            data[offset + 13],
            data[offset + 14],
            data[offset + 15],
        ]);
        let hwcap = u64::from_le_bytes([
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
    let extension_offset = u32::from_le_bytes([data[32], data[33], data[34], data[35]]) as usize;
    let mut generator = None;

    if extension_offset > 0 && extension_offset + 24 <= data.len() {
        let ext_magic = u32::from_le_bytes([
            data[extension_offset],
            data[extension_offset + 1],
            data[extension_offset + 2],
            data[extension_offset + 3],
        ]);

        if ext_magic == EXTENSION_MAGIC {
            let ext_count = u32::from_le_bytes([
                data[extension_offset + 4],
                data[extension_offset + 5],
                data[extension_offset + 6],
                data[extension_offset + 7],
            ]);

            for i in 0..ext_count as usize {
                let section_offset = extension_offset + 8 + (i * 16);
                if section_offset + 16 <= data.len() {
                    let tag = u32::from_le_bytes([
                        data[section_offset],
                        data[section_offset + 1],
                        data[section_offset + 2],
                        data[section_offset + 3],
                    ]);
                    let data_offset = u32::from_le_bytes([
                        data[section_offset + 8],
                        data[section_offset + 9],
                        data[section_offset + 10],
                        data[section_offset + 11],
                    ]) as usize;
                    let data_size = u32::from_le_bytes([
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

    Ok(CacheInfo {
        header,
        entries,
        string_table: strings,
        generator,
    })
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

/// Convert flags to architecture string
pub(crate) fn flags_to_arch_string(flags: u32) -> &'static str {
    let arch_flag = flags & 0xff00;
    match arch_flag {
        FLAG_X8664_LIB64 => "x86-64",
        FLAG_AARCH64_LIB64 => "AArch64",
        FLAG_RISCV_FLOAT_ABI_DOUBLE => "RISC-V 64-bit (lp64d)",
        FLAG_POWERPC_LIB64 => "PowerPC 64-bit",
        FLAG_ARM_LIBHF => "ARM hard-float",
        FLAG_ARM_LIBSF => "ARM soft-float",
        FLAG_SPARC_LIB64 => "SPARC 64-bit",
        FLAG_S390_LIB64 => "S390 64-bit",
        FLAG_MIPS64_LIBN32 => "MIPS N32",
        FLAG_MIPS64_LIBN64 => "MIPS 64-bit",
        FLAG_X8664_LIBX32 => "x86-64 x32",
        FLAG_RISCV_FLOAT_ABI_SOFT => "RISC-V soft-float",
        _ => {
            if (flags & FLAG_ELF_LIBC6) == FLAG_ELF_LIBC6 {
                "ELF"
            } else {
                "unknown"
            }
        }
    }
}

fn add_string(table: &mut Vec<u8>, offsets: &mut HashMap<String, u32>, string: &str) {
    if !offsets.contains_key(string) {
        let offset = table.len() as u32;
        offsets.insert(string.to_string(), offset);
        table.extend_from_slice(string.as_bytes());
        table.push(0); // NUL terminator
    }
}
