# Developer manual

How to build, check and release Jiary, and where the rest of the developer
documentation lives.

- [implementation.md](implementation.md): how Jiary works inside (module map,
  data layer, screens, session lifecycle, rendering, testing) and why. Read
  it before changing code.
- [`DEVELOPMENT.md`](../../DEVELOPMENT.md) at the repo root: where things
  stand, what's next, the backlog and the working conventions. Start there
  when resuming work.
- The [user manual](../user/README.md): what Jiary does, from the user's
  side. Keep it in sync when behavior changes.

## Building

Requires Rust stable (edition 2024). No system dependencies: SQLite is
bundled.

```sh
cargo build --release      # the binary is target/release/jiary
cargo run                  # build and run a debug build
```

A debug build uses the same database as an installed Jiary, and only one
Jiary runs at a time, so quit the installed one first.

## Checking

```sh
./check
```

Runs everything that must pass before a commit: `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, the build and the tests. Tests
never touch the real data directory: each opens its own throwaway database.

## The docs' pictures

The pictures in `docs/images/` are drawn by the real UI code: ignored tests
(`src/app/screenshot.rs`) fill throwaway databases with made-up data, render
a screen into a test buffer and write the buffer out as an SVG. The main
screen gets a made-up day; the week, month and year views get about two
years of history from a fixed seed and show last week, last month and last
year, so they're complete and come out the same every run (bar the dates). Regenerate them after changing how these screens look:

```sh
cargo test screenshot -- --ignored
```

Run it during the day: the main screen's day is laid out over the six hours
before now, and all of it has to fall on today's date.

## Releasing

1. Bump `version` in `Cargo.toml`, write the release notes in
   `docs/releases/vX.Y.Z.md` (what's new, from the user's side, and anything
   to know when upgrading, such as a schema change), and commit.
2. Tag the commit and push the tag:

   ```sh
   git tag -a vX.Y.Z -m "Jiary X.Y.Z"
   git push origin vX.Y.Z
   ```

The release workflow (`.github/workflows/release.yml`) builds Jiary for macOS
and Linux, x86_64 and ARM, and publishes the archives with their SHA-256
checksums as a GitHub release, with `docs/releases/vX.Y.Z.md` as its notes
(GitHub's generated notes when there's no such file). `install.sh` downloads from the latest
release, so it picks the new one up straight away.

Runner images age out: if a build job fails to start, check that its
`runs-on` image is still offered (macOS builds moved to `macos-15` when
`macos-14` was retired).
