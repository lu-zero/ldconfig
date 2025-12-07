use crate::elf::{ElfArch, ElfLibrary};
use camino::Utf8Path;
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

    // Header: unused (16 bytes) - padding to make header exactly 44 bytes
    cache.extend_from_slice(&[0u8; 16]);

    // Build string table
    let mut string_table = Vec::new();
    // Start with null bytes like the real ld.so.cache
    string_table.extend_from_slice(&[0u8; 4]);

    let mut string_offsets = HashMap::new();

    // Add SONAMEs and paths to string table
    for lib in libraries {
        add_string(&mut string_table, &mut string_offsets, &lib.soname);

        // Convert the path to an absolute path for the cache
        // The real ldconfig uses absolute paths in the cache
        let path_to_add = if let Some(prefix) = prefix {
            if let Ok(stripped) = lib.path.strip_prefix(prefix) {
                // Convert to absolute path by prepending '/'
                format!("/{}", stripped)
            } else {
                lib.path.to_string()
            }
        } else {
            lib.path.to_string()
        };

        add_string(&mut string_table, &mut string_offsets, &path_to_add);
    }

    // Sort libraries for consistent cache and optimized lookup
    // Primary: SONAME alphabetical
    // Secondary: hwcap priority (higher hwcap = more specialized, comes first)
    let mut sorted_libs = libraries.to_vec();
    sorted_libs.sort_by(|a, b| {
        match a.soname.cmp(&b.soname) {
            std::cmp::Ordering::Equal => {
                // Higher hwcap comes first (more specialized)
                b.hwcap.unwrap_or(0).cmp(&a.hwcap.unwrap_or(0))
            }
            other => other,
        }
    });

    // Build entries
    // Note: header_size = 44, entry_size = 24 for reference

    for lib in &sorted_libs {
        // Look up string offsets for SONAME and path
        let key_offset = *string_offsets.get(&lib.soname).unwrap_or_else(|| {
            eprintln!(
                "WARNING: SONAME '{}' not found in string offsets map!",
                lib.soname
            );
            &0u32
        });

        // Convert the path to an absolute path for the cache
        let path_to_add = if let Some(prefix) = prefix {
            if let Ok(stripped) = lib.path.strip_prefix(prefix) {
                format!("/{}", stripped)
            } else {
                lib.path.to_string()
            }
        } else {
            lib.path.to_string()
        };

        let value_offset = *string_offsets.get(&path_to_add).unwrap_or_else(|| {
            eprintln!(
                "WARNING: PATH '{}' not found in string offsets map!",
                path_to_add
            );
            &0u32
        });

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

    // Update placeholders
    let nlibs = libraries.len() as u32;
    let len_strings = string_table.len() as u32;

    cache[nlibs_pos..nlibs_pos + 4].copy_from_slice(&nlibs.to_le_bytes());
    cache[len_strings_pos..len_strings_pos + 4].copy_from_slice(&len_strings.to_le_bytes());

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
