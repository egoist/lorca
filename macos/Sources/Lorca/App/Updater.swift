import AppKit

/// The app's updater. This app no longer installs updates: Lorca on the Mac is the desktop app
/// now. The site's `/desktop-upgrade-url.txt` holds the newest desktop release's disk image, and
/// when it answers with one, the app says a new version is available and opens that link to
/// download it.
@MainActor
final class Updater {
    static let shared = Updater()

    /// A plain-text body holding one https URL; anything else is no update.
    nonisolated static let upgradeEndpoint = URL(string: "https://lorca.app/desktop-upgrade-url.txt")!

    /// A debug build is the dev loop's bundle, rebuilt in place. The menu item and the settings
    /// row leave themselves out there.
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

    private var checking = false
    private var presenting = false

    private init() {}

    /// Called once, from `applicationDidFinishLaunching`: a new version found then is offered.
    func start() {
        guard Self.isEnabled else { return }
        check(userInitiated: false)
    }

    /// False while a check is in flight; the menu item grays out.
    var canCheckForUpdates: Bool { !checking }

    /// The user's check: the update when there is one, else "You're up to date" or the error.
    func checkForUpdates() {
        check(userInitiated: true)
    }

    private func check(userInitiated: Bool) {
        guard !checking else { return }
        checking = true
        Task {
            let result = await Self.fetchUpgradeURL()
            checking = false
            switch result {
            case .success(let url?):
                offer(url)
            case .success(nil):
                if userInitiated {
                    inform(L("You’re up to date"), L("Lorca %@ is the newest version.", Self.currentVersion))
                }
            case .failure(let error):
                if userInitiated {
                    inform(L("Couldn’t check for updates"), error.localizedDescription)
                }
            }
        }
    }

    /// The download link, nil when the site has none (no update), or the error that kept it from
    /// answering.
    private nonisolated static func fetchUpgradeURL() async -> Result<URL?, Error> {
        var request = URLRequest(url: upgradeEndpoint, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: 15)
        request.setValue("text/plain", forHTTPHeaderField: "Accept")
        do {
            let (data, response) = try await URLSession.shared.data(for: request)
            guard let status = (response as? HTTPURLResponse)?.statusCode, status == 200 else { return .success(nil) }
            // A page in place of the file (a 404 rendered as 200) is no URL.
            let text = String(decoding: data.prefix(2048), as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
            guard !text.contains(where: \.isWhitespace), let url = URL(string: text), url.scheme == "https", url.host != nil
            else { return .success(nil) }
            return .success(url)
        } catch {
            return .failure(error)
        }
    }

    private func offer(_ url: URL) {
        guard !presenting else { return }
        let alert = NSAlert()
        alert.messageText = L("A new version of Lorca is available")
        alert.informativeText = L(
            "This version no longer updates itself. Download the new Lorca, quit this one, and drag the new one to Applications to replace it. Your bots and chats stay as they are."
        )
        alert.addButton(withTitle: L("Download"))
        alert.addButton(withTitle: L("Later"))
        present(alert) { response in
            if response == .alertFirstButtonReturn {
                NSWorkspace.shared.open(url)
            }
        }
    }

    private func inform(_ title: String, _ text: String) {
        let alert = NSAlert()
        alert.messageText = title
        alert.informativeText = text
        alert.addButton(withTitle: L("OK"))
        present(alert) { _ in }
    }

    /// A sheet on the key window when there is one, else a window of its own.
    private func present(_ alert: NSAlert, then handler: @escaping (NSApplication.ModalResponse) -> Void) {
        presenting = true
        let done: (NSApplication.ModalResponse) -> Void = { [weak self] response in
            self?.presenting = false
            handler(response)
        }
        if let window = NSApp.keyWindow ?? NSApp.mainWindow, window.isVisible, window.attachedSheet == nil {
            alert.beginSheetModal(for: window, completionHandler: done)
        } else {
            NSApp.activate(ignoringOtherApps: true)
            done(alert.runModal())
        }
    }
}
