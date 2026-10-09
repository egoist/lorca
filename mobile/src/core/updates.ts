// The Android app's updates. Its releases are this repository's on GitHub, tagged mobile-vX.Y.Z,
// each holding the APK lorca-X.Y.Z-android.apk; the repository's latest release stays the CLI's,
// so the app finds the newest published mobile-v release itself, from the tags. Once a day, at
// launch or back in the foreground, it checks and offers a newer version in an alert (Install
// Update, Remind Me Later, Skip This Version); Settings › Updates checks at once or installs what a
// check found, and holds the switch for the daily check. The native module downloads the APK, checks it against the size and
// SHA-256 GitHub lists for it, and hands it to the package installer, which replaces the app.
// iOS updates through TestFlight, and Lorca Dev never updates.

import * as Application from "expo-application";
import { AppState, Platform } from "react-native";
import { create } from "zustand";
import * as core from "../../modules/lorca-core";
import { t } from "../i18n";
import { loadPrefs, savePrefs } from "./prefs";
import { alert } from "../ui/alert";

const API = "https://api.github.com/repos/egoist/lorca";
const TAG_PREFIX = "mobile-v";
const DAY = 24 * 60 * 60 * 1000;

/// A newer release than this build.
export interface Release {
  version: string;
  notes: string;
  url: string;
  sha256: string;
  size: number;
}

export interface Updates {
  phase: "idle" | "checking" | "downloading" | "installing";
  /// The newest release, while it is newer than this build.
  latest?: Release;
  /// How much of the APK has arrived while downloading, from 0 to 1.
  progress: number;
  /// Why the last check or install failed.
  error?: string;
  /// A check has answered since launch.
  checked: boolean;
}

export const useUpdates = create<Updates>()(() => ({ phase: "idle", progress: 0, checked: false }));

/// The release build on Android installs its own updates.
export const updatesSupported = Platform.OS === "android" && Application.applicationId === "app.lorca";

export const installedVersion = Application.nativeApplicationVersion ?? "0.0.0";

/// `a` is a later version than `b` (both X.Y.Z).
export function isNewer(a: string, b: string): boolean {
  const pa = a.split(".").map(Number);
  const pb = b.split(".").map(Number);
  for (let i = 0; i < 3; i++) {
    if ((pa[i] ?? 0) !== (pb[i] ?? 0)) return (pa[i] ?? 0) > (pb[i] ?? 0);
  }
  return false;
}

async function github<T>(path: string): Promise<T | null> {
  const response = await fetch(`${API}${path}`, {
    headers: { Accept: "application/vnd.github+json", "X-GitHub-Api-Version": "2022-11-28" },
  });
  if (response.status === 404) return null;
  if (!response.ok) throw new Error(t("GitHub answered {status}", { status: response.status }));
  return (await response.json()) as T;
}

interface GitHubRelease {
  draft: boolean;
  prerelease: boolean;
  body: string | null;
  assets: { name: string; browser_download_url: string; size: number; digest: string | null }[];
}

/// The newest published release newer than this build, or null. A tag whose release is still a
/// draft answers 404, since drafts are hidden, and the one before it is asked.
export async function newestRelease(): Promise<Release | null> {
  const refs = (await github<{ ref: string }[]>(`/git/matching-refs/tags/${TAG_PREFIX}`)) ?? [];
  const versions = refs
    .map((r) => r.ref.replace(`refs/tags/${TAG_PREFIX}`, ""))
    .filter((v) => /^\d+\.\d+\.\d+$/.test(v) && isNewer(v, installedVersion))
    .sort((a, b) => (isNewer(a, b) ? -1 : 1));
  for (const version of versions) {
    const release = await github<GitHubRelease>(`/releases/tags/${TAG_PREFIX}${version}`);
    if (!release || release.draft || release.prerelease) continue;
    const apk = release.assets.find((a) => a.name === `lorca-${version}-android.apk`);
    const sha256 = apk?.digest?.match(/^sha256:([0-9a-f]{64})$/)?.[1];
    if (!apk || !sha256) continue;
    return { version, notes: (release.body ?? "").trim(), url: apk.browser_download_url, sha256, size: apk.size };
  }
  return null;
}

let checking: Promise<void> | null = null;

/// Asks GitHub for a newer release. A check of the user's own (Settings) reports what it found
/// there; a daily one offers a release in an alert unless the user skipped that version.
export function checkForUpdates(daily = false): Promise<void> {
  if (!updatesSupported) return Promise.resolve();
  if (checking) return checking;
  const { phase } = useUpdates.getState();
  if (phase === "downloading" || phase === "installing") return Promise.resolve();
  useUpdates.setState({ phase: "checking", error: undefined });
  checking = (async () => {
    try {
      const latest = (await newestRelease()) ?? undefined;
      savePrefs({ ...loadPrefs(), update_checked_at: Date.now() });
      useUpdates.setState({ phase: "idle", latest, checked: true });
      if (latest && daily && loadPrefs().update_skipped !== latest.version) offerUpdate(latest);
    } catch (error) {
      useUpdates.setState({ phase: "idle", error: error instanceof Error ? error.message : String(error), checked: true });
    } finally {
      checking = null;
    }
  })();
  return checking;
}

function checkIfDue() {
  const prefs = loadPrefs();
  if (prefs.update_checks === false) return;
  if (Date.now() - (prefs.update_checked_at ?? 0) < DAY) return;
  void checkForUpdates(true);
}

/// Sparkle's offer: what is new, then install it, ask again tomorrow, or never for this version.
export function offerUpdate(release: Release) {
  const notes = release.notes.length > 1200 ? `${release.notes.slice(0, 1200).trimEnd()}…` : release.notes;
  alert(
    t("Lorca {version} is available", { version: release.version }),
    [t("You have {version}.", { version: installedVersion }), notes].filter(Boolean).join("\n\n"),
    [
      { text: t("Skip This Version"), onPress: () => savePrefs({ ...loadPrefs(), update_skipped: release.version }) },
      { text: t("Remind Me Later"), style: "cancel" },
      { text: t("Install Update"), onPress: () => void installUpdate(release) },
    ],
  );
}

/// Downloads the release and hands it to the package installer. When that installs it, the
/// system stops this app; a confirmation the user declines leaves the update on offer.
export async function installUpdate(release: Release) {
  useUpdates.setState({ phase: "downloading", progress: 0, error: undefined });
  try {
    await core.installUpdate(release.url, release.sha256, release.size);
  } catch (error) {
    useUpdates.setState({ phase: "idle", error: error instanceof Error ? error.message : String(error) });
  }
}

let started = false;

/// Once, at launch: follow the installer, and check now and on each return to the foreground
/// when a day has passed since the last check.
export function startUpdateChecks() {
  if (!updatesSupported || started) return;
  started = true;
  core.onInstallStatus((status) => {
    switch (status.state) {
      case "downloading":
        useUpdates.setState({ phase: "downloading", progress: status.total > 0 ? status.received / status.total : 0 });
        break;
      case "installing":
      case "confirming":
        useUpdates.setState({ phase: "installing" });
        break;
      case "cancelled":
        useUpdates.setState({ phase: "idle" });
        break;
      case "failed":
        useUpdates.setState({ phase: "idle", error: status.message });
        break;
    }
  });
  checkIfDue();
  AppState.addEventListener("change", (state) => {
    if (state === "active") checkIfDue();
  });
}

/// Settings' switch for the daily check.
export function setDailyChecks(on: boolean) {
  savePrefs({ ...loadPrefs(), update_checks: on });
  if (on) checkIfDue();
}
