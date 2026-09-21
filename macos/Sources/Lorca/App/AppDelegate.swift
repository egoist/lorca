import AppKit

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private var mainWindowController: MainWindowController?
    private var saidUpdateRequired = false
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
            if case .rosterChanged = event { self?.relayStateChanged() }
        }
        NotificationCenter.default.addObserver(forName: AppLanguage.didChange, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.languageChanged() }
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
        Updater.shared.start()
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

    /// Views take their words when they are built, so a new language builds them again: the
    /// menu bar, then the window in the same place on the same selection (Settings › General,
    /// where the language is picked), or the small settings window while onboarding is up.
    private func languageChanged() {
        NSApp.mainMenu = MainMenu.build()
        if let old = mainWindowController {
            let selection = old.root.selection
            let wasVisible = old.window?.isVisible == true
            old.window?.saveFrame(usingName: "LorcaMainWindow")
            old.close()
            let controller = MainWindowController()
            mainWindowController = controller
            if wasVisible {
                controller.showWindow(nil)
                controller.window?.makeKeyAndOrderFront(nil)
            }
            controller.root.select(selection)
        }
        if let old = settingsWindowController {
            let frame = old.window?.frame
            old.close()
            let controller = SettingsWindowController()
            settingsWindowController = controller
            controller.showWindow(nil)
            if let frame { controller.window?.setFrame(frame, display: true) }
            controller.window?.makeKeyAndOrderFront(nil)
        }
    }

    private func presentOnboarding() {
        let controller = OnboardingWindowController { [weak self] in
            Preferences.hasOnboarded = true
            self?.onboardingWindowController?.close()
            self?.onboardingWindowController = nil
            self?.settingsWindowController?.close()
            self?.settingsWindowController = nil
            self?.showMainWindow()
        }
        onboardingWindowController = controller
        controller.showWindow(nil)
        controller.window?.center()
        controller.window?.makeKeyAndOrderFront(nil)
        activate()
    }

    // MARK: - Actions

    /// Settings is a mode of the main window. While onboarding is up there is no main window,
    /// so the relay URL and the CLI port get a small window of their own.
    @objc func showSettings(_ sender: Any?) {
        guard onboardingWindowController == nil else {
            if settingsWindowController == nil {
                settingsWindowController = SettingsWindowController()
            }
            settingsWindowController?.showWindow(nil)
            settingsWindowController?.window?.makeKeyAndOrderFront(nil)
            return
        }
        showMainWindow()
        mainWindowController?.root.showSettings()
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

    @objc func toggleCommandPalette(_ sender: Any?) {
        showMainWindow()
        mainWindowController?.toggleCommandPalette()
    }

    /// The relay turned this build away. Said once per launch, since every sync attempt gets
    /// the same answer until Lorca is updated.
    private func relayStateChanged() {
        guard store.relayUpdateRequired, !saidUpdateRequired, let window = mainWindowController?.window, window.isVisible else { return }
        saidUpdateRequired = true
        let alert = NSAlert()
        alert.messageText = L("Update Lorca to keep syncing")
        alert.informativeText = L("The relay no longer works with this version. Chats on this Mac stay as they are, and nothing syncs with your other Devices until you update.")
        if Updater.isEnabled {
            alert.addButton(withTitle: L("Check for Updates…"))
        }
        alert.addButton(withTitle: Updater.isEnabled ? L("Later") : L("OK"))
        alert.beginSheetModal(for: window) { response in
            if Updater.isEnabled, response == .alertFirstButtonReturn {
                Updater.shared.checkForUpdates()
            }
        }
    }

    @objc func checkForUpdates(_ sender: Any?) {
        Updater.shared.checkForUpdates()
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
        guard onboardingWindowController == nil else { return }
        Preferences.hasOnboarded = false
        mainWindowController?.close()
        mainWindowController = nil
        presentOnboarding()
    }

    @objc func showHelp(_ sender: Any?) {
        presentNote(
            title: L("Lorca runs on Devices you own"),
            body: L(
                "Every bot is assigned to a Runner: a Device running macOS, Linux, or Windows. That machine's CLI runs the turn with your account's provider credentials, so a bot on an offline Runner waits until it reconnects. Phones and tablets pair as Devices but never run bots.\n\nThe app talks only to the local CLI on 127.0.0.1:%@. Start it with `lorca serve`; the CLI holds your keys and provider credentials, which reach your other Devices encrypted.",
                String(Preferences.cliPort))
                .replacingOccurrences(of: "lorca serve", with: AppInfo.cliCommand)
        )
    }

    @objc func showArchitecture(_ sender: Any?) {
        presentNote(
            title: L("Three processes"),
            body: L(
                "The app talks only to the local CLI over a localhost websocket. The CLI holds the keys, runs the agent loop, and syncs ciphertext with the relay. The relay stores public keys and opaque blobs.\n\nFull notes live in ARCHITECTURE.md.")
        )
    }

    private func presentNote(title: String, body: String) {
        let alert = NSAlert()
        alert.messageText = title
        alert.informativeText = body
        alert.alertStyle = .informational
        alert.addButton(withTitle: L("OK"))
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
        if menuItem.action == #selector(toggleCommandPalette(_:)) {
            return onboardingWindowController == nil
        }
        if menuItem.action == #selector(checkForUpdates(_:)) {
            return Updater.shared.canCheckForUpdates
        }
        return true
    }
}
