[![Built for Omarchy](https://raw.githubusercontent.com/tcballard/omarchy-badges/85f859029e236e784e7b05ada6dbe73506d07a91/badges/v1/built-for-omarchy.svg)](https://github.com/tcballard/omarchy-badges)

**omatach 1.0.0 — the source is now open.** A terminal OBD-II dashboard for Omarchy:
vehicle info, live gauges and trouble codes in a tile, with OBDLink adapters.
Free donationware, dual-licensed MIT OR Apache-2.0.

**Install**

Arch / Omarchy:

```
yay -S omatach-bin        # or Omarchy menu → Install → Package
```

Any other Linux — x86_64 or aarch64, glibc 2.34 or newer:

```
curl -fsSL https://raw.githubusercontent.com/greyhorsesoftware/omatach/main/install.sh | bash
```

Installs to `~/.local/bin` by default, no sudo. `--prefix`, `--version` and `--uninstall`
are documented by `install.sh --help`.

From source — needs a C toolchain, since the one native library (libdbus) is built in:

```
cargo install --git https://github.com/greyhorsesoftware/omatach
```

**In this release**

- **Source published** under MIT OR Apache-2.0. The two engine crates it builds on are
  public too: [obd_session_rs](https://github.com/greyhorsesoftware/obd_session_rs) 1.3.0
  (link handlers, CAN identify, tiered polling, BLE / classic / WiFi / USB transports) and
  [obd_equation_rs](https://github.com/greyhorsesoftware/obd_equation_rs) 1.0.0 (the
  Torque equation dialect). Both come in as tag-pinned git dependencies, so a plain
  `git clone` builds with no extra checkouts.
- **`install.sh`** for every non-Arch Linux: picks the right architecture, verifies the
  tarball against its published SHA-256, checks the glibc floor before writing anything,
  and can uninstall itself.
- Releases now live on the main repo rather than a separate downloads repo.

**What's in it**

- Dashboard with three gauge styles — dial, bar + sparkline, big digits — a PID picker,
  metric/imperial, per-vehicle saved layouts and CSV recording.
- Vehicle info: VIN, protocol, ECUs with calibration ids, supported PIDs.
- Trouble codes: stored / pending / permanent with descriptions, and a clear that confirms
  key-on-engine-off first.
- Your own PIDs via Torque-format CSV files in `~/.config/omatach/pids/`.
- A Demo adapter with a live drive cycle, so every screen works without hardware.
- Adapters: OBDLink CX, MX+ / LX, EX / SX, MX WiFi.
