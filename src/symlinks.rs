use crate::elf::ElfLibrary;
use crate::Error;
use camino::Utf8PathBuf;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct SymlinkAction {
    pub target: Utf8PathBuf,
    pub link: Utf8PathBuf,
    pub action: SymlinkActionType,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SymlinkActionType {
    Create,
    Update,
    Skip,
}

pub fn create_symlink(target: &Path, link: &Path) -> Result<(), Error> {
    std::os::unix::fs::symlink(target, link)
        .map_err(|e| Error::Symlink(format!("Failed to create symlink: {}", e)))?;
    Ok(())
}

pub fn update_symlinks(
    _dir: &Path,
    libraries: &[ElfLibrary],
    dry_run: bool,
) -> Result<Vec<SymlinkAction>, Error> {
    let mut actions = Vec::new();

    // Group libraries by their SONAME
    let mut soname_map: std::collections::HashMap<String, Vec<&ElfLibrary>> =
        std::collections::HashMap::new();

    for lib in libraries {
        // Only consider real files (not symlinks)
        let is_symlink = std::fs::symlink_metadata(&lib.path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false);

        if !is_symlink {
            soname_map.entry(lib.soname.clone()).or_default().push(lib);
        }
    }

    // For each SONAME, find the highest-versioned library and create symlink
    for (soname, libs) in soname_map {
        if libs.is_empty() {
            continue;
        }

        // Find the highest-versioned library (by filename numerical comparison)
        let best_lib = find_highest_version_library(&libs);

        let filename = best_lib.path.file_name().unwrap_or("");

        // Only create symlink if SONAME != filename (avoid self-referencing symlinks)
        if filename != soname {
            let symlink_path = best_lib.path.parent().unwrap().join(&soname);

            // Target is just the filename (relative symlink in same directory)
            let target_path = Path::new(filename);

            if should_create_symlink(symlink_path.as_std_path(), best_lib.path.as_std_path())? {
                actions.push(SymlinkAction {
                    target: Utf8PathBuf::from(filename),
                    link: Utf8PathBuf::try_from(symlink_path.clone()).map_err(|_| {
                        Error::Config("Invalid UTF-8 in symlink path".to_string())
                    })?,
                    action: SymlinkActionType::Create,
                });

                if !dry_run {
                    // Remove existing symlink/file if it exists
                    if symlink_path.exists() || symlink_path.symlink_metadata().is_ok() {
                        let _ = fs::remove_file(&symlink_path);
                    }
                    create_symlink(target_path, symlink_path.as_std_path())?;
                }
            }
        }
    }

    Ok(actions)
}

/// Find the library with the highest version by comparing filenames numerically
/// Uses the same algorithm as glibc's _dl_cache_libcmp
fn find_highest_version_library<'a>(libs: &'a [&'a ElfLibrary]) -> &'a ElfLibrary {
    libs.iter()
        .max_by(|a, b| {
            let filename_a = a.path.file_name().unwrap_or("");
            let filename_b = b.path.file_name().unwrap_or("");
            compare_library_versions(filename_a, filename_b)
        })
        .unwrap_or(&libs[0])
}

/// Compare library versions numerically, like glibc's _dl_cache_libcmp
/// Higher version returns Greater
fn compare_library_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let mut a_chars = a.chars().peekable();
    let mut b_chars = b.chars().peekable();

    loop {
        let a_ch = a_chars.peek().copied();
        let b_ch = b_chars.peek().copied();

        match (a_ch, b_ch) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (Some(ac), Some(bc)) => {
                // If both are digits, compare numerically
                if ac.is_ascii_digit() && bc.is_ascii_digit() {
                    let mut a_num = 0u64;
                    let mut b_num = 0u64;

                    while let Some(&ch) = a_chars.peek() {
                        if ch.is_ascii_digit() {
                            a_num = a_num * 10 + (ch as u64 - '0' as u64);
                            a_chars.next();
                        } else {
                            break;
                        }
                    }

                    while let Some(&ch) = b_chars.peek() {
                        if ch.is_ascii_digit() {
                            b_num = b_num * 10 + (ch as u64 - '0' as u64);
                            b_chars.next();
                        } else {
                            break;
                        }
                    }

                    if a_num != b_num {
                        return a_num.cmp(&b_num); // Higher number is greater
                    }
                } else {
                    // Compare characters normally
                    if ac != bc {
                        return ac.cmp(&bc);
                    }
                    a_chars.next();
                    b_chars.next();
                }
            }
        }
    }
}

fn should_create_symlink(link_path: &Path, target_path: &Path) -> Result<bool, Error> {
    if !link_path.exists() {
        return Ok(true);
    }

    // Check if the symlink points to the correct target
    let current_target = fs::read_link(link_path);
    match current_target {
        Ok(current) => {
            // Compare paths (canonicalize for comparison)
            let current_canon = current.canonicalize().unwrap_or(current);
            let target_canon = target_path
                .canonicalize()
                .unwrap_or_else(|_| target_path.to_path_buf());

            Ok(current_canon != target_canon)
        }
        Err(_) => Ok(true), // If we can't read the link, assume we need to create it
    }
}

