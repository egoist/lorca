import AppKit

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private var mainWindowController: MainWindowController?
    private var onboardingWindowController: OnboardingWindowController?
    private var settingsWindowController: SettingsWindowController?

    private let store = AppStore.shared

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.appearance = nil
        NSApp.applicationIconImage = AppIcon.make()
        NSApp.mainMenu = MainMenu.build()

        if Preferences.hasOnboarded {
            showMainWindow()
        } else {
            presentOnboarding()
        }

        NSApp.activate(ignoringOtherApps: true)
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        false
    }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        if !flag, Preferences.hasOnboarded { showMainWindow() }
        return true
    }

    // MARK: - Windows

    private func showMainWindow() {
        if mainWindowController == nil {
            mainWindowController = MainWindowController()
        }
        mainWindowController?.showWindow(nil)
        mainWindowController?.window?.makeKeyAndOrderFront(nil)
    }

    private func presentOnboarding() {
        let controller = OnboardingWindowController { [weak self] in
            Preferences.hasOnboarded = true
            self?.onboardingWindowController?.close()
            self?.onboardingWindowController = nil
            self?.showMainWindow()
        }
        onboardingWindowController = controller
        controller.showWindow(nil)
        controller.window?.center()
        controller.window?.makeKeyAndOrderFront(nil)
    }

    // MARK: - Actions

    @objc func showSettings(_ sender: Any?) {
        if settingsWindowController == nil {
            settingsWindowController = SettingsWindowController()
        }
        settingsWindowController?.showWindow(nil)
        settingsWindowController?.window?.makeKeyAndOrderFront(nil)
    }

    @objc func newGroupChat(_ sender: Any?) {
        showMainWindow()
        mainWindowController?.root.presentNewGroupChat()
    }

    @objc func newBot(_ sender: Any?) {
        showMainWindow()
        mainWindowController?.root.presentNewBot()
    }

    @objc func pairComputer(_ sender: Any?) {
        showMainWindow()
        mainWindowController?.root.presentPairing()
    }

    @objc func find(_ sender: Any?) {
        showMainWindow()
        mainWindowController?.focusSearch()
    }

    @objc func toggleCLIConnection(_ sender: Any?) {
        store.setConnected(!store.isConnected)
    }

    @objc func resetMockData(_ sender: Any?) {
        store.resetMockData()
    }

    @objc func showOnboarding(_ sender: Any?) {
        Preferences.hasOnboarded = false
        mainWindowController?.close()
        mainWindowController = nil
        presentOnboarding()
    }

    @objc func showHelp(_ sender: Any?) {
        presentNote(
            title: "Tinybot runs on Computers you own",
            body: """
                Every bot is assigned to a Computer. That machine's CLI runs the turn with that machine's \
                provider credentials, so a bot on an offline Computer waits until it reconnects.

                This build renders mock data. The chrome, events and shapes match what the local CLI \
                will send over 127.0.0.1:\(Preferences.cliPort).
                """
        )
    }

    @objc func showArchitecture(_ sender: Any?) {
        presentNote(
            title: "Three processes",
            body: """
                The app talks only to the local CLI over a localhost websocket. The CLI holds the keys, \
                runs the agent loop, and syncs ciphertext with the relay. The relay stores public keys \
                and opaque blobs.

                Full notes live in ARCHITECTURE.md.
                """
        )
    }

    private func presentNote(title: String, body: String) {
        let alert = NSAlert()
        alert.messageText = title
        alert.informativeText = body
        alert.alertStyle = .informational
        alert.addButton(withTitle: "OK")
        if let window = NSApp.keyWindow {
            alert.beginSheetModal(for: window)
        } else {
            alert.runModal()
        }
    }

    // MARK: - Menu state

    func validateMenuItem(_ menuItem: NSMenuItem) -> Bool {
        if menuItem.tag == MenuTag.simulateOffline {
            menuItem.state = store.isConnected ? .off : .on
        }
        return true
    }
}
