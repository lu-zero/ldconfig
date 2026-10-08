# Architecture

The main architecture reference for this crate: the module layout, the path a
library takes from a configuration line to a cache entry, the cache file
format, how an alternate root is handled, and where the behaviour differs from
glibc's `ldconfig`.

> **Slop warning.** This codebase is largely AI-generated. Verify a claim
> against the code before relying on it; update this file when it drifts.

## Overview

One crate, a library plus the `ldconfig` binary. The binary is a thin argument
parser over the library: everything it does is reachable through the public
API.

The reference implementation is glibc's `elf/ldconfig.c` and the files around
it. Most modules name the glibc function they follow in their doc comments
(`search_dir`, `create_links`, `parse_conf`, `chroot_canon`, ...), which is the
quickest way to compare behaviour.

## Module layout

Lower modules know nothing of higher ones.

```
                 bin/ldconfig.rs  (CLI)
                        │
          ┌─────────────┼──────────────┐
          v             v              v
      config.rs      cache.rs      chroot.rs
     SearchPaths       Cache      chroot_canon
          │             │
          │   ┌─────────┼──────────┬──────────────┐
          │   v         v          v              v
          │ scanner.rs  symlinks.rs  cache_format.rs  atomic_write.rs
          │   │   │         │
          │   │   v         │
          │   │  elf.rs     │
          v   v             v
            chroot.rs
```

| Module | Visibility | Role |
|--------|------------|------|
| `lib.rs` | public | Re-exports and the public `Error` |
| `cache.rs` | public | `Cache`, its builder, `CacheEntry`, `CacheInfo`; drives a build |
| `config.rs` | public | `SearchPaths`; `ld.so.conf` parsing with `include` expansion |
| `chroot.rs` | public | `chroot_canon`: path resolution that cannot leave a root |
| `scanner.rs` | crate | Directory list setup and per-directory library selection |
| `elf.rs` | crate | Decides whether a file is cacheable and with which flags |
| `symlinks.rs` | crate | Creates and repoints soname symlinks |
| `cache_format.rs` | crate | Serialises and parses the binary cache; flag constants |
| `atomic_write.rs` | crate | Writes the cache file through a temp file and `rename(2)` |
| `error.rs` | private | The error enum wrapped by the public `Error` |

### Public API surface

`Cache`, `CacheBuilder`, `CacheEntry`, `CacheInfo`, `SearchPaths`,
`chroot_canon` and `Error`. Everything else is `pub(crate)`.

`Error` is an opaque newtype around the private enum in `error.rs`, so the
variants can change without breaking callers. Public functions return the
newtype; `cache_format.rs` returns the inner enum and `?` converts it.

## Building a cache

`Cache::builder().build(&search_paths)` runs these steps in order.

1. **Directory list** (`scanner::collect_dirs`). Each configured path is
   resolved under the root, dropped if it is missing or not a directory, and
   deduplicated by `(dev, ino)` keeping the first spelling. Every directory
   under `<dir>/glibc-hwcaps/` is queued right after its parent, in name
   order.
2. **Scan** (`scanner::scan_dir`). Each entry must look like a DSO by name
   (`lib*.so*`, `ld-*.so*`, `ld.so.*`, `ld64.so.*`) and must not be a
   packaging temp file (prelink, RPM `;`, `.dpkg-new`, `.dpkg-tmp`). A symlink
   is resolved inside the root and its target inspected; a dangling `*.so.*`
   link is removed when link updating is on.
3. **Inspect** (`elf::inspect`). The file is mapped and accepted only if it is
   an `ET_DYN` object in the host's byte order with a non-empty `PT_DYNAMIC`.
   The result is the `DT_SONAME` (the file name stands in when absent), the
   cache flags from `machine_flags`, and on x86 the ISA level from the
   `GNU_PROPERTY_X86_ISA_1_NEEDED` note.
4. **Pick one file per soname** (`scanner::merge_candidate`). Within a
   directory a regular file beats a symlink; otherwise the higher name by
   `_dl_cache_libcmp` wins. The first candidate's flags are kept, with a
   warning on mismatch, as glibc does.
5. **Symlinks** (`symlinks::create_link`). For regular directories, the
   `soname -> file` link is created or repointed unless it already resolves to
   the same inode. Something that is not a symlink is never replaced. Skipped
   with `update_symlinks(false)` or `dry_run(true)`.
6. **Entry text.** The cached path is `<configured dir>/<soname>` for regular
   directories, relying on the symlink, and `<configured dir>/<file name>` for
   `glibc-hwcaps` subdirectories. The configured spelling is used, never the
   on-disk path under the root.
7. **Serialise** (`cache_format::build_cache`), then parse the result back so
   a built `Cache` and one read from disk go through the same code.

Nothing is written to the cache file until `Cache::write_to_file`, which goes
through `atomic_write`: temp file in the target directory, `fsync`, mode
`0644`, `rename(2)`. The parent directory must already exist.

### Machine flags

`elf::machine_flags` merges the per-architecture `readelflib.c` variants that
glibc compiles one at a time. It matches on `e_machine` and each arm decides
which ELF classes it accepts, so one build of this crate can cache a sysroot of
any architecture.

A machine with no arm of its own gets the generic `FLAG_ELF_LIBC6`. Adding an
architecture means adding one arm, a test in `elf.rs`, and a line in
`cache_format::flags_string` if it introduces a new flag.

## Cache file format

Only the new format (`glibc-ld.so.cache1.1`) is read or written. All integers
are native-endian.

```
header (48 bytes)
  magic[20]            "glibc-ld.so.cache1.1"
  nlibs: u32
  len_strings: u32
  flags: u8            byte order marker, 0 = unset
  padding[3]
  extension_offset: u32
  unused[12]
entries (nlibs * 24 bytes)
  flags: u32           library type | required ABI
  key: u32             absolute file offset of the soname
  value: u32           absolute file offset of the path
  osversion: u32       unused, written as 0
  hwcap: u64           0, or bit 62 | isa_level << 32 | hwcaps index
string table (len_strings bytes, NUL-terminated, interned)
padding to a multiple of 4
extension directory
  magic: u32           0xEAA42174
  count: u32
  sections[count]      tag, flags, offset, size (4 * u32)
extension data
  tag 1                u32 string offsets of the glibc-hwcaps names
  tag 0                generator string, "ldconfig-rs <version>"
```

Entries are sorted the way `elf/cache.c` sorts them: reversed
`_dl_cache_libcmp` on the soname, then flags descending, then `glibc-hwcaps`
entries before plain ones ordered by subdirectory name. `ld.so` binary-searches
this order, so it is part of the format.

`parse_cache` rejects a truncated file, a wrong magic, a foreign byte order and
any string offset outside the string table. A malformed extension section is
ignored, like `ld.so` does. Strings are decoded lossily: a soname or path that
is not UTF-8 still yields an entry.

## Working inside a root

With a root (`-r`, the builder's `prefix`), three spellings of a path exist and
must not be mixed up:

- the **configured** path, as written in `ld.so.conf` (`/usr/lib`); this is
  the text stored in the cache
- the **real** path on the host (`<root>/usr/lib`), used for every file
  operation
- a symlink's target, which has to be re-resolved **inside** the root

`chroot_canon` does the third: it walks the path component by component,
restarts from the root on an absolute link target, stops `..` at the root, and
allows only the last component to be missing. `scanner::ScanDir` carries the
first two side by side (`path` and `real`).

The binary resolves the configuration file, `include` patterns and the cache
file's directory the same way, so nothing read or written can escape the root.

An `include` pattern is handled in two parts. Its leading wildcard-free
directory goes through `chroot_canon`; the rest is matched below that
directory on the host. Each match is then named back inside the root before
it is parsed, so a symlink reached through a wildcard is re-resolved inside
the root like any other. `SearchPaths::from_file` makes the root absolute
first, because that naming-back strips the root from host paths and has to
see one spelling of it.

## Differences from glibc

Intentional:

- **Atomic symlink updates.** glibc unlinks the old link and then creates the
  new one. Here the new link is created under a temporary `.ldconfig-*` name
  and renamed over the old one, so the soname resolves at every instant. The
  temporary name is neither scanned as a library nor swept as a stale link.
- **Every architecture in one binary.** glibc's `ldconfig` only knows the
  machine it was built for.
- **Relative `include` under a root.** glibc refuses a relative pattern when
  `-r` is given. Here it resolves against the including file's directory
  inside the root, exactly as it does without `-r`, so a stock
  `include ld.so.conf.d/*.conf` works when a root is processed from outside.
- **`include` wildcards in any component under a root.** glibc resolves the
  whole pattern inside the root before expanding it, so with `-r` a wildcard
  only works in the last component. Here `include conf.d/*/x.conf` matches
  under a root as it does without one.
- **`SearchPaths::from_file` omits the system directories.** Chain
  `.with_system()` to get glibc's list; the binary does.
- **Generator string** is `ldconfig-rs <version>`.

Not implemented:

- The old and compat cache formats (`-c old`, `-c compat`).
- The auxiliary cache (`/var/cache/ldconfig/aux-cache`), so `-i` has nothing
  to ignore; it is rejected today.
- Library mode (`-l`).
- `hwcap` directives in `ld.so.conf` are ignored with a warning, as in current
  glibc.
- A foreign byte order: both the ELF reader and the cache writer are
  native-endian, like glibc.

## Verifying against glibc

The strongest check is a byte-level or `-p` comparison with the host's own
`ldconfig`; see [`AGENTS.md`](../AGENTS.md#testing) for the commands and for
how to run the binary without touching the live system.
