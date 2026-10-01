/// <reference types="node" />

/// The Windows and Linux app's newest release, which the build reads once and the download page
/// links. Its releases are this repository's, tagged desktop-vX.Y.Z; the latest release is the
/// CLI's, so the build reads the list, as the app's updater and install.sh do.
const RELEASES = 'https://api.github.com/repos/egoist/lorca/releases?per_page=100'
const TAG = 'desktop-v'

export type DesktopRelease = {
  version: string
  /// The installer, for x64.
  windows: string
  /// Installs the newest release for the user in ~/.local, where the app updates itself.
  installScript: string
  /// The Debian packages, which install in /opt and update only with the next package.
  deb: { amd64: string; arm64: string }
}

type GitHubRelease = {
  tag_name: string
  draft: boolean
  prerelease: boolean
  assets: { name: string; browser_download_url: string }[]
}

const numeric = new Intl.Collator('en', { numeric: true })

/// The newest published desktop release in a page of the GitHub releases API that holds every file
/// the page links. A release is published once every platform has uploaded, so one missing a file
/// is skipped for the one before it.
export function parseDesktopReleases(releases: GitHubRelease[]): DesktopRelease | null {
  return (
    releases
      .filter((release) => release.tag_name.startsWith(TAG) && !release.draft && !release.prerelease)
      .sort((a, b) => numeric.compare(b.tag_name, a.tag_name))
      .flatMap((release) => {
        const file = (test: (name: string) => boolean) =>
          release.assets.find(({ name }) => test(name))?.browser_download_url
        const windows = file((name) => name.endsWith('.exe'))
        const installScript = file((name) => name === 'install.sh')
        const amd64 = file((name) => name.endsWith('_amd64.deb'))
        const arm64 = file((name) => name.endsWith('_arm64.deb'))
        if (!windows || !installScript || !amd64 || !arm64) return []
        return [{ version: release.tag_name.slice(TAG.length), windows, installScript, deb: { amd64, arm64 } }]
      })[0] ?? null
  )
}

/// Reads the newest desktop release for the build, with `GITHUB_TOKEN` or `GH_TOKEN` when one is
/// set (the API allows 60 requests an hour from an address without one). A release build stops
/// when it can't, rather than ship a page with no Windows or Linux download; the dev server warns
/// and shows the buttons disabled.
export async function fetchDesktopRelease({ required }: { required: boolean }): Promise<DesktopRelease | null> {
  const token = process.env.GITHUB_TOKEN || process.env.GH_TOKEN
  try {
    const response = await fetch(RELEASES, {
      headers: {
        accept: 'application/vnd.github+json',
        'user-agent': 'lorca.app',
        ...(token && { authorization: `Bearer ${token}` }),
      },
      signal: AbortSignal.timeout(10_000),
    })
    if (!response.ok) throw new Error(`HTTP ${response.status}`)
    const release = parseDesktopReleases(await response.json())
    if (!release) throw new Error(`no published ${TAG} release with a Windows installer, install.sh, and both Debian packages`)
    return release
  } catch (error) {
    const message = `reading ${RELEASES}: ${error instanceof Error ? error.message : error}`
    if (required) throw new Error(message)
    console.warn(message)
    return null
  }
}
