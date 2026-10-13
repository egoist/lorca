# Releasing the CLI

A computer without the app installs the `lorca` CLI with a script from the site:

```sh
curl -fsSL https://lorca.app/install-cli.sh | sh    # macOS and Linux
irm https://lorca.app/install-cli.ps1 | iex         # Windows, in PowerShell
```

The scripts, [`web/public/install-cli.sh`](../web/public/install-cli.sh) and
[`web/public/install-cli.ps1`](../web/public/install-cli.ps1), ship with the site and download
from this repo's GitHub releases, tagged `cli-vX.Y.Z`. A release holds one archive per build,
each with its SHA-256 beside it:

```
lorca-cli-macos-aarch64.tar.gz    Apple silicon
lorca-cli-linux-aarch64.tar.gz    static (musl), for any distribution
lorca-cli-linux-x86_64.tar.gz
lorca-cli-windows-x86_64.zip
<archive>.sha256
lorca-cli.json                    the version, and each archive's SHA-256 and size
lorca-cli.json.sig                an Ed25519 signature of lorca-cli.json
```

A CLI installed this way updates itself from the latest release, and installs only a
`lorca-cli.json` signed by a key it trusts ([Updates and the service](architecture/runtime.md#updates-and-the-service)).
[`.github/workflows/release-cli.yml`](../.github/workflows/release-cli.yml) builds, signs, tests,
and drafts them. It writes the release with the workflow's own token and signs with the
`CLI_UPDATE_SIGNING_KEY` secret.

## Update key

The key is an Ed25519 key pair in PEM. Its public half, base64 of the 32 raw bytes, is in `KEYS` in
[`crates/cli/src/update.rs`](../crates/cli/src/update.rs); the private half is the Actions secret
`CLI_UPDATE_SIGNING_KEY` (Settings ▸ Secrets and variables ▸ Actions). The key in use was made on
the maintainer's Mac and is kept at `~/Library/Application Support/lorca/update-keys/cli-update.key`;
keep a copy in a password manager. Without it no CLI in the field takes another release, and a
script install is the only way to a new one.

To make a key and read its public half:

```sh
openssl genpkey -algorithm ed25519 -out cli-update.key
openssl pkey -in cli-update.key -pubout -outform DER | tail -c 32 | base64
gh secret set CLI_UPDATE_SIGNING_KEY < cli-update.key
```

To move to a new key, add its public half to `KEYS` and release with the old key first, so the CLIs
in the field trust the new one before it signs; then set the secret to the new key.

## Cutting a release

The release is named after the crate's version, which `lorca --version` prints:

1. Set `version` in `[workspace.package]` of the root [`Cargo.toml`](../Cargo.toml), run
   `cargo check -p lorca` so `Cargo.lock` follows, and commit both.
2. Run the workflow on that commit, by hand or with a tag:

   ```sh
   gh workflow run release-cli.yml
   ```

   ```sh
   git tag cli-v1.0.0 && git push origin cli-v1.0.0
   ```

3. Publish the draft on GitHub, with **Set as the latest release** checked. The scripts download
   from `releases/latest/download/`, so a published CLI release has to be the repo's latest.

The workflow:

1. names the release `cli-v<version>` after the `lorca` crate. It refuses a pushed tag that says
   another version, and a tag that already exists on another commit, since a release takes the
   tag's commit. When that release is already published, it stops there;
2. builds `lorca` with `--release --locked` and `LORCA_SELF_UPDATE=1`, which makes a build that
   updates itself: `cargo build` on a macOS runner for the Mac, and `cargo zigbuild` on Linux for
   Linux (musl) and Windows (`x86_64-pc-windows-gnu`). Each binary goes alone into
   `lorca-cli-<os>-<cpu>.tar.gz` (`.zip` for Windows) with a `.sha256` beside it;
3. writes `lorca-cli.json` from the archives and signs it with `CLI_UPDATE_SIGNING_KEY`, after
   checking that the key's public half is in `KEYS`; it stops when the secret is missing;
4. serves the archives and the manifest the way a release does on Linux, macOS, and Windows
   runners and installs from them with the site's scripts: `install-cli.sh` twice, an install and
   then an update, and `install-cli.ps1` through `Invoke-Expression` in Windows PowerShell and then
   in PowerShell 7, checking `lorca --version`, the user PATH, and that `lorca update --check`
   reads the signed manifest and finds itself the latest;
5. drafts the release with the archives and the manifest, its tag to be made at the built commit
   when the draft is published. A draft left by an earlier run takes the new archives and keeps its
   title and notes.

Publishing the draft releases the update: every CLI installed with the scripts finds it within a
day and restarts into it once its bots are idle.

Runs go one at a time. To rebuild a published release, delete it and its tag first.

A release needs no deploy of the site. A change to the scripts ships with `bun run web:deploy`;
`web/public/_headers` serves them as UTF-8 text.

## The install scripts

- They read `LORCA_VERSION` (a release to install instead of the latest), `LORCA_INSTALL_DIR`
  (default `~/.local/bin`), `LORCA_NO_MODIFY_PATH=1` (leave PATH alone), and
  `LORCA_DOWNLOAD_URL` (the releases URL, `https://github.com/egoist/lorca/releases` by default;
  the workflow points it at its local server).
- They pick the archive for the OS and CPU, the arm64 build in a shell under Rosetta, and refuse
  an Intel Mac and Windows on Arm. They check the archive against its `.sha256` and move `lorca`
  into place by rename, so a running `lorca serve` keeps the file it started from.
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
