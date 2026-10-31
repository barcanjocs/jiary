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

## The README's screenshot

`docs/screenshot.svg` is drawn by the real UI code: an ignored test
(`src/app/screenshot.rs`) fills a throwaway database with a made-up day,
renders the main screen into a test buffer and writes the buffer out as an
SVG. Regenerate it after changing how the main screen looks:

```sh
cargo test screenshot -- --ignored
```

Run it during the day: the sample day is laid out over the six hours before
now, and all of it has to fall on today's date.

## Releasing

1. Bump `version` in `Cargo.toml` and commit.
2. Tag the commit and push the tag:

   ```sh
   git tag -a vX.Y.Z -m "Jiary X.Y.Z"
   git push origin vX.Y.Z
   ```

The release workflow (`.github/workflows/release.yml`) builds Jiary for macOS
and Linux, x86_64 and ARM, and publishes the archives with their SHA-256
checksums as a GitHub release. `install.sh` downloads from the latest
release, so it picks the new one up straight away.

Runner images age out: if a build job fails to start, check that its
`runs-on` image is still offered (macOS builds moved to `macos-15` when
`macos-14` was retired).
