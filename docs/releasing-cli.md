# Releasing the CLI

A computer without the app installs the `lorca` CLI with a script from the site:

```sh
curl -fsSL https://lorca.app/install-cli.sh | sh    # macOS and Linux
irm https://lorca.app/install-cli.ps1 | iex         # Windows, in PowerShell
```

The scripts, [`web/public/install-cli.sh`](../web/public/install-cli.sh) and
[`web/public/install-cli.ps1`](../web/public/install-cli.ps1), ship with the site. They download
from the GitHub releases of the public repo
[egoist/lorca-releases](https://github.com/egoist/lorca-releases), since this repo is private and
its releases need a sign-in. A release holds one archive per build, each with its SHA-256 beside
it:

```
lorca-cli-macos-aarch64.tar.gz    Apple silicon
lorca-cli-linux-aarch64.tar.gz    static (musl), for any distribution
lorca-cli-linux-x86_64.tar.gz
lorca-cli-windows-x86_64.zip      Windows 11 on Arm runs it too
<archive>.sha256
```

[`.github/workflows/release-cli.yml`](../.github/workflows/release-cli.yml) builds, tests, and
publishes them when a `v*` tag is pushed.

## One-time setup

1. Create the public repo with a first commit, which a release's tag points at:

   ```sh
   gh repo create egoist/lorca-releases --public --add-readme --description "Lorca CLI releases"
   ```

2. Create a fine-grained personal access token for that repository alone, with **Contents: Read
   and write**.
3. Save it in this repo as the Actions secret `RELEASES_TOKEN`:

   ```sh
   gh secret set RELEASES_TOKEN --repo egoist/lorca
   ```

## Cutting a release

The tag names the release and `lorca --version` prints the crate's version, so the workflow
refuses a tag the two do not share:

1. Set `version` in `[workspace.package]` of the root [`Cargo.toml`](../Cargo.toml), run
   `cargo check -p lorca` so `Cargo.lock` follows, and commit both.
2. Tag that commit and push the tag:

   ```sh
   git tag v1.0.0
   git push origin v1.0.0
   ```

The workflow:

1. checks the tag against the version of the `lorca` crate;
2. builds `lorca` with `--release --locked`: `cargo build` on a macOS runner for the Mac, and
   `cargo zigbuild` on Linux for Linux (musl) and Windows (`x86_64-pc-windows-gnu`). Each binary
   goes alone into `lorca-cli-<os>-<cpu>.tar.gz` (`.zip` for Windows) with a `.sha256` beside it;
3. serves the archives the way a release does on Linux, macOS, and Windows runners and installs
   from them with the site's scripts: `install-cli.sh` twice, an install and then an update, and
   `install-cli.ps1` through `Invoke-Expression` in Windows PowerShell and then in PowerShell 7,
   checking `lorca --version` and the user PATH;
4. creates the release `v<version>` in egoist/lorca-releases with the archives and checksums.

Running it by hand (**Actions ▸ Release CLI ▸ Run workflow**, or `gh workflow run release-cli.yml`)
builds and tests without publishing. To publish a tag again, delete its release and tag in
egoist/lorca-releases and re-run the workflow.

A release needs no deploy of the site: the scripts download from `releases/latest/download/`,
which GitHub points at the newest release. A change to the scripts ships with
`bun run web:deploy`; `web/public/_headers` serves them as UTF-8 text.

## The install scripts

- They read `LORCA_VERSION` (a release to install instead of the latest), `LORCA_INSTALL_DIR`
  (default `~/.local/bin`), `LORCA_NO_MODIFY_PATH=1` (leave PATH alone), and
  `LORCA_DOWNLOAD_URL` (the releases URL, `https://github.com/egoist/lorca-releases/releases` by
  default; the workflow points it at its local server).
- They pick the archive for the OS and CPU, the arm64 build in a shell under Rosetta, and refuse
  an Intel Mac. They check the archive against its `.sha256` and move `lorca` into place by
  rename, so a running `lorca serve` keeps the file it started from.
- The shell script adds the folder to PATH in the profile of the user's shell (`.zshrc`,
  `.bashrc`, fish's `conf.d`, else `.profile`); the PowerShell one adds it to the user `Path` in
  the registry.

## Notes

- **Two versions.** The CLI's is the crate's, `[workspace.package]` in `Cargo.toml`; the Mac
  app's is `"version"` in `package.json`, which `bun run release-mac` bumps.
- **Mac builds carry only the linker's ad-hoc signature.** `curl` sets no quarantine flag, so
  Gatekeeper never assesses what the script installs. A tarball opened from a browser download
  would need a Developer ID signature and notarization.
- **The Windows build** is linked by zig against the Universal CRT and needs nothing beyond
  Windows 10. Bots' commands run in Git for Windows' bash there (`LORCA_SHELL` picks another
  shell).
- **Locally**, `cargo zigbuild --release -p lorca --target <target>` builds the Linux and Windows
  binaries, given `brew install zig`, `cargo install --locked cargo-zigbuild`, and
  `rustup target add <target>` for the toolchain `cargo` runs.
