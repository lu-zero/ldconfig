//! Path canonicalization inside an alternate root, port of glibc's
//! elf/chroot_canon.c.

use camino::{Utf8Path, Utf8PathBuf};
use std::fs;

const ELOOP_MAX: u32 = 40;

/// Canonicalize `name` as if chroot(`root`) was done first: resolves `.`,
/// `..`, and symlinks without ever escaping `root`. Every component except
/// the last must exist. Returns the path including the `root` prefix.
pub(crate) fn chroot_canon(root: &Utf8Path, name: &Utf8Path) -> Option<Utf8PathBuf> {
    let root = root.as_str().trim_end_matches('/');
    if root.is_empty() {
        return Some(name.to_path_buf());
    }

    let mut pending: Vec<String> = name
        .as_str()
        .split('/')
        .rev()
        .filter(|c| !c.is_empty())
        .map(str::to_owned)
        .collect();
    let mut stack: Vec<String> = Vec::new();
    let mut links = 0u32;

    while let Some(comp) = pending.pop() {
        match comp.as_str() {
            "." => continue,
            ".." => {
                stack.pop();
                continue;
            }
            _ => {}
        }
        stack.push(comp);

        let mut cur = String::from(root);
        for c in &stack {
            cur.push('/');
            cur.push_str(c);
        }

        match fs::symlink_metadata(&cur) {
            Err(_) => {
                if pending.is_empty() {
                    break;
                }
                return None;
            }
            Ok(md) if md.file_type().is_symlink() => {
                links += 1;
                if links > ELOOP_MAX {
                    return None;
                }
                let target = Utf8PathBuf::try_from(fs::read_link(&cur).ok()?).ok()?;
                stack.pop();
                if target.as_str().starts_with('/') {
                    stack.clear();
                }
                for c in target.as_str().split('/').rev().filter(|c| !c.is_empty()) {
                    pending.push(c.to_owned());
                }
            }
            Ok(_) => {}
        }
    }

    let mut out = String::from(root);
    for c in &stack {
        out.push('/');
        out.push_str(c);
    }
    if out.is_empty() {
        out.push('/');
    }
    Some(Utf8PathBuf::from(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn resolves_inside_root() {
        let tmp = tempfile::tempdir().unwrap();
        let root = Utf8PathBuf::try_from(tmp.path().to_path_buf()).unwrap();
        fs::create_dir_all(root.join("usr/lib")).unwrap();
        symlink("usr/lib", root.join("lib")).unwrap();

        let canon = chroot_canon(&root, Utf8Path::new("/lib/libx.so")).unwrap();
        assert_eq!(canon, root.join("usr/lib/libx.so"));
    }

    #[test]
    fn absolute_symlink_stays_inside_root() {
        let tmp = tempfile::tempdir().unwrap();
        let root = Utf8PathBuf::try_from(tmp.path().to_path_buf()).unwrap();
        fs::create_dir_all(root.join("usr/lib")).unwrap();
        symlink("/usr/lib", root.join("lib64")).unwrap();

        let canon = chroot_canon(&root, Utf8Path::new("/lib64")).unwrap();
        assert_eq!(canon, root.join("usr/lib"));
    }

    #[test]
    fn missing_last_component_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let root = Utf8PathBuf::try_from(tmp.path().to_path_buf()).unwrap();
        fs::create_dir_all(root.join("etc")).unwrap();

        let canon = chroot_canon(&root, Utf8Path::new("/etc/missing.conf")).unwrap();
        assert_eq!(canon, root.join("etc/missing.conf"));
        assert_eq!(chroot_canon(&root, Utf8Path::new("/nodir/x/y")), None);
    }

    #[test]
    fn dotdot_stops_at_root() {
        let tmp = tempfile::tempdir().unwrap();
        let root = Utf8PathBuf::try_from(tmp.path().to_path_buf()).unwrap();
        fs::create_dir_all(root.join("etc")).unwrap();

        let canon = chroot_canon(&root, Utf8Path::new("/../../etc")).unwrap();
        assert_eq!(canon, root.join("etc"));
    }
}
