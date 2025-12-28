use crate::elf::{ElfArch, ElfLibrary};
use camino::{Utf8Path, Utf8PathBuf};
use std::collections::HashMap;

const CACHE_MAGIC: [u8; 20] = *b"glibc-ld.so.cache1.1";

// Flag constants from glibc ldconfig.h
#[allow(dead_code)]
const FLAG_TYPE_MASK: u32 = 0x00ff;
const FLAG_ELF_LIBC6: u32 = 0x0003;
const FLAG_X8664_LIB64: u32 = 0x0300;
const FLAG_AARCH64_LIB64: u32 = 0x0a00;
const FLAG_RISCV64_LIB64: u32 = 0x0500;
const FLAG_IA64_LIB64: u32 = 0x0600;
#[allow(dead_code)]
const FLAG_X8664_LIBX32: u32 = 0x0800;
const FLAG_ARM_LIBHF: u32 = 0x0900;
const FLAG_POWERPC_LIB64: u32 = 0x0400;

#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub flags: u32,
    pub key_offset: u32,
    pub value_offset: u32,
    pub osversion: u32,
    pub hwcap: u64,
}

pub fn build_cache(libraries: &[ElfLibrary], prefix: Option<&Utf8Path>) -> Vec<u8> {
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
    // Note: header_size = 48, entry_size = 24 for reference

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
        let flags = match lib.arch {
            ElfArch::X86_64 => {
                if lib.is_64bit {
                    FLAG_X8664_LIB64 | FLAG_ELF_LIBC6
                } else {
                    FLAG_ELF_LIBC6
                }
            }
            ElfArch::AArch64 => FLAG_AARCH64_LIB64 | FLAG_ELF_LIBC6,
            ElfArch::RiscV64 => FLAG_RISCV64_LIB64 | FLAG_ELF_LIBC6,
            ElfArch::PowerPC64 => FLAG_POWERPC_LIB64 | FLAG_ELF_LIBC6,
            ElfArch::IA64 => FLAG_IA64_LIB64 | FLAG_ELF_LIBC6,
            ElfArch::I686 => FLAG_ELF_LIBC6,
            ElfArch::ARM => {
                if lib.is_hardfloat {
                    FLAG_ARM_LIBHF | FLAG_ELF_LIBC6
                } else {
                    FLAG_ELF_LIBC6
                }
            }
        };

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
    // glibc requires extension_offset to be 4-byte aligned
    while cache.len() % 4 != 0 {
        cache.push(0);
    }

    // Add extension section with generator information
    let extension_offset = cache.len() as u32;

    // Extension magic: 0xEAA42174 = (uint32_t)-358342284
    const EXTENSION_MAGIC: u32 = 0xEAA42174;
    cache.extend_from_slice(&EXTENSION_MAGIC.to_le_bytes());

    // Extension directory header: count of extensions
    cache.extend_from_slice(&1u32.to_le_bytes()); // 1 extension

    // Calculate where the generator data will be
    // Extension directory = 4 (magic) + 4 (count) + 16 (section descriptor)
    let generator_data_offset = extension_offset + 4 + 4 + 16;

    // Generator string (include version from Cargo.toml)
    let generator = format!("ldconfig-rs {}", env!("CARGO_PKG_VERSION"));
    let generator_bytes = generator.as_bytes();

    // Extension section descriptor
    // Note: glibc uses tag=0 for generator (cache_extension_tag_generator enum starts at 0)
    cache.extend_from_slice(&0u32.to_le_bytes()); // tag: 0 (cache_extension_tag_generator)
    cache.extend_from_slice(&0u32.to_le_bytes()); // flags: 0
    cache.extend_from_slice(&generator_data_offset.to_le_bytes()); // offset to data
    cache.extend_from_slice(&(generator_bytes.len() as u32).to_le_bytes()); // size of data

    // Append the actual generator string (null-terminated)
    cache.extend_from_slice(generator_bytes);
    cache.push(0); // null terminator

    // Update placeholders in header
    let nlibs = sorted_libs.len() as u32;
    let len_strings = string_table.len() as u32;

    cache[nlibs_pos..nlibs_pos + 4].copy_from_slice(&nlibs.to_le_bytes());
    cache[len_strings_pos..len_strings_pos + 4].copy_from_slice(&len_strings.to_le_bytes());

    // Update extension_offset in header (at offset 32)
    cache[32..36].copy_from_slice(&extension_offset.to_le_bytes());

    cache
}

fn add_string(table: &mut Vec<u8>, offsets: &mut HashMap<String, u32>, string: &str) {
    if !offsets.contains_key(string) {
        let offset = table.len() as u32;
        offsets.insert(string.to_string(), offset);
        table.extend_from_slice(string.as_bytes());
        table.push(0); // NUL terminator
    }
}
