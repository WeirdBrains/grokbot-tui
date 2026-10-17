# Packaging and releases

Releases are built by GitHub Actions automatically: push or merge to `main` with a bumped
version in `Cargo.toml` and a release gets built. Nothing is built or signed on a local machine, and nothing is published outside this repository's GitHub
Releases unless you complete the manual steps below.

## What a release produces

| Target | Artifact on the GitHub Release | Built by |
|---|---|---|
| Linux x86_64 / aarch64 | `grokbot-tui-<target>.tar.xz` | cargo-dist (`release.yml`) |
| macOS x86_64 / arm64 | `grokbot-tui-<target>.tar.xz` | cargo-dist |
| Windows x64 | `grokbot-tui-x86_64-pc-windows-msvc.zip` | cargo-dist |
| Shell / PowerShell installers | `grokbot-tui-installer.sh`, `grokbot-tui-installer.ps1` | cargo-dist |
| Homebrew (macOS + Linux) | `grokbot-tui.rb` formula | cargo-dist |
| macOS installer | `grokbot-tui-<target>.pkg` (**unsigned**) | cargo-dist (`pkgbuild`/`productbuild`) |
| Windows installer | `grokbot-tui-x86_64-pc-windows-msvc.msi` (**unsigned**, WiX) | cargo-dist |
| Debian / Ubuntu | `grokbot-tui_<ver>-1_<arch>.deb` | `build-linux-packages.yml` (cargo-deb) |
| Fedora / RHEL | `grokbot-tui-<ver>-1.<arch>.rpm` | `build-linux-packages.yml` (cargo-generate-rpm) |
| Arch / AUR | `PKGBUILD` + `.SRCINFO` for `grokbot-tui-bin` and `grokbot-tui` | `publish-manifests.yml` |
| winget | version / installer / locale manifests (winget-pkgs layout) | `publish-manifests.yml` |
| Source | `source.tar.gz` | cargo-dist |

Every artifact has a matching `.sha256` file; `sha256.sum` lists cargo-dist's.
The AUR and winget files are bundled as `packaging-manifests-<tag>.tar.gz`.

Archive names start with `grokbot-tui-`, the Cargo package name (also the binary's name).
Releases before 1.2.0 used `gb-` (the old package name). No `.dmg` is built; the `.pkg` is the macOS installer.

## Cutting a release

1. Bump `version` in `[package]` in `Cargo.toml`.
2. In `CHANGELOG.md`, rename `[Unreleased]` to the new version and date, then add a
   new empty `[Unreleased]` section.
3. Commit and push (or merge a pull request) to `main`. That's all:
   `.github/workflows/auto-release.yml` takes it from there:
   - runs CI (`ci.yml`: fmt, clippy `-D warnings`, tests, release build) and stops if it fails;
   - reads the version from `Cargo.toml`; the release tag is `v<version>`;
   - does nothing if a GitHub Release for that tag already exists, or a Release run for it
     is already queued or running (so pushes that don't bump the version, and repeated
     pushes, never make a second release);
   - otherwise creates and pushes the annotated tag `v<version>` on the pushed commit
     (unless the tag already exists, see below) and starts `release.yml` for it.

   **Tagging yourself still works.** If you create and push `v<version>` yourself (e.g.
   `git push origin main v1.2.0`), the tag is used as is: auto-release runs for the
   branch push and for the tag push, one of them starts the release, the other sees it
   queued or done and stops. A pushed tag that doesn't match the `Cargo.toml` version at
   that commit is ignored. If a release run fails, the next push to `main` (or re-running
   the failed workflow) tries again.

   **Why `workflow_dispatch`:** tags pushed with the default `GITHUB_TOKEN` don't trigger
   other workflows, so `release.yml` no longer starts on tag pushes. It's started with
   `workflow_dispatch` (`dispatch-releases = true` in `dist-workspace.toml`), which
   `GITHUB_TOKEN` is allowed to trigger. No personal access token or extra secret is
   needed; the job only uses `contents: write` (push the tag) and `actions: write`
   (start the run). To start a release by hand: Actions → Release → Run workflow, with
   the tag as input (`gh workflow run release.yml --ref v1.2.0 -f tag=v1.2.0`).

   A version with a pre-release suffix such as `1.3.0-rc.1` makes a GitHub pre-release
   and skips the manifest job.
4. GitHub Actions runs `.github/workflows/release.yml`:
   - **`plan`**: decides what to build. On pull requests it runs only this check.
   - **`build-local-artifacts`**: native builds for each target (archives, `.pkg`, `.msi`).
   - **`build-global-artifacts`**: installers, Homebrew formula, checksums.
   - **`custom-build-linux-packages`**: `.deb` and `.rpm` on x86_64 and arm64 runners
     (the `.deb` is installed and smoke-tested).
   - **`host`**: creates the GitHub Release and attaches everything.
   - **`custom-publish-manifests`**: downloads the release's `.sha256` files and
     source tarball, renders the PKGBUILDs and winget manifests
     (`packaging/render-manifests.sh`), generates `.SRCINFO` in an Arch container,
     checks the PKGBUILDs with `namcap`, and attaches the bundle.

To preview locally: `dist plan --tag v1.2.0` (install cargo-dist 0.33.0).

Pull requests and pushes to other branches run `ci.yml`; `release.yml` also runs its
`plan` step on pull requests.

### Changing the release configuration

- cargo-dist config is in `dist-workspace.toml` (plus `[profile.dist]` in `Cargo.toml`).
  After editing it, run `dist generate` to regenerate `release.yml`. Do not hand-edit
  `release.yml`; CI checks that it matches the config.
  `auto-release.yml` and `ci.yml` are ours, not generated, so `dist generate` leaves them
  alone; keep `dispatch-releases = true`, or auto-release can't start `release.yml`.
- `wix/main.wxs` is hand-edited (product named `grokbot-tui`) and excluded
  from that check with `allow-dirty = ["msi"]`. Keep the upgrade GUIDs in
  `Cargo.toml` unchanged, or Windows upgrades will install side by side.
- `.deb` / `.rpm` contents: `[package.metadata.deb]` and
  `[package.metadata.generate-rpm]` in `Cargo.toml`.
- AUR / winget templates: `packaging/aur/*/PKGBUILD.in`, `packaging/winget/*.yaml.in`.
  Identity values are in `packaging/identity.env`. URLs and checksums are filled in
  at release time; never commit real checksums into the templates.

## Package identity

| Value | Where | Value |
|---|---|---|
| Homebrew tap repo | `dist-workspace.toml` `tap` | `WeirdBrains/homebrew-tap` (`brew install WeirdBrains/tap/grokbot-tui`) |
| macOS bundle identifier | `dist-workspace.toml` `[dist.mac-pkg-config] identifier` | `io.github.weirdbrains.grokbot-tui` |
| winget PackageIdentifier | `packaging/identity.env` `WINGET_ID` | `WeirdBrains.grokbot-tui` |
| winget Publisher / PublisherUrl | `packaging/identity.env` `PUBLISHER`, `PUBLISHER_URL` | `WeirdBrains` / `https://github.com/WeirdBrains` |
| Maintainer name/email (to confirm) | `Cargo.toml` `authors`, `[package.metadata.deb] maintainer`, `packaging/identity.env` `MAINTAINER` | from git history |

## License metadata

The project is licensed under BSD-3-Clause (`LICENSE` at the repo root). Each format
takes the license from `[package] license` in `Cargo.toml`:

| Format | License field | License text shipped |
|---|---|---|
| Archives (`.tar.xz` / `.zip`) | n/a | `LICENSE` (cargo-dist includes it automatically) |
| Homebrew formula | `license "BSD-3-Clause"` | in the archive |
| `.deb` | `debian/copyright`: `License: BSD-3-Clause` | full text in `/usr/share/doc/grokbot-tui/copyright` |
| `.rpm` | `License: BSD-3-Clause` | `/usr/share/licenses/grokbot-tui/LICENSE` |
| AUR PKGBUILDs | `license=('BSD-3-Clause')` (SPDX, as Arch now requires) | `/usr/share/licenses/<pkgname>/LICENSE` |
| winget | `License: BSD-3-Clause`, `LicenseUrl`, `Copyright` | link to `LICENSE` at the release tag |
| `.msi` / `.pkg` | none | not included in the installer (the binary only) |

To also install the license file with the MSI, follow the "license sidecar" comments in
`wix/main.wxs` (untested; needs a Windows build to verify).

## Manual steps that need your accounts or secrets

None of these are automated. A release succeeds without them; they only make the
packages installable from each ecosystem's usual place.

1. **Homebrew tap** (GitHub account).
   - Create the public repo `WeirdBrains/homebrew-tap` (the `tap` setting).
   - Manual: each release, copy `grokbot-tui.rb` from the release into `Formula/` in the
     tap.
   - Automatic (optional):
     1. Add `"homebrew"` to `publish-jobs` in `dist-workspace.toml` and run `dist generate`.
     2. Create a fine-grained token with *Contents: read/write* on `WeirdBrains/homebrew-tap` only.
     3. Save it as the repo secret `HOMEBREW_TAP_TOKEN`.
   - Users then run `brew install WeirdBrains/tap/grokbot-tui`.
   - Getting into homebrew-core is a separate, stricter review.
2. **AUR** (AUR account with an SSH key).
   - For each of `grokbot-tui-bin` (prebuilt) and `grokbot-tui` (source build), take
     `PKGBUILD` and `.SRCINFO` from `packaging-manifests-<tag>.tar.gz`, then:
     ```sh
     git clone ssh://aur@aur.archlinux.org/grokbot-tui-bin.git
     cp PKGBUILD .SRCINFO grokbot-tui-bin/
     cd grokbot-tui-bin && git commit -am "Update to <ver>" && git push
     ```
   - Repeat every release. Automating this would need an `AUR_SSH_KEY` secret and a
     push job (not set up).
3. **winget** (GitHub account; Microsoft CLA on your first PR).
   - Copy `winget/manifests/w/WeirdBrains/grokbot-tui/<ver>/` from the bundle into a fork
     of `microsoft/winget-pkgs`, at the same path.
   - Check with `winget validate --manifest <dir>` on Windows, then open a PR
     (or use `wingetcreate submit <dir>`).
   - Unsigned MSIs are accepted but may be flagged by automated scans and SmartScreen.
4. **macOS signing and notarization** (Apple Developer Program, paid).
   - The `.pkg` and binaries are unsigned, so Gatekeeper blocks the `.pkg` until the user
     allows it.
   - To sign:
     - get a *Developer ID Application* certificate (binaries) and a
       *Developer ID Installer* certificate (`.pkg`);
     - configure cargo-dist's `macos-sign` with its `CODESIGN_*` secrets;
     - add `productsign` and `notarytool` steps for the `.pkg` (not wired up).
   - Homebrew, `.tar.xz` and the shell installer don't need this for CLI use.
5. **Windows signing** (paid certificate).
   - The `.msi` and `.exe` are unsigned (SmartScreen warns).
   - cargo-dist supports `ssldotcom-windows-sign` or `azure-windows-sign`
     (Azure Trusted Signing), configured with secrets.
6. **Linux repositories** (optional).
   - `.deb` / `.rpm` are only attached to the GitHub Release.
   - Publishing to an apt/dnf repository (PPA, Copr, Cloudsmith, …) is not set up.
7. **GitHub settings.**
   - Actions must be enabled for the repository.
   - The workflows request `contents: write` themselves; no secrets are needed for a
     basic release.
   - The arm64 Linux builds use GitHub's `ubuntu-22.04-arm` runners.
