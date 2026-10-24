# Where omatach's shipped data comes from

Everything under `data/` is compiled into the binary by `build.rs` as one
deflated vault (see `data/manifest.toml`). Nothing is fetched at runtime and
nothing is written to disk. This file records where each entry came from.

## `j1979_catalog.json` — the Mode 01 PID catalog

383 datapoints across 162 Service 01 PID codes.

**Scope.** The shipped PID set is bounded by the Service 01 table in the
Wikipedia article ["OBD-II PIDs"](https://en.wikipedia.org/wiki/OBD-II_PIDs).
Audited 2026-10-24 against that table:

- the article lists 174 Service 01 PIDs with a description and a formula or range
- omatach ships 162 of them, and **nothing outside that set**
- deliberately not shipped: the support bitmaps `00 20 40 60 80 A0 C0`, which
  are blacklisted from the picker because they carry no value of their own, and
  `7F 81 82 89 8A`

`src/catalog.rs` pins this boundary in a test (`01A6` is listed and present,
`01B4` is unlisted and absent), so widening the set is a deliberate act with a
failing test in the way.

**Authorship.** Only the PID *numbers* follow that article. Every datapoint
name, short name, unit, min/max, category, decode equation and sensor
enable-test in this file is Grey Horse Software's own work, generated from the
PID knowledge base behind [RevOBD](https://greyhorsesoftware.com) — a product
of the same author. Equations are written in the Torque dialect and evaluated
by [obd_equation_rs](https://github.com/greyhorsesoftware/obd_equation_rs).

**Extended and manufacturer PIDs are not bundled.** They come only from
Torque-format CSV files a user drops in `~/.config/omatach/pids/`. That was a
scope decision from the first plan, not an omission.

## `dtcs.json` — trouble-code description libraries

Five libraries, selected by the VIN's WMI (make library first, then generic):

| library | entries | what it describes |
|---|---|---|
| `j1979` | 2,959 | generic P/B/C/U codes as defined by SAE J2012 |
| `ford` | 11,693 | Ford-defined codes |
| `fca` | 8,534 | FCA-defined codes |
| `gm_global_a` | 13,585 | GM Global A codes |
| `gm_global_b` | 7,505 | GM Global B codes, keyed with the failure-type suffix |

These are code-to-description tables: a five-character code mapped to the short
definition of what the code means. They describe diagnostic codes defined by
the standard and by each manufacturer, and are bundled so codes resolve to
readable text offline. Manufacturer names are used for identification only;
all trademarks belong to their owners. omatach is not affiliated with, endorsed
by, or a product of any vehicle manufacturer.

## `adapters.json`

Four OBDLink entries: BLE name patterns and service/characteristic UUIDs,
Bluetooth-classic and WiFi name patterns, and USB serial-port name patterns.
All of it is observable by scanning for the device, and it is matched by
`Catalog::resolve` in the session engine.

## `unitconversions.json`

Ten metric→imperial pairs as `m`/`b` linear coefficients. Standard unit
conversion factors.

## `mock/11bit.json`, `mock/29bit.json`

The Demo adapter's canned responses, derived from real captures with the VIN and
calibration identifiers **replaced by synthetic values** (`OMA11BIT000000001`,
`OMA29BIT000000002`), mode 22/06 captures dropped, and the support bitmaps
synthesized from the PIDs actually present. No real vehicle is identifiable
from them.
