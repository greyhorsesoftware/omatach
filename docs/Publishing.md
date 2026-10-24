# Publishing omatach — from release archive to `yay -S omatach-bin`

Starting point: the release workflow has run with `publish` ticked (or a `v*`
tag was pushed) and the `omatach-<ver>-{x86_64,aarch64}-linux.tar.gz` tarballs
plus their `.sha256` files are attached to release `v<ver>` on
`greyhorsesoftware/omatach` itself. Everything below happens on the Omarchy
box, once per release.

The AUR package is one of three install channels and the only one that needs
this ritual. The other two need nothing per release: `install.sh` resolves the
latest release through the GitHub API, and `cargo install --git` builds from the
tag. The release workflow's `verify-install` job already proves the `install.sh`
path against the release it just published.

## 1. First time only — AUR account

1. Create an account at <https://aur.archlinux.org>.
2. My Account → SSH Public Key → paste `~/.ssh/id_ed25519_omatach.pub`.
3. Add to `~/.ssh/config` so git uses that key for the AUR:
   ```
   Host aur.archlinux.org
     IdentityFile ~/.ssh/id_ed25519_omatach
     User aur
   ```
4. Tools: `sudo pacman -S --needed base-devel pacman-contrib namcap`
   (`updpkgsums` comes from `pacman-contrib`).
5. Clone the package repo. The name does not exist yet — cloning it creates it
   on the first push:
   ```
   git clone ssh://aur@aur.archlinux.org/omatach-bin.git
   ```

## 2. Every release

Before tagging: update `RELEASE_NOTES.md` in the omatach repo — the workflow
uses it verbatim as the GitHub release description (keep the badge line at the
top; replace the "In this release" bullets).

```
cd omatach-bin
cp ~/Projects/omatach/packaging/PKGBUILD .    # source of truth for the PKGBUILD
```

1. Set `pkgver=<ver>` to match the release tag (without the `v`) and
   `pkgrel=1`. Bump only `pkgrel` when re-publishing the same version.
2. Fill the checksum from the published tarball:
   ```
   updpkgsums
   ```
   It downloads the archive from the `source=` URL and rewrites
   `sha256sums=()`. If the download fails, the release is not public or the
   version in the URL is wrong — fix that first.
3. Build and install locally as the real test:
   ```
   makepkg -si
   omatach                    # picker shows the Demo adapter; walker shows the entry
   namcap PKGBUILD omatach-bin-*.pkg.tar.zst   # lint; warnings about deps are worth reading
   ```
4. Regenerate the index the AUR actually reads:
   ```
   makepkg --printsrcinfo > .SRCINFO
   ```
5. Push:
   ```
   git add PKGBUILD .SRCINFO
   git commit -m "omatach-bin <ver>"
   git push
   ```
   Do not add the built package or the downloaded tarball; the AUR repo holds
   only `PKGBUILD` and `.SRCINFO`.
6. Copy the finished `PKGBUILD` back over `packaging/PKGBUILD` in the omatach
   repo so the two copies agree (the AUR's is the published one).

Within a minute `yay -S omatach-bin` and Omarchy's Install → Package find the
new version; users on the old one see it as an update.

## 3. What to check after the first publish

- <https://aur.archlinux.org/packages/omatach-bin> shows the version, the
  description and the `greyhorsesoftware/omatach` URL.
- On a clean Omarchy install: Install → Package → `omatach` → it installs,
  `omatach` runs, the walker entry launches it in a terminal and it tiles.
- Switching the Omarchy theme recolours the running app without a restart.
- A classic-Bluetooth OBDLink pairs with `bluetoothctl` and then appears in the
  picker (`bluez-utils` is an optdepend, so the package does not pull it in).

## Notes

- The AUR is the only registry Omarchy uses; Install → Package searches it.
  Install → TUI only registers a launcher for something already installed —
  the package's `.desktop` file already does that.
- `license=('MIT OR Apache-2.0')` matches the source licence; the tarball
  carries both LICENSE files and the PKGBUILD installs them into
  `/usr/share/licenses/omatach-bin/`.
- Both `x86_64` and `aarch64` tarballs are published; the PKGBUILD carries a
  `source_<arch>` / `sha256sums_<arch>` pair for each and `updpkgsums` fills
  both. `makepkg` picks the pair for the machine it runs on.
- Never commit from this machine's checkout of `omatach-bin` without running
  `makepkg -si` first; a broken PKGBUILD is public immediately.
