# omatach

[![Built for Omarchy](https://raw.githubusercontent.com/tcballard/omarchy-badges/85f859029e236e784e7b05ada6dbe73506d07a91/badges/v1/built-for-omarchy.svg)](https://github.com/tcballard/omarchy-badges)

A terminal OBD-II dashboard for [Omarchy](https://omarchy.org) — vehicle info,
live gauges and trouble codes, in a tile, with OBDLink adapters. Free donationware.

```
─── omatach · OMA11BIT000000001 · CAN 11/500 ······································ ● connected ───

  RPM                        rpm  SPEED                     km/h  ECT                         °C
            ⣀⡠⠤⠤⢤⠤⠤⠤⢄⡀                                            ████████████░░░░░░░░░░░  91.0
         ⣀⠔⠉    ⠘    ⠈⠑⢄⡀                                          ▅▇▇▃     ▅▇▆▃    ▁▅█▆▂
       ⡠⢪               ⢨⠢⡀                                       █████▆▂▁▄█████▆▂▁▄█████▅
      ⡎  ⠁              ⠁ ⠈⡆              ╷ ╷ ╶─┐   ┌─┐
     ⡜ ⠠⢄⡀                 ⠘⡄             └─┤   │   │ │
    ⢰⠁   ⠈⠑⠢⢄⡀              ⢱               ╵   ╵ ▪ └─┘
    ⣜⣀⡀      ⠈⠑⠢⣄⡀         ⣀⣘⡄
               2340
 + add    - remove    ←→ select    s style    u units    R record    i info    t trouble    ? help    q quit
```

## Install

Arch / Omarchy:

```
yay -S omatach-bin        # or Omarchy menu → Install → Package
omatach
```

Any other Linux — x86_64 or aarch64, glibc 2.34 or newer:

```
curl -fsSL https://raw.githubusercontent.com/greyhorsesoftware/omatach/main/install.sh | bash
```

That verifies the release against its published SHA-256 and installs to
`~/.local/bin`, no sudo. `install.sh --help` documents `--prefix`, `--version`
and `--uninstall`; pass flags through the pipe with `bash -s -- --prefix /usr/local`.

From source — needs a C toolchain, since the one native library (libdbus) is
built in rather than linked:

```
cargo install --git https://github.com/greyhorsesoftware/omatach
```

No adapter? Pick **Demo adapter** in the picker — a simulated car with a live
drive cycle, so you can try every screen.

## Adapters

OBDLink only: **CX** (Bluetooth LE), **MX+ / LX** (Bluetooth classic — pair once
with `bluetoothctl pair <addr>`), **EX / SX** (USB), **MX WiFi**. The picker
lists what it finds; `↵` connects.

## Screens

| key | screen | what |
|---|---|---|
| `d` | Dashboard | gauges in three styles — dial, bar + sparkline, big digits. `+` picks PIDs, `s` cycles style, `u` metric/imperial, `R` records |
| `i` | Vehicle Info | VIN, protocol, ECUs with calibration ids, supported PIDs |
| `t` | Trouble Codes | stored / pending / permanent with descriptions (J1979 + Ford / GM / FCA libraries); `x` clears, with a key-on-engine-off confirm |

`/` opens a command palette, `?` lists every key, `$` is the tip jar, `q` quits.
Colours come from your terminal palette, so an Omarchy theme switch recolours it live.

## Recording

`R` on the dashboard writes a wide CSV to `~/.local/share/omatach/recordings/`:
`Time (s)` first, RPM and engine load pinned to columns B and C, one dense row
per update. It is the same format the RevOBD app records and charts.

## Your own PIDs

Drop Torque-format CSV files in `~/.config/omatach/pids/` and press `r` on the
dashboard (or restart). Rows need a hex header (`7E0`, `18DA10F1`) or none;
rows with a `VAL{…}` equation and no PID become computed values. Skipped rows
are explained in `~/.local/state/omatach/omatach.log`.

## Config

`~/.config/omatach/config.toml`

```toml
units = "metric"      # or "imperial" — the u key toggles and saves this
[ui]
braille = true        # false → dials become bars on terminals that render braille badly
```

Dashboard layouts are saved per vehicle under `~/.config/omatach/dashboards/`.

## Status

Tested and working on the OBDLink hardware — USB, WiFi, Bluetooth LE and
Bluetooth classic — as well as against the demo adapter. USB and WiFi share the
transport path already proven in RevOBD.

## Support

omatach is free. If it saved you a trip to the shop, `$` in the app opens the
donation link, or scan the QR it shows.

Built by [Grey Horse Software](https://greyhorsesoftware.com) on the same
engine as [RevOBD](https://greyhorsesoftware.com): the open-source
[obd_session_rs](https://github.com/greyhorsesoftware/obd_session_rs) and
[obd_equation_rs](https://github.com/greyhorsesoftware/obd_equation_rs).

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your
option — the same terms as the engine crates.

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
