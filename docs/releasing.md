# Cutting a release

A release can ship four tracks, each from its own tag: the CLI (`cli-vX.Y.Z`), the Windows and Linux app (`desktop-vX.Y.Z`), the Mac app (`mac-vX.Y.Z`), and the phone app (`mobile-vX.Y.Z`). A release usually bumps all four together, with patch versions, in one commit. Each track's one-time setup, secrets, and local builds are in its own doc: [CLI](releasing-cli.md), [desktop](releasing-desktop.md), [Mac](releasing-mac.md), [phone](releasing-mobile.md). This page is the order of steps across them.

## Before the bump

1. `main` is clean and matches `origin/main`. Unrelated untracked files stay out of the release commit.
2. List what changed since each track's last tag, for example `git log --oneline cli-v0.1.17..HEAD`. Commits often merge without changelog entries. A user-visible change gets an entry in the changelog of every app that shows it.
3. Choose the versions. Use a patch increment unless the change calls for more.

## Bump and notes

| Track | Version lives in | Notes live in | Tag |
| --- | --- | --- | --- |
| CLI | `version` under `[workspace.package]` in `Cargo.toml` | the apps' changelogs | `cli-vX.Y.Z` |
| Desktop | `version` in `desktop/mygo.config.ts` | `desktop/CHANGELOG.md` | `desktop-vX.Y.Z` |
| Mac | `"version"` in `package.json` | `CHANGELOG.md` | `mac-vX.Y.Z` |
| Phone | `const version` in `mobile/app.config.ts` | `mobile/CHANGELOG.md` | `mobile-vX.Y.Z` |

In each changelog, `## [Unreleased]` stays as an empty heading. The entries move under a new `## [X.Y.Z]` heading below it. The root `CHANGELOG.md` has one line per bullet. The desktop and phone changelogs wrap at 100 columns, with two-space continuation lines.

Run `cargo check -p lorca` without `--locked`, so `Cargo.lock` records the new CLI version. Only the workspace crates' version lines change.

Commit the seven files by path, not with `git add -A`:

```sh
git add Cargo.toml Cargo.lock package.json CHANGELOG.md desktop/mygo.config.ts desktop/CHANGELOG.md mobile/app.config.ts mobile/CHANGELOG.md
git commit
```

Use the message `bump cli/desktop/mac/mobile`, with a body that names each version and the notable changes. Push the branch on its own: `git push origin main`.

## Tags

Tag the pushed commit, then push each tag in its own `git push`:

```sh
git tag cli-v0.1.17 && git tag desktop-v0.1.12 && git tag mac-v1.0.17 && git tag mobile-v1.0.6
git push origin cli-v0.1.17
git push origin desktop-v0.1.12
git push origin mac-v1.0.17
git push origin mobile-v1.0.6
```

Push one tag per `git push`. GitHub starts no workflow for a push that carries more than three tags, and a push of four started none. If you pushed several together and no run starts, delete those remote tags with `git push origin --delete <tag>`, then push them one at a time.

Each tag must name the version in its file, or its workflow refuses it.

What each tag does:

- **CLI** drafts `cli-vX.Y.Z` with the archives and the signed manifest.
- **Desktop** drafts `desktop-vX.Y.Z` with the installers, and uses the changelog's section as the notes.
- **Mac** publishes as soon as its run finishes: a pushed `mac-v` tag uploads the disk image, the archives, and the appcast to R2, and installed copies find it there. No draft.
- **Phone** uploads the iOS build to TestFlight and drafts `mobile-vX.Y.Z` with the Android APK.

## Watch the runs

```sh
gh run list --limit 12
```

Four runs appear: Release CLI, Release desktop, Release Mac app, and Release phone app. A failed desktop or phone job can be re-run from the Actions tab. It uploads into the same draft. CLI runs go one at a time.

## Publish the drafts

Publishing a draft releases its update, so check its notes and assets first.

- CLI: `gh release edit cli-v0.1.17 --draft=false --latest`. The install scripts download from the latest release.
- Desktop: `gh release edit desktop-v0.1.12 --draft=false --latest=false`. The apps find it by its tag.
- Phone, Android: `gh release edit mobile-v1.0.6 --draft=false --latest=false`. Android apps see it at their next daily check.
- Mac: nothing to publish. The run has already put the release live.
- iOS: TestFlight lists the build once Apple has processed it.

## Before you push a tag

The Mac and iOS uploads reach users when their runs finish, so check the bump commit and the changelogs before pushing `mac-v` or `mobile-v`. To rebuild a published CLI release, delete it and its tag first ([CLI](releasing-cli.md#cutting-a-release)). The desktop script refuses a published version unless `FORCE=1` is set ([desktop](releasing-desktop.md#cutting-a-release)).
