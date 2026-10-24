# Security

omatach talks to a vehicle over an OBD-II adapter and reads two kinds of
untrusted input: the bytes an adapter sends back, and the Torque-format CSV
files a user drops in `~/.config/omatach/pids/`. It writes recordings and logs
under the user's own data directories. It makes no network requests, and the
only outbound action it can take is `xdg-open` on the one hard-coded donation
URL when you press `$`.

The risk surface worth reporting:

- a reply from an adapter that causes a panic, a hang, or a bad decode
- a CSV or equation that causes a panic or unbounded work
- anything that writes outside `~/.config/omatach`, `~/.local/share/omatach`
  or `~/.local/state/omatach`
- a mode `04` (clear codes) request reaching a vehicle without the key-on-
  engine-off confirmation

If you find one of those, or anything else you believe is a security issue,
email **info@greyhorsesoftware.com** rather than opening a public issue.
Include the adapter, the vehicle if relevant, and the bytes or the CSV row that
triggers it. We aim to acknowledge within a week.

Please do not report findings that require physically connecting a hostile
device to a vehicle you do not own.
