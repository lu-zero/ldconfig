//! `ldconfig -p` with a reader that has already gone away.

use camino::Utf8PathBuf;
use ldconfig::{Cache, SearchPaths};
use std::io::Read;
use std::process::{Command, Stdio};

#[test]
fn a_closed_stdout_exits_with_sigpipe_status() {
    let dir = tempfile::tempdir().unwrap();
    let root = Utf8PathBuf::try_from(dir.path().to_path_buf()).unwrap();
    let cache_path = root.join("ld.so.cache");
    let cache = Cache::builder()
        .update_symlinks(false)
        .prefix(root.as_path())
        .build(&SearchPaths::new(Vec::new()))
        .unwrap();
    cache.write_to_file(&cache_path).unwrap();

    // Close the read end before the child starts, so the first write fails
    // instead of racing a full buffer.
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    let mut child = Command::new(env!("CARGO_BIN_EXE_ldconfig"))
        .args(["-p", "-C", cache_path.as_str()])
        .stdout(writer)
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let status = child.wait().unwrap();
    let mut err = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut err)
        .unwrap();

    assert!(err.is_empty(), "{err}");
    assert_eq!(status.code(), Some(141));
}
