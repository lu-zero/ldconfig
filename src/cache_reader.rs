use crate::error::LdconfigError;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct CacheHeader {
    pub magic: String,
    pub nlibs: u32,
    pub len_strings: u32,
}

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
    pub header: CacheHeader,
    pub entries: Vec<CacheEntry>,
    pub string_table: Vec<String>,
}

pub fn read_cache_file(path: &Path) -> Result<CacheInfo, LdconfigError> {
    let data = fs::read(path)
        .map_err(|e| LdconfigError::CacheWrite(format!("Failed to read cache: {}", e)))?;

    parse_cache_data(&data)
}

pub fn parse_cache_data(data: &[u8]) -> Result<CacheInfo, LdconfigError> {
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
    let header_size = 44; // 20 (magic) + 4 (nlibs) + 4 (len_strings) + 20 (unused)
    let entry_size = 24; // 4 + 4 + 4 + 4 + 8 (no padding) - matches ld-so-cache format

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

    Ok(CacheInfo {
        header,
        entries,
        string_table: strings,
    })
}
