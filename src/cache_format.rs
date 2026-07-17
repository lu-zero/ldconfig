//! ld.so.cache binary format (new format only, magic glibc-ld.so.cache1.1).
//!
//! Layout and constants follow glibc's elf/cache.c and
//! sysdeps/generic/dl-cache.h; flag values sysdeps/generic/ldconfig.h.

use crate::error::Error;
use std::cmp::Ordering;
use std::collections::HashMap;
use tracing::debug;

pub(crate) const CACHE_MAGIC: [u8; 20] = *b"glibc-ld.so.cache1.1";

pub(crate) const FLAG_TYPE_MASK: u32 = 0x00ff;
pub(crate) const FLAG_REQUIRED_MASK: u32 = 0xff00;
pub(crate) const FLAG_ELF_LIBC6: u32 = 0x0003;
pub(crate) const FLAG_SPARC_LIB64: u32 = 0x0100;
pub(crate) const FLAG_X8664_LIB64: u32 = 0x0300;
pub(crate) const FLAG_S390_LIB64: u32 = 0x0400;
pub(crate) const FLAG_POWERPC_LIB64: u32 = 0x0500;
pub(crate) const FLAG_MIPS64_LIBN32: u32 = 0x0600;
pub(crate) const FLAG_MIPS64_LIBN64: u32 = 0x0700;
pub(crate) const FLAG_X8664_LIBX32: u32 = 0x0800;
pub(crate) const FLAG_ARM_LIBHF: u32 = 0x0900;
pub(crate) const FLAG_AARCH64_LIB64: u32 = 0x0a00;
pub(crate) const FLAG_ARM_LIBSF: u32 = 0x0b00;
pub(crate) const FLAG_MIPS_LIB32_NAN2008: u32 = 0x0c00;
pub(crate) const FLAG_MIPS64_LIBN32_NAN2008: u32 = 0x0d00;
pub(crate) const FLAG_MIPS64_LIBN64_NAN2008: u32 = 0x0e00;
pub(crate) const FLAG_RISCV_FLOAT_ABI_SOFT: u32 = 0x0f00;
pub(crate) const FLAG_RISCV_FLOAT_ABI_DOUBLE: u32 = 0x1000;
pub(crate) const FLAG_LARCH_FLOAT_ABI_SOFT: u32 = 0x1100;
pub(crate) const FLAG_LARCH_FLOAT_ABI_DOUBLE: u32 = 0x1200;

const EXTENSION_MAGIC: u32 = 0xEAA4_2174;
const TAG_GENERATOR: u32 = 0;
const TAG_GLIBC_HWCAPS: u32 = 1;

/// Marks the hwcap field as a glibc-hwcaps string index (dl-cache.h).
const DL_CACHE_HWCAP_EXTENSION: u64 = 1 << 62;
const DL_CACHE_HWCAP_ISA_LEVEL_MASK: u64 = (1 << 10) - 1;

const HEADER_SIZE: usize = 48;
const ENTRY_SIZE: usize = 24;

const ENDIAN_CURRENT: u8 = if cfg!(target_endian = "little") { 2 } else { 3 };

/// One library destined for the cache.
#[derive(Debug, Clone)]
pub(crate) struct FileEntry {
    /// Cache key: the soname.
    pub soname: String,
    /// Cache value: the full path text.
    pub path: String,
    pub flags: u32,
    /// x86 ISA level; only stored for glibc-hwcaps entries.
    pub isa_level: u32,
    /// glibc-hwcaps subdirectory name, if any.
    pub hwcaps: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct CacheEntry {
    pub flags: u32,
    pub key_offset: u32,
    pub value_offset: u32,
    pub hwcap: u64,
    /// Resolved glibc-hwcaps subdirectory name for extension entries.
    pub hwcaps: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct CacheInfo {
    pub entries: Vec<CacheEntry>,
    pub generator: Option<String>,
}

/// Entry order written by glibc (elf/cache.c compare()): reversed
/// _dl_cache_libcmp on the soname, then flags descending, then
/// glibc-hwcaps entries before plain ones, ordered by subdirectory name.
fn compare(a: &FileEntry, b: &FileEntry) -> Ordering {
    dl_cache_libcmp(&b.soname, &a.soname)
        .then_with(|| b.flags.cmp(&a.flags))
        .then_with(|| match (&a.hwcaps, &b.hwcaps) {
            (Some(x), Some(y)) => x.cmp(y),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        })
}

/// Serialize entries into cache bytes.
pub(crate) fn build_cache(entries: &[FileEntry]) -> Vec<u8> {
    let mut sorted: Vec<&FileEntry> = entries.iter().collect();
    sorted.sort_by(|a, b| compare(a, b));

    // glibc-hwcaps subdirectory names, indexed in name order like
    // assign_glibc_hwcaps_indices.
    let mut hwcaps_names: Vec<&str> = Vec::new();
    for e in entries {
        if let Some(n) = &e.hwcaps {
            if !hwcaps_names.contains(&n.as_str()) {
                hwcaps_names.push(n);
            }
        }
    }
    hwcaps_names.sort_unstable();

    let string_table_offset = HEADER_SIZE + sorted.len() * ENTRY_SIZE;
    let mut table: Vec<u8> = Vec::new();
    let mut offsets: HashMap<String, u32> = HashMap::new();
    // Strings are interned like glibc's stringtable (without its suffix
    // merging); stored offsets are absolute file offsets.
    let mut add_string = |table: &mut Vec<u8>, s: &str| -> u32 {
        if let Some(&off) = offsets.get(s) {
            return off;
        }
        let off = (string_table_offset + table.len()) as u32;
        offsets.insert(s.to_owned(), off);
        table.extend_from_slice(s.as_bytes());
        table.push(0);
        off
    };

    let mut cache = Vec::new();
    cache.extend_from_slice(&CACHE_MAGIC);
    cache.extend_from_slice(&(sorted.len() as u32).to_ne_bytes());
    let len_strings_pos = cache.len();
    cache.extend_from_slice(&0u32.to_ne_bytes());
    cache.push(ENDIAN_CURRENT);
    cache.extend_from_slice(&[0u8; 3]);
    let extension_offset_pos = cache.len();
    cache.extend_from_slice(&0u32.to_ne_bytes());
    cache.extend_from_slice(&[0u8; 12]); // unused[3]

    for e in &sorted {
        let key = add_string(&mut table, &e.soname);
        let value = add_string(&mut table, &e.path);
        let hwcap = match &e.hwcaps {
            Some(n) => {
                let index = hwcaps_names.iter().position(|x| x == n).unwrap() as u64;
                DL_CACHE_HWCAP_EXTENSION | (u64::from(e.isa_level) << 32) | index
            }
            None => 0,
        };
        cache.extend_from_slice(&e.flags.to_ne_bytes());
        cache.extend_from_slice(&key.to_ne_bytes());
        cache.extend_from_slice(&value.to_ne_bytes());
        cache.extend_from_slice(&0u32.to_ne_bytes()); // osversion_unused
        cache.extend_from_slice(&hwcap.to_ne_bytes());
    }

    let hwcaps_offsets: Vec<u32> = hwcaps_names
        .iter()
        .map(|n| add_string(&mut table, n))
        .collect();

    cache[len_strings_pos..len_strings_pos + 4]
        .copy_from_slice(&(table.len() as u32).to_ne_bytes());
    cache.extend_from_slice(&table);

    while cache.len() % 4 != 0 {
        cache.push(0);
    }

    // Extension directory, then the hwcaps index array, then the
    // generator string (write_extensions in elf/cache.c).
    let extension_offset = cache.len() as u32;
    cache[extension_offset_pos..extension_offset_pos + 4]
        .copy_from_slice(&extension_offset.to_ne_bytes());

    let generator = format!("ldconfig-rs {}", env!("CARGO_PKG_VERSION"));
    let section_count: u32 = if hwcaps_offsets.is_empty() { 1 } else { 2 };
    let data_start = extension_offset + 8 + 16 * section_count;
    let hwcaps_size = (hwcaps_offsets.len() * 4) as u32;

    cache.extend_from_slice(&EXTENSION_MAGIC.to_ne_bytes());
    cache.extend_from_slice(&section_count.to_ne_bytes());

    cache.extend_from_slice(&TAG_GENERATOR.to_ne_bytes());
    cache.extend_from_slice(&0u32.to_ne_bytes()); // flags
    cache.extend_from_slice(&(data_start + hwcaps_size).to_ne_bytes());
    cache.extend_from_slice(&(generator.len() as u32).to_ne_bytes());

    if !hwcaps_offsets.is_empty() {
        cache.extend_from_slice(&TAG_GLIBC_HWCAPS.to_ne_bytes());
        cache.extend_from_slice(&0u32.to_ne_bytes()); // flags
        cache.extend_from_slice(&data_start.to_ne_bytes());
        cache.extend_from_slice(&hwcaps_size.to_ne_bytes());
        for off in &hwcaps_offsets {
            cache.extend_from_slice(&off.to_ne_bytes());
        }
    }
    cache.extend_from_slice(generator.as_bytes());

    cache
}

fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    data.get(offset..offset + 4)
        .map(|b| u32::from_ne_bytes(b.try_into().unwrap()))
}

fn read_u64(data: &[u8], offset: usize) -> Option<u64> {
    data.get(offset..offset + 8)
        .map(|b| u64::from_ne_bytes(b.try_into().unwrap()))
}

fn read_string(data: &[u8], offset: usize) -> Option<String> {
    let bytes = data.get(offset..)?;
    let nul = bytes.iter().position(|&b| b == 0)?;
    Some(String::from_utf8_lossy(&bytes[..nul]).into_owned())
}

/// Parse cache bytes. Rejects anything that is not a well-formed
/// native-endian new-format cache.
pub(crate) fn parse_cache(data: &[u8]) -> Result<CacheInfo, Error> {
    if data.len() < HEADER_SIZE {
        return Err(Error::InvalidCache("file too small"));
    }
    if data[..20] != CACHE_MAGIC {
        return Err(Error::InvalidCache(
            "wrong magic (only the new format is supported)",
        ));
    }
    let nlibs = read_u32(data, 20).unwrap() as usize;
    let len_strings = read_u32(data, 24).unwrap() as usize;
    // 0 = unset (written by older ldconfig); only the low two bits carry
    // the byte order, the rest is ignored by readers (dl-cache.h).
    if data[28] != 0 && (data[28] & 3) != ENDIAN_CURRENT {
        return Err(Error::InvalidCache("wrong endianness"));
    }

    let entries_end = nlibs
        .checked_mul(ENTRY_SIZE)
        .and_then(|n| n.checked_add(HEADER_SIZE))
        .filter(|&end| end <= data.len())
        .ok_or(Error::InvalidCache("truncated entries"))?;
    if entries_end
        .checked_add(len_strings)
        .filter(|&e| e <= data.len())
        .is_none()
    {
        return Err(Error::InvalidCache("truncated string table"));
    }

    // Extensions are optional; a malformed section is ignored, like ld.so.
    let mut generator = None;
    let mut hwcaps_array: Vec<u32> = Vec::new();
    let ext_offset = read_u32(data, 32).unwrap() as usize;
    if ext_offset != 0 && ext_offset.is_multiple_of(4) {
        if let Some(magic) = read_u32(data, ext_offset) {
            if magic == EXTENSION_MAGIC {
                let count = read_u32(data, ext_offset + 4).unwrap_or(0) as usize;
                for i in 0..count {
                    let sec = ext_offset + 8 + i * 16;
                    let (Some(tag), Some(off), Some(size)) = (
                        read_u32(data, sec),
                        read_u32(data, sec + 8),
                        read_u32(data, sec + 12),
                    ) else {
                        break;
                    };
                    let (off, size) = (off as usize, size as usize);
                    if off.checked_add(size).filter(|&e| e <= data.len()).is_none() {
                        continue;
                    }
                    match tag {
                        TAG_GENERATOR => {
                            generator =
                                Some(String::from_utf8_lossy(&data[off..off + size]).into_owned());
                        }
                        TAG_GLIBC_HWCAPS => {
                            hwcaps_array = data[off..off + size]
                                .chunks_exact(4)
                                .map(|b| u32::from_ne_bytes(b.try_into().unwrap()))
                                .collect();
                        }
                        _ => debug!("ignoring unknown cache extension tag {}", tag),
                    }
                }
            }
        }
    }

    let strtab = entries_end..entries_end + len_strings;
    let mut entries = Vec::with_capacity(nlibs);
    for i in 0..nlibs {
        let off = HEADER_SIZE + i * ENTRY_SIZE;
        let flags = read_u32(data, off).unwrap();
        let key_offset = read_u32(data, off + 4).unwrap();
        let value_offset = read_u32(data, off + 8).unwrap();
        let hwcap = read_u64(data, off + 16).unwrap();

        if !strtab.contains(&(key_offset as usize)) || !strtab.contains(&(value_offset as usize)) {
            return Err(Error::InvalidCache("entry string offset out of range"));
        }

        let hwcaps =
            if (hwcap >> 32) & !DL_CACHE_HWCAP_ISA_LEVEL_MASK == DL_CACHE_HWCAP_EXTENSION >> 32 {
                hwcaps_array
                    .get(hwcap as u32 as usize)
                    .and_then(|&str_off| read_string(data, str_off as usize))
            } else {
                None
            };

        entries.push(CacheEntry {
            flags,
            key_offset,
            value_offset,
            hwcap,
            hwcaps,
        });
    }

    Ok(CacheInfo { entries, generator })
}

/// Flag rendering matching glibc's print_entry.
pub(crate) fn flags_string(flags: u32) -> String {
    let mut s = String::new();
    match flags & FLAG_TYPE_MASK {
        FLAG_ELF_LIBC6 => s.push_str("libc6"),
        _ => s.push_str("unknown or unsupported flag"),
    }
    match flags & FLAG_REQUIRED_MASK {
        0 => {}
        FLAG_SPARC_LIB64 | FLAG_S390_LIB64 | FLAG_POWERPC_LIB64 | FLAG_MIPS64_LIBN64 => {
            s.push_str(",64bit")
        }
        FLAG_X8664_LIB64 => s.push_str(",x86-64"),
        FLAG_MIPS64_LIBN32 => s.push_str(",N32"),
        FLAG_X8664_LIBX32 => s.push_str(",x32"),
        FLAG_ARM_LIBHF => s.push_str(",hard-float"),
        FLAG_AARCH64_LIB64 => s.push_str(",AArch64"),
        FLAG_ARM_LIBSF | FLAG_RISCV_FLOAT_ABI_SOFT | FLAG_LARCH_FLOAT_ABI_SOFT => {
            s.push_str(",soft-float")
        }
        FLAG_MIPS_LIB32_NAN2008 => s.push_str(",nan2008"),
        FLAG_MIPS64_LIBN32_NAN2008 => s.push_str(",N32,nan2008"),
        FLAG_MIPS64_LIBN64_NAN2008 => s.push_str(",64bit,nan2008"),
        FLAG_RISCV_FLOAT_ABI_DOUBLE | FLAG_LARCH_FLOAT_ABI_DOUBLE => s.push_str(",double-float"),
        other => {
            s.push(',');
            s.push_str(&other.to_string());
        }
    }
    s
}

/// Numeric-aware string comparison matching glibc's `_dl_cache_libcmp`.
/// Digits sort after non-digits; runs of digits compare numerically.
pub(crate) fn dl_cache_libcmp(p1: &str, p2: &str) -> Ordering {
    let b1 = p1.as_bytes();
    let b2 = p2.as_bytes();
    let mut i = 0;
    let mut j = 0;

    while i < b1.len() {
        if b1[i].is_ascii_digit() {
            if j < b2.len() && b2[j].is_ascii_digit() {
                // Both digits: compare numerically.
                let mut val1: i64 = 0;
                let mut val2: i64 = 0;
                while i < b1.len() && b1[i].is_ascii_digit() {
                    val1 = val1
                        .saturating_mul(10)
                        .saturating_add((b1[i] - b'0') as i64);
                    i += 1;
                }
                while j < b2.len() && b2[j].is_ascii_digit() {
                    val2 = val2
                        .saturating_mul(10)
                        .saturating_add((b2[j] - b'0') as i64);
                    j += 1;
                }
                if val1 != val2 {
                    return val1.cmp(&val2);
                }
            } else {
                // p1 digit, p2 non-digit: digits sort after non-digits.
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
    // p1 ended: compare NUL (0) vs p2's current char.
    0u8.cmp(b2.get(j).unwrap_or(&0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(soname: &str, path: &str, flags: u32, hwcaps: Option<&str>) -> FileEntry {
        FileEntry {
            soname: soname.into(),
            path: path.into(),
            flags,
            isa_level: 0,
            hwcaps: hwcaps.map(str::to_owned),
        }
    }

    #[test]
    fn libcmp_identical() {
        assert_eq!(
            dl_cache_libcmp("libfoo.so.1", "libfoo.so.1"),
            Ordering::Equal
        );
    }

    #[test]
    fn libcmp_numeric_order() {
        assert_eq!(
            dl_cache_libcmp("libfoo.so.2", "libfoo.so.10"),
            Ordering::Less
        );
        assert_eq!(
            dl_cache_libcmp("libfoo.so.10", "libfoo.so.2"),
            Ordering::Greater
        );
    }

    #[test]
    fn libcmp_digit_after_nondigit() {
        assert_eq!(
            dl_cache_libcmp("libfoo.so.1a", "libfoo.so.1."),
            Ordering::Greater
        );
    }

    #[test]
    fn libcmp_prefix() {
        assert_eq!(dl_cache_libcmp("libfoo.so", "libfoo.so.1"), Ordering::Less);
    }

    #[test]
    fn libcmp_different_names() {
        assert_eq!(dl_cache_libcmp("liba.so", "libb.so"), Ordering::Less);
        assert_eq!(dl_cache_libcmp("libb.so", "liba.so"), Ordering::Greater);
    }

    #[test]
    fn libcmp_multi_version() {
        assert_eq!(
            dl_cache_libcmp("libfoo.so.1.2.3", "libfoo.so.1.2.4"),
            Ordering::Less
        );
        assert_eq!(
            dl_cache_libcmp("libfoo.so.1.10", "libfoo.so.1.9"),
            Ordering::Greater
        );
    }

    #[test]
    fn libcmp_huge_numbers_no_panic() {
        assert_eq!(
            dl_cache_libcmp("libfoo.so.99999999999999999999", "libfoo.so.1"),
            Ordering::Greater
        );
    }

    #[test]
    fn sort_reversed_by_soname_then_flags() {
        let mut entries = [
            entry("libz.so.1", "/usr/lib32/libz.so.1", 0x0003, None),
            entry("liba.so.1", "/usr/lib/liba.so.1", 0x0303, None),
            entry("libz.so.1", "/usr/lib/libz.so.1", 0x0303, None),
        ];
        entries.sort_by(compare);
        let order: Vec<(&str, u32)> = entries
            .iter()
            .map(|e| (e.soname.as_str(), e.flags))
            .collect();
        // Reversed name order; for equal sonames the higher flags first.
        assert_eq!(
            order,
            [
                ("libz.so.1", 0x0303),
                ("libz.so.1", 0x0003),
                ("liba.so.1", 0x0303),
            ]
        );
    }

    #[test]
    fn sort_hwcaps_entries_first_by_name() {
        let mut entries = [
            entry("libx.so.1", "/usr/lib/libx.so.1", 0x0303, None),
            entry(
                "libx.so.1",
                "/usr/lib/glibc-hwcaps/x86-64-v3/libx.so.1",
                0x0303,
                Some("x86-64-v3"),
            ),
            entry(
                "libx.so.1",
                "/usr/lib/glibc-hwcaps/x86-64-v2/libx.so.1",
                0x0303,
                Some("x86-64-v2"),
            ),
        ];
        entries.sort_by(compare);
        let order: Vec<Option<&str>> = entries.iter().map(|e| e.hwcaps.as_deref()).collect();
        assert_eq!(order, [Some("x86-64-v2"), Some("x86-64-v3"), None]);
    }

    #[test]
    fn round_trip_build_parse() {
        let entries = vec![
            entry("libtest.so.1", "/usr/lib/libtest.so.1", 0x0303, None),
            entry("libother.so.2", "/usr/lib/libother.so.2", 0x0303, None),
        ];
        let data = build_cache(&entries);
        let info = parse_cache(&data).unwrap();

        assert_eq!(info.entries.len(), 2);
        assert!(info.generator.unwrap().starts_with("ldconfig-rs"));
        // No trailing NUL after the generator string.
        assert_ne!(*data.last().unwrap(), 0);

        for e in &info.entries {
            let key = read_string(&data, e.key_offset as usize).unwrap();
            let value = read_string(&data, e.value_offset as usize).unwrap();
            assert!(key.contains(".so"));
            assert!(value.starts_with("/usr/lib/"));
            assert_eq!(e.hwcap, 0);
        }
    }

    #[test]
    fn round_trip_hwcaps_extension() {
        let entries = vec![
            entry("liba.so.1", "/usr/lib/liba.so.1", 0x0303, None),
            entry(
                "liba.so.1",
                "/usr/lib/glibc-hwcaps/x86-64-v3/liba.so.1",
                0x0303,
                Some("x86-64-v3"),
            ),
        ];
        let data = build_cache(&entries);
        let info = parse_cache(&data).unwrap();

        assert_eq!(info.entries.len(), 2);
        let hw = &info.entries[0]; // hwcaps entry sorts first
        assert_eq!(hw.hwcaps.as_deref(), Some("x86-64-v3"));
        assert_eq!(hw.hwcap >> 62, 1);
        assert_eq!(hw.hwcap as u32, 0);
        assert_eq!(info.entries[1].hwcaps, None);
        assert_eq!(info.entries[1].hwcap, 0);
    }

    #[test]
    fn isa_level_encoded_for_hwcaps_entries() {
        let mut e = entry(
            "liba.so.1",
            "/usr/lib/glibc-hwcaps/x86-64-v3/liba.so.1",
            0x0303,
            Some("x86-64-v3"),
        );
        e.isa_level = 2;
        let data = build_cache(&[e]);
        let info = parse_cache(&data).unwrap();
        assert_eq!((info.entries[0].hwcap >> 32) & 0x3ff, 2);
    }

    #[test]
    fn parse_rejects_garbage_without_panicking() {
        assert!(parse_cache(b"").is_err());
        assert!(parse_cache(b"garbage").is_err());
        assert!(parse_cache(&[0u8; 48]).is_err());

        // Truncations of a valid cache must never panic.
        let data = build_cache(&[entry("liba.so.1", "/usr/lib/liba.so.1", 0x0303, None)]);
        for len in 0..data.len() {
            let _ = parse_cache(&data[..len]);
        }

        // nlibs lying about the entry count must error, not panic.
        let mut bad = data.clone();
        bad[20..24].copy_from_slice(&u32::MAX.to_ne_bytes());
        assert!(parse_cache(&bad).is_err());
    }

    #[test]
    fn flags_strings_match_glibc() {
        assert_eq!(flags_string(0x0303), "libc6,x86-64");
        assert_eq!(flags_string(0x0003), "libc6");
        assert_eq!(flags_string(0x0803), "libc6,x32");
        assert_eq!(flags_string(0x0903), "libc6,hard-float");
        assert_eq!(flags_string(0x0a03), "libc6,AArch64");
        assert_eq!(flags_string(0x0b03), "libc6,soft-float");
        assert_eq!(flags_string(0x0503), "libc6,64bit");
        assert_eq!(flags_string(0x1003), "libc6,double-float");
        assert_eq!(flags_string(0x0002), "unknown or unsupported flag");
    }
}
