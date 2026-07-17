//! Directory scanning, mirroring glibc's search_dir and directory setup.

use crate::chroot::chroot_canon;
use crate::elf;
use camino::{Utf8Path, Utf8PathBuf};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::os::unix::fs::MetadataExt;
use tracing::{debug, warn};

/// A directory to scan: the configured path (used as cache entry text)
/// plus its on-disk location under the -r prefix.
#[derive(Debug, Clone)]
pub(crate) struct ScanDir {
    pub path: Utf8PathBuf,
    pub real: Utf8PathBuf,
    /// glibc-hwcaps subdirectory name, if this is one.
    pub hwcaps: Option<String>,
}

/// One library chosen for a directory: `name` is the file name on disk,
/// `soname` the cache key.
#[derive(Debug, Clone)]
pub(crate) struct DirLib {
    pub name: String,
    pub soname: String,
    pub flags: u32,
    pub isa_level: u32,
    pub is_link: bool,
}

/// Matches glibc's _dl_is_dso() from elf/dl-is_dso.h.
pub(crate) fn is_dso(name: &str) -> bool {
    ((name.starts_with("lib") || name.starts_with("ld-")) && name.contains(".so"))
        || name.starts_with("ld.so.")
        || name.starts_with("ld64.so.")
}

/// Temporary files from prelink, RPM, and dpkg; glibc's
/// skip_dso_based_on_name.
fn is_temp_dso(name: &str) -> bool {
    let b = name.as_bytes();
    // ".#prelink#" suffix or ".#prelink#.XXXXXX" (6-char random suffix).
    b.ends_with(b".#prelink#")
        || (b.len() >= 17 && &b[b.len() - 17..b.len() - 6] == b".#prelink#.")
        || name.contains(';')
        || name.ends_with(".dpkg-new")
        || name.ends_with(".dpkg-tmp")
}

fn resolve(prefix: &Utf8Path, path: &Utf8Path) -> Option<Utf8PathBuf> {
    if prefix == "/" {
        Some(path.to_path_buf())
    } else {
        chroot_canon(prefix, path)
    }
}

/// Build the scan list: strip trailing slashes, drop nonexistent
/// directories, deduplicate by (dev, ino) keeping the first configured
/// path text, and queue glibc-hwcaps subdirectories after their parent.
pub(crate) fn collect_dirs(dirs: &[Utf8PathBuf], prefix: &Utf8Path) -> Vec<ScanDir> {
    let mut seen: HashSet<(u64, u64)> = HashSet::new();
    let mut out = Vec::new();

    for dir in dirs {
        let trimmed = dir.as_str().trim_end_matches('/');
        if trimmed.is_empty() {
            continue;
        }
        let logical = Utf8PathBuf::from(trimmed);
        let Some(real) = resolve(prefix, &logical) else {
            debug!("Can't stat {}", logical);
            continue;
        };
        let Ok(md) = fs::metadata(&real) else {
            debug!("Can't stat {}", logical);
            continue;
        };
        if !md.is_dir() {
            continue;
        }
        if !seen.insert((md.dev(), md.ino())) {
            debug!("Path `{}' given more than once", logical);
            continue;
        }
        out.push(ScanDir {
            path: logical.clone(),
            real: real.clone(),
            hwcaps: None,
        });

        // glibc-hwcaps subdirectories (add_glibc_hwcaps_subdirectories):
        // every directory under <dir>/glibc-hwcaps, no name whitelist.
        let hw = real.join("glibc-hwcaps");
        let Ok(rd) = fs::read_dir(&hw) else { continue };
        let mut subs: Vec<String> = Vec::new();
        for entry in rd.flatten() {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            // Names with ':' cannot be looked up by the dynamic loader.
            if name.starts_with('.') || name.contains(':') {
                continue;
            }
            let Ok(md) = fs::metadata(entry.path()) else {
                continue;
            };
            if !md.is_dir() || !seen.insert((md.dev(), md.ino())) {
                continue;
            }
            subs.push(name);
        }
        subs.sort();
        for name in subs {
            out.push(ScanDir {
                path: logical.join("glibc-hwcaps").join(&name),
                real: hw.join(&name),
                hwcaps: Some(name),
            });
        }
    }
    out
}

/// glibc's per-soname resolution inside one directory: prefer a real file
/// over a symlink, otherwise the higher name per _dl_cache_libcmp. The
/// first entry's flags are kept (glibc quirk), with a warning on mismatch.
fn merge_candidate(dlibs: &mut HashMap<String, DirLib>, cand: DirLib, dir: &Utf8Path) {
    use crate::cache_format::dl_cache_libcmp;

    match dlibs.get_mut(&cand.soname) {
        None => {
            dlibs.insert(cand.soname.clone(), cand);
        }
        Some(existing) => {
            if (!cand.is_link && existing.is_link)
                || (cand.is_link == existing.is_link
                    && dl_cache_libcmp(&existing.name, &cand.name) == Ordering::Less)
            {
                if existing.flags != cand.flags {
                    warn!(
                        "libraries {} and {} in directory {} have same soname but different type.",
                        existing.name, cand.name, dir
                    );
                }
                existing.name = cand.name;
                existing.is_link = cand.is_link;
                existing.isa_level = cand.isa_level;
            }
        }
    }
}

/// Scan one directory, returning the winning library per soname.
/// `remove_stale_links` removes dangling *.so.* symlinks like glibc does
/// when link updating is enabled.
pub(crate) fn scan_dir(sd: &ScanDir, prefix: &Utf8Path, remove_stale_links: bool) -> Vec<DirLib> {
    let Ok(rd) = fs::read_dir(&sd.real) else {
        debug!("Can't open directory {}", sd.path);
        return Vec::new();
    };

    let mut dlibs: HashMap<String, DirLib> = HashMap::new();
    for entry in rd.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Ok(ft) = entry.file_type() else { continue };
        let is_link = ft.is_symlink();

        // In glibc-hwcaps directories the DSO name filter only applies to
        // regular files (search_dir).
        if !is_dso(&name) && (!is_link || sd.hwcaps.is_none()) {
            continue;
        }
        if is_temp_dso(&name) {
            continue;
        }

        let full = sd.real.join(&name);
        let mut inspect_path = full.clone();
        if is_link {
            // glibc resolves the link inside the root (chroot_canon) and
            // inspects the resolved file; failed resolution skips the
            // entry untouched.
            let Some(target) = resolve(prefix, &sd.path.join(&name)) else {
                continue;
            };
            match fs::metadata(&target) {
                Ok(md) if md.is_file() => {}
                Ok(_) => continue,
                Err(_) => {
                    // Remove stale symlinks.
                    if remove_stale_links && name.contains(".so.") {
                        let _ = fs::remove_file(&full);
                    }
                    continue;
                }
            }
            inspect_path = target;
        } else if !ft.is_file() {
            continue;
        }

        let Some(info) = elf::inspect(inspect_path.as_std_path()) else {
            continue;
        };
        let mut soname = info.soname.unwrap_or_else(|| name.clone());
        let mut is_link = is_link;
        if is_link && name != soname {
            // Only the dev-symlink form (libfoo.so being a prefix of the
            // soname) keeps link status; anything else is treated as a
            // normal file (search_dir comment on link hygiene).
            if !(name.ends_with(".so") && soname.starts_with(name.as_str())) {
                is_link = false;
            }
        }
        if is_link {
            soname = name.clone();
        }

        merge_candidate(
            &mut dlibs,
            DirLib {
                name,
                soname,
                flags: info.flags,
                isa_level: info.isa_level,
                is_link,
            },
            &sd.path,
        );
    }

    let mut libs: Vec<DirLib> = dlibs.into_values().collect();
    libs.sort_by(|a, b| a.soname.cmp(&b.soname));
    libs
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn temp_files_skipped() {
        assert!(is_temp_dso("libfoo.so.1.#prelink#"));
        assert!(is_temp_dso("libfoo.so.1.#prelink#.ab12cd"));
        assert!(is_temp_dso("libfoo.so.1;5f3a"));
        assert!(is_temp_dso("libfoo.so.1.dpkg-new"));
        assert!(!is_temp_dso("libfoo.so.1"));
        // The prelink check is suffix-anchored, like glibc's.
        assert!(!is_temp_dso("libp.#prelink#.so.1"));
    }

    fn lib(name: &str, soname: &str, is_link: bool) -> DirLib {
        DirLib {
            name: name.into(),
            soname: soname.into(),
            flags: 0x0303,
            isa_level: 0,
            is_link,
        }
    }

    #[test]
    fn file_beats_link_for_same_soname() {
        let dir = Utf8Path::new("/usr/lib");
        let mut m = HashMap::new();
        merge_candidate(&mut m, lib("libfoo.so.1", "libfoo.so.1", true), dir);
        merge_candidate(&mut m, lib("libfoo.so.1.2.3", "libfoo.so.1", false), dir);
        let winner = &m["libfoo.so.1"];
        assert_eq!(winner.name, "libfoo.so.1.2.3");
        assert!(!winner.is_link);

        // A link never displaces a file.
        merge_candidate(&mut m, lib("libfoo.so.1.9", "libfoo.so.1", true), dir);
        assert_eq!(m["libfoo.so.1"].name, "libfoo.so.1.2.3");
    }

    #[test]
    fn higher_version_wins_between_files() {
        let dir = Utf8Path::new("/usr/lib");
        let mut m = HashMap::new();
        merge_candidate(&mut m, lib("libfoo.so.1.2", "libfoo.so.1", false), dir);
        merge_candidate(&mut m, lib("libfoo.so.1.10", "libfoo.so.1", false), dir);
        assert_eq!(m["libfoo.so.1"].name, "libfoo.so.1.10");
        merge_candidate(&mut m, lib("libfoo.so.1.9", "libfoo.so.1", false), dir);
        assert_eq!(m["libfoo.so.1"].name, "libfoo.so.1.10");
    }
}
