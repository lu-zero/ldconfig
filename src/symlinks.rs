//! Symlink management, mirroring glibc's create_links.

use crate::chroot::chroot_canon;
use camino::Utf8Path;
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use tracing::{debug, warn};

/// stat() that resolves symlinks inside the -r root (glibc chroot_stat).
fn chroot_stat(prefix: &Utf8Path, real: &Utf8Path, logical: &Utf8Path) -> io::Result<fs::Metadata> {
    if prefix == "/" {
        return fs::metadata(real);
    }
    let md = fs::symlink_metadata(real)?;
    if !md.file_type().is_symlink() {
        return Ok(md);
    }
    let canon =
        chroot_canon(prefix, logical).ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
    fs::metadata(canon)
}

/// Create or update the `soname` -> `libname` symlink in one directory.
/// Never removes anything that is not a symlink.
pub(crate) fn create_link(
    prefix: &Utf8Path,
    real_dir: &Utf8Path,
    dir: &Utf8Path,
    libname: &str,
    soname: &str,
) {
    if libname == soname {
        return;
    }
    let link = real_dir.join(soname);
    let target = real_dir.join(libname);

    let mut do_remove = true;
    match chroot_stat(prefix, &link, &dir.join(soname)) {
        Ok(st_so) => {
            let Ok(st_lib) = chroot_stat(prefix, &target, &dir.join(libname)) else {
                warn!("Can't stat {}/{}", dir, libname);
                return;
            };
            if st_so.dev() == st_lib.dev() && st_so.ino() == st_lib.ino() {
                return; // link is already correct
            }
            match fs::symlink_metadata(&link) {
                Ok(md) if md.file_type().is_symlink() => {}
                _ => {
                    warn!("{}/{} is not a symbolic link", dir, soname);
                    return;
                }
            }
        }
        Err(_) => {
            // Unless it is a stale symlink, there is no need to remove.
            do_remove = matches!(fs::symlink_metadata(&link),
                                 Ok(md) if md.file_type().is_symlink());
        }
    }

    if do_remove {
        if let Err(e) = fs::remove_file(&link) {
            warn!("Can't unlink {}/{}: {}", dir, soname, e);
            return;
        }
    }
    match std::os::unix::fs::symlink(libname, &link) {
        Ok(()) => debug!("{} -> {} (changed)", soname, libname),
        Err(e) => warn!("Can't link {}/{} to {}: {}", dir, soname, libname, e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use camino::Utf8PathBuf;
    use std::os::unix::fs::symlink;

    fn setup() -> (tempfile::TempDir, Utf8PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = Utf8PathBuf::try_from(tmp.path().to_path_buf()).unwrap();
        fs::write(dir.join("libfoo.so.1.2.3"), b"x").unwrap();
        (tmp, dir)
    }

    fn link_target(dir: &Utf8Path, name: &str) -> Option<String> {
        fs::read_link(dir.join(name))
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
    }

    #[test]
    fn creates_missing_link() {
        let (_tmp, dir) = setup();
        create_link(
            Utf8Path::new("/"),
            &dir,
            &dir,
            "libfoo.so.1.2.3",
            "libfoo.so.1",
        );
        assert_eq!(link_target(&dir, "libfoo.so.1").unwrap(), "libfoo.so.1.2.3");
    }

    #[test]
    fn repoints_wrong_link() {
        let (_tmp, dir) = setup();
        fs::write(dir.join("libfoo.so.1.0"), b"x").unwrap();
        symlink("libfoo.so.1.0", dir.join("libfoo.so.1")).unwrap();
        create_link(
            Utf8Path::new("/"),
            &dir,
            &dir,
            "libfoo.so.1.2.3",
            "libfoo.so.1",
        );
        assert_eq!(link_target(&dir, "libfoo.so.1").unwrap(), "libfoo.so.1.2.3");
    }

    #[test]
    fn replaces_dangling_link() {
        let (_tmp, dir) = setup();
        symlink("libgone.so.9", dir.join("libfoo.so.1")).unwrap();
        create_link(
            Utf8Path::new("/"),
            &dir,
            &dir,
            "libfoo.so.1.2.3",
            "libfoo.so.1",
        );
        assert_eq!(link_target(&dir, "libfoo.so.1").unwrap(), "libfoo.so.1.2.3");
    }

    #[test]
    fn never_removes_regular_file() {
        let (_tmp, dir) = setup();
        fs::write(dir.join("libfoo.so.1"), b"real file").unwrap();
        create_link(
            Utf8Path::new("/"),
            &dir,
            &dir,
            "libfoo.so.1.2.3",
            "libfoo.so.1",
        );
        let md = fs::symlink_metadata(dir.join("libfoo.so.1")).unwrap();
        assert!(md.file_type().is_file());
        assert_eq!(fs::read(dir.join("libfoo.so.1")).unwrap(), b"real file");
    }

    #[test]
    fn correct_link_untouched() {
        let (_tmp, dir) = setup();
        symlink("libfoo.so.1.2.3", dir.join("libfoo.so.1")).unwrap();
        let before = fs::symlink_metadata(dir.join("libfoo.so.1")).unwrap().ino();
        create_link(
            Utf8Path::new("/"),
            &dir,
            &dir,
            "libfoo.so.1.2.3",
            "libfoo.so.1",
        );
        let after = fs::symlink_metadata(dir.join("libfoo.so.1")).unwrap().ino();
        assert_eq!(before, after);
    }
}
