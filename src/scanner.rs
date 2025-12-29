use crate::elf::{parse_elf_file, ElfLibrary};
use crate::error::Error;
use crate::hwcap::detect_hwcap_dirs;
use camino::Utf8PathBuf;
use std::collections::HashMap;
use std::path::Path;

/// Check if a filename looks like a DSO (Dynamic Shared Object)
/// Matches glibc's _dl_is_dso() logic from elf/dl-is_dso.h
pub fn is_dso(name: &str) -> bool {
    // Pattern 1: lib*.so* or ld-*.so*
    let has_lib_or_ld_prefix = name.starts_with("lib") || name.starts_with("ld-");
    let has_so = name.contains(".so");

    // Pattern 2: ld.so.*
    let is_ld_so = name.starts_with("ld.so.");

    // Pattern 3: ld64.so.*
    let is_ld64_so = name.starts_with("ld64.so.");

    (has_lib_or_ld_prefix && has_so) || is_ld_so || is_ld64_so
}

/// Check if a path should be scanned as a library
pub fn should_scan_library(path: &Path) -> bool {
    if let Some(filename) = path.file_name() {
        if let Some(name) = filename.to_str() {
            return is_dso(name);
        }
    }
    false
}

/// Check if a symlink should be included in the cache
pub fn should_include_symlink(filename: &str, soname: &str, path: &Utf8PathBuf) -> bool {
    if filename.ends_with(".so") && !filename.contains(".so.") {
        // Bare .so symlink: include if target has same base name + .so.VERSION pattern
        if let Ok(target) = std::fs::read_link(path.as_std_path()) {
            let target_name = target.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let base = filename.trim_end_matches(".so");
            // Include if target is like libfoo.so.X (standard pattern)
            // Exclude if target is like libfoo-X.so (dash-version) or libbar.so (different base)
            target_name.starts_with(&format!("{}.", base)) && target_name.contains(".so.")
        } else {
            false
        }
    } else {
        // Versioned symlink (.so.X): include only if filename matches SONAME
        filename == soname
    }
}

/// Scan all libraries in the given directories, separating real files from symlinks
/// Returns (real_files, symlinks)
pub fn scan_all_libraries(
    dirs: &[Utf8PathBuf],
) -> Result<(Vec<ElfLibrary>, Vec<ElfLibrary>), Error> {
    let mut real_files = Vec::new();
    let mut symlinks = Vec::new();

    for dir in dirs {
        if !dir.exists() {
            continue;
        }

        // Scan base directory
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_file() && should_scan_library(&path) {
                if let Some(lib) = parse_elf_file(&path) {
                    let is_symlink = std::fs::symlink_metadata(&path)
                        .map(|m| m.file_type().is_symlink())
                        .unwrap_or(false);

                    if is_symlink {
                        symlinks.push(lib);
                    } else {
                        real_files.push(lib);
                    }
                }
            }
        }

        // Scan hwcap subdirectories
        let hwcap_dirs = detect_hwcap_dirs(dir.as_std_path())?;
        for (hwcap_path, hwcap) in hwcap_dirs {
            for entry in std::fs::read_dir(&hwcap_path)? {
                let entry = entry?;
                let path = entry.path();

                if path.is_file() && should_scan_library(&path) {
                    if let Some(mut lib) = parse_elf_file(&path) {
                        let is_symlink = std::fs::symlink_metadata(&path)
                            .map(|m| m.file_type().is_symlink())
                            .unwrap_or(false);

                        // Set hwcap value for this library
                        let arch = lib.arch;
                        lib.hwcap = Some(hwcap.to_bitmask(arch));

                        if is_symlink {
                            symlinks.push(lib);
                        } else {
                            real_files.push(lib);
                        }
                    }
                }
            }
        }
    }

    Ok((real_files, symlinks))
}

/// Deduplicate libraries by (directory, filename) pair
pub fn deduplicate_libraries(libraries: &[ElfLibrary]) -> Vec<ElfLibrary> {
    let mut unique_libs: HashMap<(Utf8PathBuf, String), ElfLibrary> = HashMap::new();

    for lib in libraries {
        let dir = lib.path.parent().unwrap_or_else(|| "".as_ref()).to_owned();
        let filename = lib
            .path
            .file_name()
            .unwrap_or(lib.path.as_str())
            .to_string();
        let key = (dir, filename);

        // Keep first occurrence
        unique_libs.entry(key).or_insert_with(|| lib.clone());
    }

    unique_libs.into_values().collect()
}

/// Deduplicate scan directories by removing directories that are symlinks
/// to canonical paths already in the list. Keep the CANONICAL path, not the symlink.
pub fn deduplicate_scan_directories(dirs: &[Utf8PathBuf]) -> Vec<Utf8PathBuf> {
    let mut canonical_to_first: HashMap<Utf8PathBuf, Utf8PathBuf> = HashMap::new();

    for dir in dirs {
        // Get canonical path
        let canonical = if let Ok(canon) = dir.as_std_path().canonicalize() {
            Utf8PathBuf::try_from(canon).unwrap_or_else(|_| dir.clone())
        } else {
            dir.clone()
        };

        // Keep the canonical path (not the symlink)
        canonical_to_first
            .entry(canonical.clone())
            .or_insert(canonical);
    }

    canonical_to_first.into_values().collect()
}
