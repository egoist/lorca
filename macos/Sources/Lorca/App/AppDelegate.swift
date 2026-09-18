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
        installSignalHandlers()

        // No window until the CLI answers `hello`: a Device with an identity gets the main
        // window, one without gets onboarding. If the CLI stays silent, the main window shows
        // its offline state after a grace period instead of flashing before onboarding.
        store.observe(self) { [weak self] event in
            if case .identityChanged = event { self?.identityStateChanged() }
        }
        Notifier.shared.visibleChat = { [weak self] in
            guard let controller = self?.mainWindowController, let window = controller.window, window.isVisible, !window.isMiniaturized,
                case let .chat(id) = controller.root.selection
            else { return nil }
            return id
        }
        Notifier.shared.openChat = { [weak self] id in
            self?.showMainWindow()
            self?.mainWindowController?.root.select(.chat(id))
        }
        Notifier.shared.start()
        store.start()
        Task { @MainActor [weak self] in
            try? await Task.sleep(nanoseconds: 2_500_000_000)
            guard let self, self.store.hasIdentity == nil, self.onboardingWindowController == nil else { return }
            self.showMainWindow()
        }
    }

    /// Activation happens when a window exists to bring forward. Activating at launch, before
    /// the CLI has answered, leaves the window that appears later behind other apps.
    private func activate() {
        NSApp.activate(ignoringOtherApps: true)
    }

    /// Onboarding closes itself through `onFinish`, so the phrase step is never yanked away by
    /// the `identity.changed` event that precedes the create response.
    private func identityStateChanged() {
        switch store.hasIdentity {
        case .some(true):
            if onboardingWindowController == nil { showMainWindow() }
        case .some(false):
            guard onboardingWindowController == nil else { return }
            mainWindowController?.close()
            mainWindowController = nil
            presentOnboarding()
        case .none:
            break
        }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        false
    }

    func applicationWillTerminate(_ notification: Notification) {
        store.stop()
    }

    /// A SIGTERM (the dev loop, `kill`) should stop the CLI child too, not just this process.
    private var termSource: DispatchSourceSignal?
    private func installSignalHandlers() {
        signal(SIGTERM, SIG_IGN)
        let source = DispatchSource.makeSignalSource(signal: SIGTERM, queue: .main)
        source.setEventHandler { [weak self] in
            self?.store.stop()
            exit(0)
        }
        source.resume()
        termSource = source
    }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        if !flag, store.hasIdentity != false { showMainWindow() }
        return true
    }

    // MARK: - Windows

    private func showMainWindow() {
        if mainWindowController == nil {
            mainWindowController = MainWindowController()
        }
        mainWindowController?.showWindow(nil)
        mainWindowController?.window?.makeKeyAndOrderFront(nil)
        activate()
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
        activate()
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

    @objc func pairDevice(_ sender: Any?) {
        showMainWindow()
        mainWindowController?.root.presentPairing()
    }

    @objc func find(_ sender: Any?) {
        showMainWindow()
        mainWindowController?.focusSearch()
    }

    @objc func toggleCLIConnection(_ sender: Any?) {
        if store.isMock {
            store.setConnected(!store.isConnected)
        } else {
            store.reconnect()
        }
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
            title: "Lorca runs on Devices you own",
            body: """
                Every bot is assigned to a Runner: a Device running macOS, Linux, or Windows. That machine's CLI \
                runs the turn with that machine's provider credentials, so a bot on an offline Runner waits \
                until it reconnects. Phones and tablets pair as Devices but never run bots.

                The app talks only to the local CLI on 127.0.0.1:\(Preferences.cliPort). Start it with \
                `lorca serve`; the CLI holds your keys and provider credentials.
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
            menuItem.title = store.isMock ? "Simulate CLI Offline" : "Reconnect to CLI"
            menuItem.state = store.isMock && !store.isConnected ? .on : .off
        }
        if menuItem.tag == MenuTag.replayMock {
            return store.isMock
        }
        return true
    }
}
