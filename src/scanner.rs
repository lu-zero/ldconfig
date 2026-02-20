use crate::elf::{parse_elf_file, ElfLibrary};
use crate::error::Error;
use crate::hwcap::detect_hwcap_dirs;
use camino::Utf8PathBuf;
use std::collections::HashSet;
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

/// Deduplicate libraries by (directory, filename) pair, preserving order
pub fn deduplicate_libraries(libraries: &[ElfLibrary]) -> Vec<ElfLibrary> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();

    for lib in libraries {
        let dir = lib.path.parent().unwrap_or_else(|| "".as_ref()).to_owned();
        let filename = lib
            .path
            .file_name()
            .unwrap_or(lib.path.as_str())
            .to_string();

        if seen.insert((dir, filename)) {
            result.push(lib.clone());
        }
    }

    result
}

/// Deduplicate scan directories by removing directories that are symlinks
/// to canonical paths already in the list. Keep the CANONICAL path, not the symlink.
/// Preserves first-occurrence order.
pub fn deduplicate_scan_directories(dirs: &[Utf8PathBuf]) -> Vec<Utf8PathBuf> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();

    for dir in dirs {
        let canonical = if let Ok(canon) = dir.as_std_path().canonicalize() {
            Utf8PathBuf::try_from(canon).unwrap_or_else(|_| dir.clone())
        } else {
            dir.clone()
        };

        if seen.insert(canonical.clone()) {
            result.push(canonical);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::elf::{ElfArch, ElfLibrary};

    #[test]
    fn is_dso_standard_libs() {
        assert!(is_dso("libfoo.so"));
        assert!(is_dso("libfoo.so.1"));
        assert!(is_dso("libfoo.so.1.2.3"));
        assert!(is_dso("ld-linux-x86-64.so.2"));
        assert!(is_dso("ld.so.1"));
        assert!(is_dso("ld64.so.2"));
    }

    #[test]
    fn is_dso_rejects_non_libs() {
        assert!(!is_dso("foo.txt"));
        assert!(!is_dso("libfoo.a"));
        assert!(!is_dso("README.md"));
        assert!(!is_dso("foo.so")); // no lib/ld prefix
    }

    fn make_lib(path: &str) -> ElfLibrary {
        ElfLibrary {
            soname: "libtest.so.1".into(),
            path: Utf8PathBuf::from(path),
            is_64bit: true,
            arch: ElfArch::X86_64,
            is_hardfloat: false,
            osversion: 0,
            hwcap: None,
        }
    }

    #[test]
    fn dedup_preserves_order() {
        let libs = vec![
            make_lib("/usr/lib/libfoo.so.1"),
            make_lib("/usr/lib/libbar.so.1"),
            make_lib("/usr/lib/libfoo.so.1"), // duplicate
            make_lib("/usr/lib/libbaz.so.1"),
        ];

        let result = deduplicate_libraries(&libs);
        let names: Vec<&str> = result.iter().map(|l| l.path.file_name().unwrap()).collect();
        assert_eq!(names, ["libfoo.so.1", "libbar.so.1", "libbaz.so.1"]);
    }

    #[test]
    fn dedup_same_filename_different_dirs() {
        let libs = vec![
            make_lib("/usr/lib/libfoo.so.1"),
            make_lib("/lib/libfoo.so.1"),
        ];

        let result = deduplicate_libraries(&libs);
        assert_eq!(result.len(), 2); // different dirs, both kept
    }
}
