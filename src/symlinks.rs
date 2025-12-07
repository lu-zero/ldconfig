use crate::elf::ElfLibrary;
use crate::error::LdconfigError;
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

pub fn create_symlink(target: &Path, link: &Path) -> Result<(), LdconfigError> {
    std::os::unix::fs::symlink(target, link)
        .map_err(|e| LdconfigError::Symlink(format!("Failed to create symlink: {}", e)))?;
    Ok(())
}

pub fn update_symlinks(
    _dir: &Path,
    libraries: &[ElfLibrary],
    dry_run: bool,
) -> Result<Vec<SymlinkAction>, LdconfigError> {
    let mut actions = Vec::new();

    // Group libraries by their SONAME
    let mut soname_map: std::collections::HashMap<String, Vec<&ElfLibrary>> =
        std::collections::HashMap::new();

    for lib in libraries {
        soname_map.entry(lib.soname.clone()).or_default().push(lib);
    }

    // For each SONAME, find the newest library and create appropriate symlinks
    for (soname, libs) in soname_map {
        // Find the newest library (by modification time)
        let newest_lib = find_newest_library(&libs);

        // Create libname.so.X -> libname.so.X.Y.Z symlink
        let so_x_path = create_so_x_path(newest_lib.path.as_std_path(), &soname);

        if should_create_symlink(&so_x_path, newest_lib.path.as_std_path())? {
            actions.push(SymlinkAction {
                target: newest_lib.path.clone(),
                link: Utf8PathBuf::try_from(so_x_path.clone()).map_err(|_| {
                    LdconfigError::Config("Invalid UTF-8 in symlink path".to_string())
                })?,
                action: SymlinkActionType::Create,
            });

            if !dry_run {
                create_symlink(newest_lib.path.as_std_path(), &so_x_path)?;
            }
        }

        // Create libname.so -> libname.so.X symlink (if libname.so exists)
        let dev_symlink_path = create_dev_symlink_path(newest_lib.path.as_std_path(), &soname);
        if dev_symlink_path.exists() {
            if should_create_symlink(&dev_symlink_path, &so_x_path)? {
                actions.push(SymlinkAction {
                    target: Utf8PathBuf::try_from(so_x_path.clone()).map_err(|_| {
                        LdconfigError::Config("Invalid UTF-8 in symlink path".to_string())
                    })?,
                    link: Utf8PathBuf::try_from(dev_symlink_path.clone()).map_err(|_| {
                        LdconfigError::Config("Invalid UTF-8 in symlink path".to_string())
                    })?,
                    action: SymlinkActionType::Create,
                });

                if !dry_run {
                    create_symlink(&so_x_path, &dev_symlink_path)?;
                }
            }
        }
    }

    Ok(actions)
}

fn find_newest_library<'a>(libs: &'a [&'a ElfLibrary]) -> &'a ElfLibrary {
    libs.iter()
        .max_by(|a, b| {
            compare_soname_versions(&a.soname, &b.soname)
        })
        .unwrap_or(&libs[0])
}

/// Compare SONAME versions numerically (e.g., 1.2.10 > 1.2.9)
fn compare_soname_versions(a: &str, b: &str) -> std::cmp::Ordering {
    // Extract version from SONAME (e.g., "libfoo.so.1.2.3" -> [1, 2, 3])
    let extract_version = |s: &str| -> Vec<u32> {
        s.rsplit('.').take_while(|part| part.parse::<u32>().is_ok())
            .map(|part| part.parse::<u32>().unwrap())
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    };

    let ver_a = extract_version(a);
    let ver_b = extract_version(b);

    // Compare version components
    for (va, vb) in ver_a.iter().zip(ver_b.iter()) {
        match va.cmp(vb) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }

    // If all components equal, longer version wins
    ver_a.len().cmp(&ver_b.len())
}

use std::path::PathBuf;

fn should_create_symlink(link_path: &Path, target_path: &Path) -> Result<bool, LdconfigError> {
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

fn create_so_x_path(_real_path: &Path, soname: &str) -> PathBuf {
    // SONAME is already the correct symlink name (e.g., "libfoo.so.1")
    // Just use parent directory of real path + SONAME
    _real_path.parent().unwrap().join(soname)
}

fn create_dev_symlink_path(real_path: &Path, soname: &str) -> PathBuf {
    // For libfoo.so.1, create libfoo.so
    if let Some(parent) = real_path.parent() {
        let filename = real_path.file_name().unwrap().to_string_lossy();

        // Extract the base name
        if let Some(lib_name) = filename.strip_suffix(".so") {
            if let Some(first_dot) = lib_name.find('.') {
                let base = &lib_name[..first_dot];
                return parent.join(format!("{}.so", base));
            }
        }
    }

    // Fallback: remove the version from SONAME
    if let Some(last_dot) = soname.rfind('.') {
        let base = &soname[..last_dot];
        return real_path.parent().unwrap().join(format!("{}.so", base));
    }

    real_path.parent().unwrap().join(soname)
}
