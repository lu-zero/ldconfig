# Project Conventions

## Build Commands

There is no CI in this repository yet, so these checks are the gate. Run all
of them before opening a PR.

```bash
# Unit tests and doctests
cargo test

cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check

# Rustdoc warnings are treated as errors
RUSTDOCFLAGS='-D warnings' cargo doc --no-deps

# MSRV verification
cargo install cargo-msrv
cargo msrv verify
```

### Pre-PR checklist

| Check | Command |
|-------|---------|
| tests | `cargo test` (unit + **doctests**) |
| clippy | `cargo clippy --all-targets -- -D warnings` |
| fmt | `cargo fmt --all -- --check` |
| doc | `RUSTDOCFLAGS='-D warnings' cargo doc --no-deps` |
| parity | the `-p` comparison in [Testing](#testing), on a glibc host |

## Running the binary safely

**Never run `ldconfig` from this repository against the live system without
`-X` and `-C`.** Run bare, it does what the real tool does: it repoints soname
symlinks in `/usr/lib*` and replaces `/etc/ld.so.cache`. As root that changes
how every program on the host resolves its libraries.

- Read-only against the host: `-X` (no symlink changes) plus `-C <file>` (cache
  written somewhere harmless).
- Anything that should create or change symlinks: build a throwaway root and
  use `-r <root>`.
- `-p` only reads.

The same holds for the library: `Cache::builder()` updates symlinks by default.
Tests and examples pass `.update_symlinks(false)` or work in a `tempfile`
directory.

## Architecture

- One crate: a library and the `ldconfig` binary. Keep `src/bin/ldconfig.rs` a
  thin argument parser; behaviour belongs in the library so it stays reachable
  through the API.
- **Read [`docs/architecture.md`](./docs/architecture.md) first.** It is the
  main reference: module layout, the build pipeline, the cache file format,
  root handling and the known differences from glibc. Keep it updated as the
  design changes.
- The public surface is `Cache`, `CacheBuilder`, `CacheEntry`, `CacheInfo`,
  `SearchPaths`, `chroot_canon` and `Error`. Everything else is `pub(crate)`;
  widening it is an API decision, not a convenience.

## glibc fidelity

The reference is glibc's `elf/ldconfig.c`, `elf/cache.c`, `elf/readelflib.c`
and the `sysdeps/**/readelflib.c` variants.

- Output must match the real tool: same entries, same order, same path text,
  same flags. A divergence is a bug unless it is listed under "Differences from
  glibc" in the architecture document.
- When a function follows a glibc one, name it in the doc comment
  (`search_dir`, `create_links`, `parse_conf`). That is how the next reader
  finds the behaviour to compare against.
- A deliberate divergence gets one sentence saying what glibc does instead,
  and an entry in the architecture document.

### Licensing

This project is MIT-licensed; glibc is LGPL-2.1-or-later. Match glibc's
behaviour and its on-disk format, and write the implementation in your own
structure and wording. Do not paste glibc source, or a line-by-line
transliteration of it, into this tree. Constants and layouts that define the
file format are interface and are fine to reproduce.

## Dependencies

- `goblin` — ELF headers, program headers and the dynamic section
- `memmap2` — mapping libraries for inspection
- `camino` — UTF-8 paths in the public API
- `bon` — the `Cache` builder
- `bpaf` — the CLI
- `glob` — `include` patterns in `ld.so.conf`
- `tempfile` — atomic cache writes and atomic symlink replacement
- `thiserror`, `tracing`, `tracing-subscriber`

Dev only: `anyhow`, and `ld-so-cache` as an independent parser for
cross-validation in `examples/compare_caches.rs`.

Crates.io deps take a semver requirement, never an exact pin. `Cargo.lock` is
gitignored; floating within the requirement is intended. Do not add a
dependency for something a few lines of code cover.

## Coding Style

- `rustfmt` — all code must be formatted
- No dead code, no unused dependencies
- Doc comments on all public types and functions
- Tests live in a `#[cfg(test)] mod tests` block next to the code
- Keep doctests compiling and green; rustdoc under `RUSTDOCFLAGS=-D warnings`
  must stay clean
- Paths are `Utf8Path` / `Utf8PathBuf`, not `String`. A path that reaches a
  cache entry is text by then; until that point it keeps its type.
- Parsing untrusted bytes (a cache file, an ELF file) never panics and never
  indexes without a bounds check. Reject or skip; a library that cannot be
  parsed is not cached.
- Scanning is best-effort like glibc's: an unreadable directory or a broken
  library is a `debug!` or `warn!` and the run continues. Only failing to read
  or write the cache itself is an error.

### Output

- `println!` is for what the real tool prints on stdout: the `-p` listing.
- Diagnostics go through `tracing`. `warn!` for what glibc reports as a
  warning, `debug!` for what `-v` shows.
- A fatal CLI error goes through `die()` in the binary, which prints
  `ldconfig: <message>` and exits 1, like glibc.

### Unslop Rules

Enforced, not just a style nit — see [Slop Warning](#slop-warning) below.

**Comments**
- Default to no comment. Add one only when the *why* is non-obvious: a hidden
  constraint, a glibc quirk being reproduced, an invariant a future reader
  would violate without warning.
- Terse: one sentence beats a paragraph. A comment that needs several
  paragraphs to justify a few lines of code means the code needs simplifying.
- Never restate what the code already says through its own names.
- Never reference the current task, a commit, a PR or issue number, or a
  session. That context belongs in the commit message.
- No commented-out code and no `// removed: ...` markers; `git log` is the
  history.
- Hard limits: no comment line over **150 characters**, no comment paragraph
  over **5 lines** (3 is better). Hitting either means cut, don't wrap.

**Dead weight**
- No speculative abstraction for a single call site: no config knobs, trait
  generalisations or feature flags without a second concrete caller today.
- No error handling, fallback or validation for a scenario the caller's own
  guarantees already rule out.
- `#[allow(dead_code)]` is not a way to keep something "just in case" — delete
  it.

**Tests must exercise project logic, not the standard library**
- Don't add a test whose assertions would hold for any correct implementation
  of the underlying primitive.
- Prefer one test at the actual decision point over several that restate the
  same branch.
- A test that depends on the host (a library path, an architecture) must skip
  cleanly elsewhere; see `inspect_the_running_libc` in `src/elf.rs`.

## Commits

[Conventional Commits](https://www.conventionalcommits.org/):

- `feat:` — new functionality
- `fix:` — bug fix
- `refactor:` — code restructuring without behaviour change
- `docs:` — documentation only
- `test:` — adding or updating tests
- `ci:` — CI/CD changes
- `chore:` — maintenance (dependencies, tooling, releases)

Append `!` to the type for a breaking API or CLI change (`feat!:`).

Rules for the rest of the message:

- Imperative subject after the prefix, around 50 characters, no trailing
  period (`fix: replace soname symlinks atomically`).
- The body says why, not what the diff already shows. No body line over
  **150 characters**, no paragraph over **5 lines**.
- One logical change per commit, and every commit builds and passes the tests.
- User-visible changes get a `CHANGELOG.md` entry in the same series.

When a commit was significantly assisted by an AI tool, note it with an
`Assisted-by:` trailer rather than a `Co-Authored-By:` trailer. Use the
kernel's format (`AGENT_NAME:MODEL_VERSION`, colon-separated, e.g.
`Assisted-by: Claude:claude-opus-5-5`). The agent never adds a `Signed-off-by`
(DCO) — that is the human's.

## MSRV

`rust-version` in `Cargo.toml` is the floor (currently **1.90**). The crate
tracks latest stable dependencies; do not pin a crate to an older release to
satisfy a lower MSRV. When a dependency bump or a newly stabilised API needs a
newer compiler, raise `rust-version` and run `cargo msrv verify`.

## Testing

Unit tests cover config parsing, directory scanning, symlink handling, ELF
flag selection, sorting and the binary format. They build their own fixtures
in `tempfile` directories and never read `/etc/ld.so.conf`.

Parity with the real tool is the check that finds real bugs. On a glibc host,
without touching the system:

```bash
cargo build
target/debug/ldconfig -X -C /tmp/test.cache
diff <(target/debug/ldconfig -p -C /tmp/test.cache) <(/sbin/ldconfig -p)
```

Only the first line, which names the cache file, and the `Cache generated by:`
line may differ. `compare_caches` does the same
through an independent parser:

```bash
cargo run --example compare_caches -- /tmp/test.cache /etc/ld.so.cache
```

For another architecture, point `-r` at an unpacked stage3 or container root
and compare against the cache it ships. The byte order must match the host's.

Symlink behaviour is tested in a scratch root, never on the host:

```bash
mkdir -p /tmp/root/usr/lib64 /tmp/root/etc
cp /usr/lib64/libz.so.1.* /tmp/root/usr/lib64/
target/debug/ldconfig -r /tmp/root -v
```

## Slop Warning

This codebase was largely AI-generated. Be skeptical of existing code — it may
contain bugs or surprising behaviour. Do not assume existing patterns are
correct, including patterns in comments and docs: when one contradicts the
[Unslop Rules](#unslop-rules), fix it rather than copying it into new code.
