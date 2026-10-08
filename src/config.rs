//! ld.so.conf parsing, mirroring glibc's parse_conf.

use crate::chroot::chroot_canon;
use crate::Error;
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
    ///
    /// Only the configured directories are returned; chain
    /// [`with_system`](Self::with_system) for what glibc scans. As in
    /// glibc, a missing file gives an empty list and an unreadable one a
    /// warning. The only error is a `prefix` that cannot be made absolute or
    /// is not UTF-8.
    pub fn from_file(path: impl AsRef<Utf8Path>, prefix: Option<&Utf8Path>) -> Result<Self, Error> {
        // An absolute root with no `.` components or trailing slash: include
        // matches come back from the filesystem and are mapped into the root
        // by stripping it, which only works on one spelling.
        let prefix = match prefix.filter(|p| !p.as_str().is_empty()) {
            Some(p) => {
                let absolute = Utf8PathBuf::try_from(std::path::absolute(p)?)
                    .map_err(|e| std::io::Error::new(ErrorKind::InvalidData, e))?;
                (absolute.as_str() != "/").then_some(absolute)
            }
            None => None,
        };

        let mut dirs = Vec::new();
        parse_conf(path.as_ref(), prefix.as_deref(), &mut dirs, 0);
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
    // Relative patterns resolve against the including file's directory, with
    // or without a root. glibc refuses them under -r, which drops the
    // `include ld.so.conf.d/*.conf` every distribution config starts with.
    let pattern = if pattern.starts_with('/') {
        Utf8PathBuf::from(pattern)
    } else {
        match from.parent() {
            Some(dir) if !dir.as_str().is_empty() => dir.join(pattern),
            _ => Utf8PathBuf::from(pattern),
        }
    };
    let Some(prefix) = prefix else {
        for real in glob_paths(from, pattern.as_str()) {
            parse_conf(&real, None, dirs, depth + 1);
        }
        return;
    };

    // Only the leading literal directory is resolved inside the root; the
    // wildcard part is matched below it, and each match named back inside
    // the root so nested includes and symlinks stay confined there.
    let (dir, rest) = split_at_wildcard(&pattern);
    let Some(real_dir) = chroot_canon(prefix, &dir) else {
        return;
    };
    let glob_pattern = format!("{}/{rest}", glob::Pattern::escape(real_dir.as_str()));
    for real in glob_paths(from, &glob_pattern) {
        if let Ok(below) = real.strip_prefix(&real_dir) {
            parse_conf(&dir.join(below), Some(prefix), dirs, depth + 1);
        }
    }
}

/// The UTF-8 paths matching `pattern`, in glob order; problems are warnings.
fn glob_paths(from: &Utf8Path, pattern: &str) -> Vec<Utf8PathBuf> {
    let paths = match glob::glob(pattern) {
        Ok(paths) => paths,
        Err(e) => {
            warn!("{}: bad include pattern {}: {}", from, pattern, e);
            return Vec::new();
        }
    };
    paths
        .filter_map(|entry| match entry {
            Ok(p) => Utf8PathBuf::try_from(p).ok(),
            Err(e) => {
                warn!("{}: cannot read {}: {}", from, pattern, e);
                None
            }
        })
        .collect()
}

/// Split `pattern` into its leading wildcard-free directory and the rest,
/// which starts at the first component with a wildcard, or is the file name.
fn split_at_wildcard(pattern: &Utf8Path) -> (Utf8PathBuf, String) {
    let components: Vec<&str> = pattern.as_str().split('/').collect();
    let cut = components
        .iter()
        .position(|c| c.contains(['*', '?', '[']))
        .unwrap_or(components.len() - 1);
    let dir = components[..cut].join("/");
    let dir = if dir.is_empty() && pattern.as_str().starts_with('/') {
        "/".to_owned()
    } else {
        dir
    };
    (Utf8PathBuf::from(dir), components[cut..].join("/"))
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

        let paths = SearchPaths::from_file(root.join("ld.so.conf"), None)
            .unwrap()
            .with_system();
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

    // Distribution configs use `include ld.so.conf.d/*.conf`; under a root
    // that must still pull in the files next to the including one.
    #[test]
    fn relative_include_resolves_inside_the_root() {
        let (_tmp, root) = tempdir();
        write(
            &root.join("etc/ld.so.conf"),
            "include ld.so.conf.d/*.conf\n/opt/lib\n",
        );
        write(
            &root.join("etc/ld.so.conf.d/a.conf"),
            "/a/lib\ninclude sub/*.conf\n",
        );
        write(&root.join("etc/ld.so.conf.d/sub/n.conf"), "/nested/lib\n");

        let paths = SearchPaths::from_file(Utf8Path::new("/etc/ld.so.conf"), Some(&root)).unwrap();
        let dirs: Vec<&str> = paths.iter().map(|d| d.as_str()).collect();
        assert_eq!(dirs, ["/a/lib", "/nested/lib", "/opt/lib"]);
    }

    fn rooted_dirs(root: &Utf8Path) -> Vec<String> {
        let paths = SearchPaths::from_file(Utf8Path::new("/etc/ld.so.conf"), Some(root)).unwrap();
        paths.iter().map(|d| d.to_string()).collect()
    }

    // `ldconfig -r ./sysroot` is a natural spelling of the root.
    #[test]
    fn includes_survive_a_relative_root() {
        let (_tmp, absolute) = tempdir();
        // `./../../…/tmp/x`: the scratch root as seen from the working
        // directory, without writing into the source tree.
        let depth = std::env::current_dir().unwrap().components().count() - 1;
        let root = Utf8PathBuf::from(format!(
            "./{}{}",
            "../".repeat(depth),
            absolute.as_str().trim_start_matches('/')
        ));
        write(
            &root.join("etc/ld.so.conf"),
            "include ld.so.conf.d/*.conf\n",
        );
        write(&root.join("etc/ld.so.conf.d/a.conf"), "/a/lib\n");

        assert_eq!(rooted_dirs(&root), ["/a/lib"]);
    }

    #[test]
    fn include_wildcards_work_in_any_component_under_a_root() {
        let (_tmp, root) = tempdir();
        write(&root.join("etc/ld.so.conf"), "include conf.d/*/x.conf\n");
        write(&root.join("etc/conf.d/one/x.conf"), "/one/lib\n");
        write(&root.join("etc/conf.d/two/x.conf"), "/two/lib\n");

        assert_eq!(rooted_dirs(&root), ["/one/lib", "/two/lib"]);
    }

    #[test]
    fn a_root_path_with_glob_characters_is_taken_literally() {
        let tmp = tempfile::Builder::new().prefix("a[b]*").tempdir().unwrap();
        let root = Utf8PathBuf::try_from(tmp.path().to_path_buf()).unwrap();
        write(
            &root.join("etc/ld.so.conf"),
            "include ld.so.conf.d/*.conf\n",
        );
        write(&root.join("etc/ld.so.conf.d/a.conf"), "/a/lib\n");

        assert_eq!(rooted_dirs(&root), ["/a/lib"]);
    }

    #[test]
    fn includes_cannot_leave_the_root() {
        let (_tmp, base) = tempdir();
        let root = base.join("root");
        write(&base.join("outside/o.conf"), "/OUTSIDE/lib\n");
        write(
            &root.join("etc/ld.so.conf"),
            "include ../../outside/*.conf\ninclude linked.d/*.conf\ninclude any.d/*/o.conf\n",
        );
        // A symlink out of the root, named literally and reached by a wildcard.
        std::fs::create_dir_all(root.join("etc/any.d")).unwrap();
        for link in ["etc/linked.d", "etc/any.d/evil"] {
            std::os::unix::fs::symlink(base.join("outside"), root.join(link)).unwrap();
        }

        assert_eq!(rooted_dirs(&root), Vec::<String>::new());
    }

    #[test]
    fn missing_config_yields_no_dirs_until_with_system() {
        let (_tmp, root) = tempdir();
        let paths = SearchPaths::from_file(root.join("nonexistent.conf"), None).unwrap();
        assert!(paths.is_empty());
        let paths = paths.with_system();
        let dirs: Vec<&str> = paths.iter().map(|d| d.as_str()).collect();
        assert_eq!(dirs, ["/usr/lib", "/usr/lib64", "/lib", "/lib64"]);
    }
}
