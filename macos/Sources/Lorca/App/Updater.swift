import AppKit
import Sparkle

/// The app's updater: a thin front for Sparkle, which downloads, checks the EdDSA signature,
/// and swaps the bundle. The feed URL and the public key are in Info.plist (`SUFeedURL`,
/// `SUPublicEDKey`), written by scripts/app.ts. docs/releasing-mac.md covers publishing.
@MainActor
final class Updater {
    static let shared = Updater()

    /// Posted on the main thread when a check finishes, however it ended.
    nonisolated static let didFinishCheck = Notification.Name("lorca.updater.didFinishCheck")

    /// A debug build is the dev loop's bundle, rebuilt in place. The menu item and the settings
    /// rows leave themselves out there.
    nonisolated static let isEnabled: Bool = {
        #if DEBUG
            return false
        #else
            return true
        #endif
    }()

    nonisolated static var currentVersion: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "—"
    }

    private var controller: SPUStandardUpdaterController?
    private let delegate = UpdaterDelegate()
    private var updater: SPUUpdater? { controller?.updater }

    private init() {}

    /// Called once, from `applicationDidFinishLaunching`.
    func start() {
        guard Self.isEnabled, controller == nil else { return }
        controller = SPUStandardUpdaterController(
            startingUpdater: true, updaterDelegate: delegate, userDriverDelegate: nil)
        // Starting the updater arms the scheduled check, which waits out its interval (a day)
        // from the last one, so a relaunch alone checks nothing.
        if automaticallyChecksForUpdates {
            updater?.checkForUpdatesInBackground()
        }
    }

    /// False while a check is in flight; the menu item grays out.
    var canCheckForUpdates: Bool { updater?.canCheckForUpdates ?? false }

    /// The user's check: Sparkle's progress window, then the update or "You're up to date".
    func checkForUpdates() {
        controller?.checkForUpdates(nil)
    }

    var lastCheckDescription: String {
        guard let date = updater?.lastUpdateCheckDate else { return "Never checked" }
        let formatter = DateFormatter()
        formatter.dateStyle = .medium
        formatter.timeStyle = .short
        formatter.doesRelativeDateFormatting = true
        return "Last checked \(formatter.string(from: date))"
    }

    // Sparkle keeps these in UserDefaults; the switches read and write through.

    var automaticallyChecksForUpdates: Bool {
        get { updater?.automaticallyChecksForUpdates ?? false }
        set { updater?.automaticallyChecksForUpdates = newValue }
    }

    var automaticallyDownloadsUpdates: Bool {
        get { updater?.automaticallyDownloadsUpdates ?? false }
        set { updater?.automaticallyDownloadsUpdates = newValue }
    }
}

/// Sparkle calls its delegate off the main actor.
private final class UpdaterDelegate: NSObject, SPUUpdaterDelegate {
    func updater(_ updater: SPUUpdater, didFinishUpdateCycleFor updateCheck: SPUUpdateCheck, error: (any Error)?) {
        DispatchQueue.main.async {
            NotificationCenter.default.post(name: Updater.didFinishCheck, object: nil)
        }
    }
}
