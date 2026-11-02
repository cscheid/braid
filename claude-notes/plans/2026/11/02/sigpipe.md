# Handle SIGPIPE properly for one-shot commands

## Overview

`braid list | head` panics with "failed printing to stdout: Broken pipe".
Rust's runtime sets `SIGPIPE` to ignore, so a write to a closed pipe returns
`EPIPE` and `println!` panics. The standard Unix behavior is to die quietly
on `SIGPIPE` (`ls | head` exits 141 with no message).

A global `signal(SIGPIPE, SIG_DFL)` in `main` is **not** the design: braid
has long-lived paths (`braid serve`, `braid mcp` over stdio), and
`SIGPIPE` disposition is process-wide, not per-fd. Rust ignores it so that
writes surface as `Err` the code can handle.

**Design:** restore `SIG_DFL` only for an **allowlist** of read-only commands
that print their result to stdout. An allowlist fails safe: a new command
added later keeps Rust's default (a panic on `| head`, the status quo)
rather than silently gaining kill-on-EPIPE semantics.

Key audit finding (see Details): *every* data command syncs (`open_skein`
dials and pulls), so "commands that don't sync" is nearly empty. What makes
the read commands safe is different: they all call `session.shutdown()`
(`repo.stop()`, which flushes the cache) **before** their first stdout
write. So the kill can only land after sync and cache flush are finished.

Rejected alternative: convert every `println!` in `commands.rs` (73 sites) to
a helper that treats `ErrorKind::BrokenPipe` as a clean exit. More
invasive, and a new `println!` reintroduces the bug (a `clippy.toml`
`disallowed-macros` entry could guard that). Its one real advantage is
that it also fixes Windows; see "Windows" under Details. Revisit if
Windows `| head` matters.

Corrections to the DeepWiki answer this plan started from: `main` is
`async fn main()` returning `()` (`crates/braid/src/main.rs:294`), not
`anyhow::Result<()>`; `mcp.rs` uses stdout as its protocol channel; `libc`
is in `Cargo.lock` (0.2.191) but is not yet a direct dependency of
`crates/braid`.

## Phase 1: tests first

- [x] Add `crates/braid/tests/sigpipe.rs`. For determinism, do not race a
      drop against the child's first write: create the pipe with
      `std::io::pipe()` (stable since 1.87; confirm the toolchain), `drop`
      the read end **before** spawning, and pass the write end as the
      child's stdout. Every stdout write then gets `EPIPE`, regardless of
      output size or timing, so no large fixture is needed.
- [x] Assert stderr (captured separately) contains no `panicked` / `Broken
      pipe`, and the exit is a SIGPIPE termination (unix
      `ExitStatusExt::signal() == Some(13)`; works on macOS and Linux,
      gate the test `#[cfg(unix)]`).
- [x] Cover one command per allowlist class: `list`, `show --json`,
      `export`, `agents-info` (no skein needed), `config`.
- [x] Reuse the existing test harness for skein setup (see how
      `tests/serve.rs` and the CLI tests create a skein and set
      `BRAID_SYNC_URL` / `BRAID_NO_CACHE`) so the child doesn't wait on a
      real network timeout.
- [x] Negative guard (cheap, deterministic): a non-allowlisted command,
      for example `braid create ...` with the closed-pipe stdout, must *not*
      die by signal 13 (it keeps Rust's default; it may panic or exit 0).
      This pins the allowlist boundary without needing a live server.
- [x] Confirm the positive tests fail on current `main` for the right
      reason: exit code 101 plus `panicked ... Broken pipe` on stderr.
- [x] Dropped: the `serve`/`mcp` "still running after stdout closes"
      guard test. `serve` prints one line at startup, and `mcp` only writes
      in response to requests, so the test is inherently timing-dependent.
      The allowlist plus the negative guard above cover the intent.

## Phase 2: implement

- [x] Add `libc = "0.2"` to `[workspace.dependencies]` in the root
      `Cargo.toml` (the repo pins shared deps there), and
      `[target.'cfg(unix)'.dependencies] libc = { workspace = true }` in
      `crates/braid/Cargo.toml`. No musl impact: libc is already compiled
      for every target via tokio/mio/std, and no C toolchain is needed.
- [x] In `main.rs`, add `#[cfg(unix)] fn restore_sigpipe()` calling
      `unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) }` with a
      `// SAFETY:` comment; a non-unix no-op. This is the repo's first
      `unsafe`; no lint forbids it.
- [x] Call it from the allowlisted match arms only (Details has the list),
      not before the `match`. Do not match on a denylist.
- [x] Verify the Phase 1 tests now pass.

## Phase 3: docs and verification

- [x] No CLI surface change, so `agents-info.md` and docs-drift are
      unaffected; confirm `cargo nextest run -p braid` keeps `docs_drift`
      green.
- [x] `cargo clippy -p braid --all-targets -- -D warnings`
- [x] `cargo nextest run -p braid`
- [ ] Manual check: `braid list | head -1; echo ${pipestatus[1]}` prints 141
      with no panic (zsh; `PIPESTATUS` in bash).
- [ ] `cargo xtask ci` before pushing.
- [x] `q2 render --strict claude-notes/plans/2026/11/02/sigpipe.md` before
      pushing (the file is under `claude-notes/`).
- [ ] If this is tracked as a strand, mark it `in_progress` with
      `BRAID_AUTHOR=claude` when starting and close it with a summary.
- [ ] Reconcile this checklist against what landed, then commit the plan.

## Details

### Phase 1 audit (done)

Every command except the config-only ones goes through
`Session::open` → `open_skein` (`ops.rs:256`, `skein.rs:73`), which dials
the server (`connect`, `skein.rs:116`) and `pull()`s (`skein.rs:131`). So
list/show/ready/etc. all sync. The read commands then call
`session.shutdown()` → `repo.stop()` (`ops.rs:335`, `skein.rs:201`) before
printing (`commands.rs` `list` ~1138, `show`, `ready`, `blocked`,
`search`, `dep_list`, `dep_cycles`, `dep_tree`, `export` ~945).

Classification:

| Command | Opens sync? | Prints after shutdown? | SIG_DFL? |
|---|---|---|---|
| `list`, `show`, `ready`, `blocked`, `search`, `dep list/tree/cycles`, `export` | yes | yes | **allow** |
| `agents-info` (no `--install`), `secret`, `config` | no | n/a | **allow** |
| `agents-info --install` | no | writes a file | allow is harmless; keep out for simplicity |
| `create`, `update`, `close`, `reopen`, `defer`, `undefer`, `comment`, `delete`, `dep add/remove`, `import`, `init`, `rotate`, `sync` | yes | prints around push/confirm | **exclude** (mutating; SIGPIPE mid-write/push is the real risk) |
| `mcp`, `serve`, `ui` | yes, long-lived | stdout is a protocol channel (`mcp`) or one startup line (`serve`) | **exclude** |

`secret` and `config` print many small lines; a kill partway through is
harmless.

Why a kill after `shutdown()` is safe: cache writes go through samod's
storage, which writes atomically (temp file + rename, `cache.rs:262-274`),
and the flush has completed by then. Network sockets are not a SIGPIPE
source on the paths we ship: std sends with `MSG_NOSIGNAL` on Linux
(`library/std/src/sys/net/connection/socket/mod.rs:433`) and mio sets
`SO_NOSIGPIPE` on Apple targets (`mio/src/sys/unix/net.rs:39`). So the
websocket/TCP sync path is already immune, and the real SIGPIPE exposure
is stdout/stderr only. That is why a global reset would be survivable for
`serve`, but the allowlist is still the right conservative default, and
for `mcp` a silent kill on host disconnect is a behavior change.

### Other writers and children

`ui.rs`, `serve.rs`, and `skein.rs` only use `eprintln!` (plus one startup
`println!` in `serve`). Panics on `eprintln!` to a closed stderr are a
separate, unaddressed case. `build.rs` and tests run `Command::new` (npm,
bash, tar); no production code spawns children, so no pipe interaction.
`xtask` is a dev tool, out of scope.

### Windows

Windows is built, tested (`ci.yml` windows job), and released
(`release.yml` x86_64-pc-windows-msvc, `install.ps1`). With the signal
approach, Windows keeps the current panic on `| head` (accepted: the
no-op is explicit). If Windows matters, the helper approach is the only
one that covers it (`BrokenPipe` / `ERROR_NO_DATA`).

### Third option: panic hook / catch_unwind

Possible (a hook that matches "failed printing to stdout" and exits 141),
but it depends on matching panic message text, and the panic fires inside
tokio's runtime context. Not recommended over either alternative.
