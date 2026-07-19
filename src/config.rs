//! ld.so.conf parsing, mirroring glibc's parse_conf.

use crate::chroot::chroot_canon;
use crate::error::Error;
use camino::{Utf8Path, Utf8PathBuf};
use std::fs;
use std::io::ErrorKind;
use std::ops::Deref;
use tracing::warn;

/// Built-in system directories, appended after the configured ones like
/// glibc's add_system_dir calls. /usr precedes the top-level aliases so
/// that merged-usr systems cache the /usr path text, as their glibc does.
const SYSTEM_DIRS: [&str; 4] = ["/usr/lib", "/usr/lib64", "/lib", "/lib64"];

const MAX_INCLUDE_DEPTH: u32 = 32;

/// List of directories to scan for libraries
///
/// This is a simple wrapper around `Vec<Utf8PathBuf>` that provides
/// convenient constructors for creating directory lists from config files
/// or defaults. Paths are as configured, without the -r prefix applied.
#[derive(Debug, Clone)]
pub struct SearchPaths(Vec<Utf8PathBuf>);

impl SearchPaths {
    /// Parse a configuration file. `path` names the file inside `prefix`
    /// (the -r root); includes are expanded in place and resolved inside
    /// the prefix.
    pub fn from_file(path: impl AsRef<Utf8Path>, prefix: Option<&Utf8Path>) -> Result<Self, Error> {
        let prefix = prefix
            .map(|p| p.as_str().trim_end_matches('/'))
            .filter(|p| !p.is_empty())
            .map(Utf8Path::new);

        let mut dirs = Vec::new();
        parse_conf(path.as_ref(), prefix, &mut dirs, 0);
        Ok(Self(dirs))
    }

    /// Append the built-in system directories to the search paths.
    pub fn with_system(mut self) -> Self {
        self.0.extend(SYSTEM_DIRS.map(Utf8PathBuf::from));
        self
    }

    /// Create config from explicit directory list
    pub fn new(directories: Vec<Utf8PathBuf>) -> Self {
        Self(directories)
    }
}

impl Default for SearchPaths {
    /// Create default config (standard system directories)
    fn default() -> Self {
        Self(SYSTEM_DIRS.map(Utf8PathBuf::from).to_vec())
    }
}

impl Deref for SearchPaths {
    type Target = [Utf8PathBuf];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<[Utf8PathBuf]> for SearchPaths {
    fn as_ref(&self) -> &[Utf8PathBuf] {
        &self.0
    }
}

impl From<Vec<Utf8PathBuf>> for SearchPaths {
    fn from(directories: Vec<Utf8PathBuf>) -> Self {
        Self(directories)
    }
}

/// Directive keyword followed by a blank. glibc matches `include`
/// case-sensitively but `hwcap` case-insensitively.
fn directive<'a>(line: &'a str, keyword: &str, ignore_case: bool) -> Option<&'a str> {
    // checked: a non-char-boundary can only occur when the prefix is not
    // the ASCII keyword, so None is simply "no match".
    let (head, rest) = line.split_at_checked(keyword.len())?;
    let matches = if ignore_case {
        head.eq_ignore_ascii_case(keyword)
    } else {
        head == keyword
    };
    (matches && rest.starts_with([' ', '\t'])).then_some(rest)
}

fn parse_conf(file: &Utf8Path, prefix: Option<&Utf8Path>, dirs: &mut Vec<Utf8PathBuf>, depth: u32) {
    if depth > MAX_INCLUDE_DEPTH {
        warn!("{}: include nesting too deep", file);
        return;
    }
    let real = match prefix {
        Some(p) => match chroot_canon(p, file) {
            Some(r) => r,
            None => return,
        },
        None => file.to_path_buf(),
    };
    let content = match fs::read_to_string(&real) {
        Ok(c) => c,
        Err(e) if e.kind() == ErrorKind::NotFound => return,
        Err(e) => {
            warn!(
                "Warning: ignoring configuration file that cannot be opened: {}: {}",
                file, e
            );
            return;
        }
    };

    for line in content.lines() {
        // '#' anywhere terminates the line; no quoting exists.
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = directive(line, "include", false) {
            for pattern in rest.split_whitespace() {
                expand_include(file, prefix, pattern, dirs, depth);
            }
        } else if directive(line, "hwcap", true).is_some() {
            warn!("{}: hwcap directive ignored", file);
        } else {
            let dir = line.trim_end_matches('/');
            if !dir.is_empty() {
                dirs.push(Utf8PathBuf::from(dir));
            }
        }
    }
}

fn expand_include(
    from: &Utf8Path,
    prefix: Option<&Utf8Path>,
    pattern: &str,
    dirs: &mut Vec<Utf8PathBuf>,
    depth: u32,
) {
    if prefix.is_some() && !pattern.starts_with('/') {
        warn!(
            "{}: need absolute file name for configuration file when using -r",
            from
        );
        return;
    }
    // Relative patterns resolve against the including file's directory.
    let pattern = if pattern.starts_with('/') {
        Utf8PathBuf::from(pattern)
    } else {
        match from.parent() {
            Some(dir) if !dir.as_str().is_empty() => dir.join(pattern),
            _ => Utf8PathBuf::from(pattern),
        }
    };
    let glob_pattern = match prefix {
        Some(p) => match chroot_canon(p, &pattern) {
            Some(c) => c,
            None => return,
        },
        None => pattern.clone(),
    };

    let paths = match glob::glob(glob_pattern.as_str()) {
        Ok(paths) => paths,
        Err(e) => {
            warn!("{}: bad include pattern {}: {}", from, pattern, e);
            return;
        }
    };
    for entry in paths {
        let real = match entry {
            Ok(p) => match Utf8PathBuf::try_from(p) {
                Ok(p) => p,
                Err(_) => continue,
            },
            Err(e) => {
                warn!("{}: cannot read {}: {}", from, pattern, e);
                continue;
            }
        };
        // Recurse with the path inside the prefix so nested includes
        // resolve there too.
        let logical = match prefix {
            Some(p) => match real.strip_prefix(p) {
                Ok(rel) => Utf8PathBuf::from(format!("/{}", rel)),
                Err(_) => real,
            },
            None => real,
        };
        parse_conf(&logical, prefix, dirs, depth + 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Utf8Path, content: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn tempdir() -> (tempfile::TempDir, Utf8PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let path = Utf8PathBuf::try_from(tmp.path().to_path_buf()).unwrap();
        (tmp, path)
    }

    #[test]
    fn includes_expand_in_place_and_recurse() {
        let (_tmp, root) = tempdir();
        write(
            &root.join("ld.so.conf"),
            "include ld.so.conf.d/*.conf\n/opt/lib # trailing comment\n",
        );
        write(
            &root.join("ld.so.conf.d/a.conf"),
            "/a/lib\ninclude sub/*.conf\n",
        );
        write(&root.join("ld.so.conf.d/b.conf"), "/b/lib\n");
        write(&root.join("ld.so.conf.d/sub/n.conf"), "/nested/lib\n");

        let paths = SearchPaths::from_file(root.join("ld.so.conf"), None).unwrap().with_system();
        let dirs: Vec<&str> = paths.iter().map(|d| d.as_str()).collect();
        // Include expands where it appears (before /opt/lib), nested
        // includes work, system dirs come last.
        assert_eq!(
            dirs,
            [
                "/a/lib",
                "/nested/lib",
                "/b/lib",
                "/opt/lib",
                "/usr/lib",
                "/usr/lib64",
                "/lib",
                "/lib64",
            ]
        );
    }

    #[test]
    fn non_ascii_lines_are_directories_not_panics() {
        let (_tmp, root) = tempdir();
        write(
            &root.join("ld.so.conf"),
            "/lib/\u{65e5}\ninclud\u{e9} x\n/ok/lib\n",
        );
        let paths = SearchPaths::from_file(root.join("ld.so.conf"), None).unwrap();
        let dirs: Vec<&str> = paths.iter().map(|d| d.as_str()).collect();
        assert_eq!(dirs[..3], ["/lib/\u{65e5}", "includ\u{e9} x", "/ok/lib"]);
    }

    #[test]
    fn hwcap_directive_ignored_and_comments_stripped() {
        let (_tmp, root) = tempdir();
        write(
            &root.join("ld.so.conf"),
            "# comment\nhwcap 0 nosegneg\n  /spaced/lib  \n/slash/lib///\n",
        );
        let paths = SearchPaths::from_file(root.join("ld.so.conf"), None).unwrap();
        let dirs: Vec<&str> = paths.iter().map(|d| d.as_str()).collect();
        assert_eq!(dirs[..2], ["/spaced/lib", "/slash/lib"]);
    }

    #[test]
    fn prefix_confines_includes_to_root() {
        let (_tmp, root) = tempdir();
        // Config inside the "chroot" includes /etc/ld.so.conf.d/*.conf;
        // the glob must resolve under the root, not the host.
        write(
            &root.join("etc/ld.so.conf"),
            "include /etc/ld.so.conf.d/*.conf\n",
        );
        write(&root.join("etc/ld.so.conf.d/x.conf"), "/x/lib\n");

        let paths = SearchPaths::from_file(Utf8Path::new("/etc/ld.so.conf"), Some(&root)).unwrap();
        let dirs: Vec<&str> = paths.iter().map(|d| d.as_str()).collect();
        assert_eq!(dirs[0], "/x/lib");
    }

    #[test]
    fn missing_config_yields_system_dirs() {
        let (_tmp, root) = tempdir();
        let paths = SearchPaths::from_file(root.join("nonexistent.conf"), None).unwrap().with_system();
        let dirs: Vec<&str> = paths.iter().map(|d| d.as_str()).collect();
        assert_eq!(dirs, ["/usr/lib", "/usr/lib64", "/lib", "/lib64"]);
    }
}
