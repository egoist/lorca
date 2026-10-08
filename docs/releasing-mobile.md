# Releasing the phone app

The phone app in `mobile/` ships to TestFlight on iOS and to this repository's GitHub releases on
Android, tagged `mobile-v<version>`. The version is `version` in
[`mobile/app.config.ts`](../mobile/app.config.ts), on both platforms. A TestFlight build carries
the time it was made, in UTC, as its build number, so any number of builds of one version can go
up; Android orders its builds by a versionCode made from the version (1.2.3 is 1002003), so each
Android release is a new version.

A tag builds both on GitHub Actions:

```sh
git tag mobile-v1.0.1 && git push origin mobile-v1.0.1
```

- Workflow: [`.github/workflows/release-mobile.yml`](../.github/workflows/release-mobile.yml).
- iOS: [`scripts/release-ios.ts`](../scripts/release-ios.ts) (`bun run release-ios`).
- Android: [`scripts/release-android.ts`](../scripts/release-android.ts) (`bun run release-android`).
- The Android updater: [`mobile/src/core/updates.ts`](../mobile/src/core/updates.ts) and
  `Updater.kt` in [`mobile/modules/lorca-core`](../mobile/modules/lorca-core/android/src/main/java/app/lorca/core/Updater.kt).

## How the Android app updates

A release holds `lorca-<version>-android.apk`. The repository's latest release stays the CLI's,
which `install-cli.sh` downloads, so the app reads the `mobile-v` tags through the GitHub API and
asks for the newest one's release; a draft or a prerelease is passed over. Once a day, at launch
or back in the foreground, the release build checks, and offers a newer version in an alert with
its notes: Install Update, Remind Me Later, Skip This Version. Settings › Updates checks at once
or installs what a check found, and holds the switch for the daily check.

Installing downloads the APK, checks it against the size and SHA-256 that GitHub lists for the
asset, checks that it is `app.lorca` and newer than the running build, and hands it to Android's
package installer. Android installs it only when it is signed with the installed app's key, and
then stops the app and replaces it; the user opens it again. The first time, Android asks the user
to let Lorca install apps, then to confirm the update. From Android 12 an app that may install
apps and updates itself is not asked to confirm again (`UPDATE_PACKAGES_WITHOUT_USER_ACTION`); some
phones, Xiaomi's among them, ask every time. Apart from that, on phones with Google Play, Play
Protect may stop an APK it has not seen before, a first install or an update, with "App scan
recommended": Scan app, or Install without scanning under More details.

Lorca Dev never updates, nor do iOS builds, which TestFlight updates.

## One-time setup

The workflow reads these Actions secrets (Settings ▸ Secrets and variables ▸ Actions). Each job
stops before building when one it needs is missing.

| Secret | What |
| --- | --- |
| `ASC_KEY` | The App Store Connect API key, the text of `AuthKey_<id>.p8` |
| `ASC_KEY_ID` | Its key ID |
| `ASC_ISSUER_ID` | The team's issuer ID |
| `IOS_SIGNING_IDENTITY` | An Apple Development certificate and its private key, as a base64 `.p12` |
| `IOS_SIGNING_IDENTITY_PASSWORD` | The `.p12`'s password |
| `ANDROID_KEYSTORE` | The Android release keystore, base64 |
| `ANDROID_KEYSTORE_PASSWORD` | Its password |

### 1. App Store Connect API key

On a Mac, `release-ios` signs in with the Apple account in Xcode. The runner has none, so it signs
in with an API key: App Store Connect ▸ Users and Access ▸ Integrations ▸ App Store Connect API ▸
Team Keys, a key with the **Admin** role. Admin lets the export sign with Apple's cloud-managed
distribution certificate, so no distribution certificate is kept anywhere; a key with a lesser
role fails the export with a cloud signing permission error. Download the `.p8` (App Store
Connect offers it once), and note the key ID and the issuer ID above the list.

```sh
gh secret set ASC_KEY < AuthKey_ABC123DEFG.p8
gh secret set ASC_KEY_ID --body ABC123DEFG
gh secret set ASC_ISSUER_ID --body 00000000-0000-0000-0000-000000000000
```

### 2. Apple Development identity

Automatic signing archives with an Apple Development certificate of team GJE9R5VE87 and re-signs
for the App Store at export. Without one in the keychain, Xcode would make a new certificate on
every run. Export the one Xcode made on the Mac that releases: Keychain Access ▸ login ▸ My
Certificates, the "Apple Development: …" item with its private key, File ▸ Export Items… as
`identity.p12` with a password.

```sh
base64 -i identity.p12 | gh secret set IOS_SIGNING_IDENTITY
gh secret set IOS_SIGNING_IDENTITY_PASSWORD
```

When that certificate expires or is revoked, export its successor the same way.

### 3. Android release key

The release key is a PKCS12 keystore with one key, alias `lorca`, whose certificate's SHA-256 is
`CERTIFICATE_SHA256` in [`scripts/release-android.ts`](../scripts/release-android.ts):

```
c2e73615dcd09e5580c536b9511b00e89d816b1d18a2aeed71bbef0daa8bd16c
```

`release-android` refuses an APK signed by any other key, since no installed app would take it.
The key in use was made on the maintainer's Mac and is kept at
`~/Library/Application Support/lorca/android-keys/release.keystore`, with its password in
`release.keystore.password` beside it, where `release-android` finds both (`ANDROID_KEYSTORE` and
`ANDROID_KEYSTORE_PASSWORD` name others). Keep a copy of both in a password manager. Without the
key no installed app can be updated again: each would have to be uninstalled, which deletes its
account and chats, and installed afresh.

```sh
keys="$HOME/Library/Application Support/lorca/android-keys"
base64 -i "$keys/release.keystore" | gh secret set ANDROID_KEYSTORE
gh secret set ANDROID_KEYSTORE_PASSWORD < "$keys/release.keystore.password"
```

It was made with:

```sh
keytool -genkeypair -keystore release.keystore -storetype PKCS12 -alias lorca \
  -keyalg RSA -keysize 4096 -validity 36500 -dname "CN=Lorca, O=Lorca"
```

### 4. Android developer verification

Certified Android devices install apps from outside Google Play only from verified developers:
enforced in Brazil, Indonesia, Singapore, and Thailand from September 30, 2026, and everywhere in
2027. Register `app.lorca` in the [Android Developer Console](https://developer.android.com/developer-verification/guides/android-developer-console)
with the certificate's SHA-256 above, once the developer account is verified.

## Cutting a release

1. Set `version` in [`mobile/app.config.ts`](../mobile/app.config.ts), and give it a
   `## [<version>]` section in [`mobile/CHANGELOG.md`](../mobile/CHANGELOG.md): the section becomes
   the release's notes and the update alert's text, and an Android release stops without it.
2. Tag the commit `mobile-v<version>` and push the tag, or run **Release phone app** by hand from
   the Actions tab, which drafts the release on the commit it runs on; publishing the draft makes
   the tag. A run on a commit other than the one an existing `mobile-v<version>` tag names is
   refused, and so is a version whose release is already published.
3. The iOS job uploads to App Store Connect, and TestFlight lists the build once Apple has
   processed it. The Android job drafts `mobile-v<version>` with the APK, or adds it to the draft an
   earlier run left. Check the draft, then publish it without making it the latest release:

   ```sh
   gh release edit mobile-v1.0.1 --draft=false --latest=false
   ```

   On the release's page, untick **Set as the latest release** before **Publish release**.
   Android apps see it at their next daily check.

A run by hand can release one platform, which is how a TestFlight build goes up without a new
version:

```sh
gh workflow run release-mobile.yml -f platforms=iOS
gh workflow run release-mobile.yml -f platforms=Android
```

The iOS job runs on the `xcode-27` image, a preview, so Xcode matches the Mac's.

## From a Mac

`bun run release-ios` uploads to TestFlight with the Apple account in Xcode, as before
(`--local` archives only), and `bun run release-android` writes
`dist/android/lorca-<version>-android.apk`, signed with the key in the folder above; upload it to
the draft with `gh release upload mobile-v<version> dist/android/lorca-<version>-android.apk`.
`release-android` needs the Android SDK (`ANDROID_HOME`, else `~/Library/Android/sdk`) with an
NDK, `cargo-ndk`, the Rust targets `aarch64-linux-android` and `x86_64-linux-android`, and a JDK
17 or later (`JAVA_HOME`, else Android Studio's). Each script generates the production project in
a copy (`dist/ios/mobile`, `dist/android/mobile`), so `mobile/ios` and `mobile/android` stay the
dev loop's Lorca Dev projects.

To try an update, install an older release's APK, then publish a newer one and choose
Settings › Updates › Check for Updates.
