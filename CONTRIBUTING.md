# Contributing to omatach

Thanks for looking. Small, focused pull requests are easiest to review.

## Before you open a PR

```
cargo build --locked
cargo test --locked
cargo rustc --locked -- -D warnings   # omatach itself must be warning-free
```

CI runs exactly those three on ubuntu-22.04. The engine crates are not held to
the warnings gate.

- **No captured vehicle data.** Tests use synthetic bytes, and the mock
  fixtures carry synthetic VINs and calibration ids. If you add a fixture,
  replace anything that identifies a real vehicle.
- **The shipped PID set is bounded** by the publicly listed Service 01 PIDs —
  see `data/PROVENANCE.md`, and the test in `src/catalog.rs` that enforces it.
  Extended and manufacturer PIDs belong in a user's own Torque CSV, not in the
  bundled catalog.
- **Data lives in the vault.** Anything under `data/` is compiled in by
  `build.rs` via `data/manifest.toml`; a new file needs a manifest entry, and
  `vault::names` gives you a compile-time constant for it. Nothing reads a data
  path at runtime.
- **Terminal output is themed by the terminal.** Use the palette's named
  colours rather than absolute RGB, so an Omarchy theme switch keeps working.
- **Equations are the Torque dialect**, evaluated by `obd_equation_rs`. Behaviour
  questions about the dialect belong in that repo.

## Hardware

Most work can be done against the Demo adapter (`omatach --mock demo`), which
runs a live drive cycle through every screen. `--dev` also lists the raw mock
fixtures. If a change touches a transport, say in the PR which adapter you
tested on — the four OBDLink families take different paths.

## Engine changes

The session engine and the equation evaluator are separate repos, pulled in as
tag-pinned git dependencies:

- [obd_session_rs](https://github.com/greyhorsesoftware/obd_session_rs) — link
  handlers, CAN identify, polling, transports
- [obd_equation_rs](https://github.com/greyhorsesoftware/obd_equation_rs) — the
  equation dialect

A fix that belongs there should go there first; bumping the tag in `Cargo.toml`
is then a one-line change here, with `Cargo.lock` in the same commit because CI
builds `--locked`.

## License

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in omatach by you, as defined in the Apache-2.0 license, shall be
dual licensed as MIT OR Apache-2.0 (see `LICENSE-MIT` and `LICENSE-APACHE`),
without any additional terms or conditions.
