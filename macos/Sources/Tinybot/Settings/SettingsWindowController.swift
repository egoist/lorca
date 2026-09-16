import AppKit

final class SettingsWindowController: NSWindowController {
    init() {
        let tabController = NSTabViewController()
        tabController.tabStyle = .toolbar

        let tabs: [(NSViewController, String, String)] = [
            (GeneralSettingsViewController(), "General", "gearshape"),
            (DevicesSettingsViewController(), "Devices", "laptopcomputer"),
            (ProvidersSettingsViewController(), "Providers", "key"),
            (AdvancedSettingsViewController(), "Advanced", "slider.horizontal.3"),
        ]

        for (controller, label, symbol) in tabs {
            let item = NSTabViewItem(viewController: controller)
            item.label = label
            item.image = NSImage(systemSymbolName: symbol, accessibilityDescription: label)
            tabController.addTabViewItem(item)
        }

        let window = NSWindow(contentViewController: tabController)
        window.title = "Settings"
        window.styleMask.insert(.closable)
        window.styleMask.remove(.resizable)
        window.setContentSize(NSSize(width: 520, height: 380))
        window.center()
        super.init(window: window)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }
}

// MARK: - Base

class SettingsPaneViewController: NSViewController {
    let column = Build.stack([], spacing: 18)

    override func loadView() {
        let container = NSView()
        container.translatesAutoresizingMaskIntoConstraints = false
        column.orientation = .vertical
        column.alignment = .leading
        container.addSubview(column)

        NSLayoutConstraint.activate([
            container.widthAnchor.constraint(equalToConstant: 520),
            column.topAnchor.constraint(equalTo: container.topAnchor, constant: 24),
            column.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 24),
            column.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -24),
            column.bottomAnchor.constraint(
                lessThanOrEqualTo: container.bottomAnchor, constant: -24),
            container.heightAnchor.constraint(greaterThanOrEqualToConstant: 340),
        ])
        view = container
    }

    func addSection(_ section: SectionView) {
        column.addArrangedSubview(section)
        section.widthAnchor.constraint(equalTo: column.widthAnchor).isActive = true
    }

    func addFootnote(_ text: String) {
        let label = Build.label(text, font: Theme.Font.caption, color: .tertiaryLabelColor, lines: 0)
        column.addArrangedSubview(label)
        label.widthAnchor.constraint(equalTo: column.widthAnchor).isActive = true
    }
}

// MARK: - General

final class GeneralSettingsViewController: SettingsPaneViewController {
    private let appearance = NSPopUpButton()
    private let dictationLanguage = NSPopUpButton()

    override func viewDidLoad() {
        super.viewDidLoad()
        title = "General"

        let sendOnReturn = checkbox(
            "Return sends the message", state: Preferences.sendOnReturn,
            action: #selector(toggleSendOnReturn))
        let timestamps = checkbox(
            "Show timestamps in transcripts", state: Preferences.showTimestamps,
            action: #selector(toggleTimestamps))

        appearance.addItems(withTitles: ["System", "Light", "Dark"])
        appearance.target = self
        appearance.action = #selector(changeAppearance)
        appearance.translatesAutoresizingMaskIntoConstraints = false

        let appearanceRow = NSView()
        appearanceRow.translatesAutoresizingMaskIntoConstraints = false
        let label = Build.label("Appearance", font: .systemFont(ofSize: 12), color: .secondaryLabelColor)
        appearanceRow.addSubview(label)
        appearanceRow.addSubview(appearance)
        NSLayoutConstraint.activate([
            label.leadingAnchor.constraint(equalTo: appearanceRow.leadingAnchor),
            label.centerYAnchor.constraint(equalTo: appearance.centerYAnchor),
            appearance.leadingAnchor.constraint(equalTo: label.trailingAnchor, constant: 12),
            appearance.topAnchor.constraint(equalTo: appearanceRow.topAnchor),
            appearance.bottomAnchor.constraint(equalTo: appearanceRow.bottomAnchor),
            appearance.widthAnchor.constraint(equalToConstant: 140),
        ])

        // The language the Dictate button listens in. Automatic follows the keyboard input
        // source, then the system languages.
        dictationLanguage.addItem(withTitle: "Automatic (\(Dictation.displayName(Dictation.automaticLocale())))")
        dictationLanguage.menu?.addItem(.separator())
        for locale in Dictation.supportedLocales {
            dictationLanguage.addItem(withTitle: Dictation.displayName(locale))
            dictationLanguage.lastItem?.representedObject = locale.identifier
        }
        if let chosen = Preferences.dictationLanguage,
            let item = dictationLanguage.itemArray.first(where: { $0.representedObject as? String == chosen })
        {
            dictationLanguage.select(item)
        }
        dictationLanguage.target = self
        dictationLanguage.action = #selector(changeDictationLanguage)
        dictationLanguage.translatesAutoresizingMaskIntoConstraints = false
        let dictationRow = NSView()
        dictationRow.translatesAutoresizingMaskIntoConstraints = false
        let dictationLabel = Build.label("Dictation", font: .systemFont(ofSize: 12), color: .secondaryLabelColor)
        dictationRow.addSubview(dictationLabel)
        dictationRow.addSubview(dictationLanguage)
        NSLayoutConstraint.activate([
            dictationLabel.leadingAnchor.constraint(equalTo: dictationRow.leadingAnchor),
            dictationLabel.centerYAnchor.constraint(equalTo: dictationLanguage.centerYAnchor),
            dictationLanguage.leadingAnchor.constraint(equalTo: dictationLabel.trailingAnchor, constant: 12),
            dictationLanguage.topAnchor.constraint(equalTo: dictationRow.topAnchor),
            dictationLanguage.bottomAnchor.constraint(equalTo: dictationRow.bottomAnchor),
            dictationLanguage.widthAnchor.constraint(equalToConstant: 220),
        ])

        column.addArrangedSubview(sendOnReturn)
        column.addArrangedSubview(timestamps)
        column.addArrangedSubview(appearanceRow)
        column.addArrangedSubview(dictationRow)
        column.setCustomSpacing(8, after: sendOnReturn)
        addFootnote(
            "Tinybot talks only to the CLI on this Mac. Nothing here is synced; each Device keeps its own settings."
        )
    }

    private func checkbox(_ title: String, state: Bool, action: Selector) -> NSButton {
        let button = NSButton(checkboxWithTitle: title, target: self, action: action)
        button.state = state ? .on : .off
        button.translatesAutoresizingMaskIntoConstraints = false
        return button
    }

    @objc private func toggleSendOnReturn(_ sender: NSButton) {
        Preferences.sendOnReturn = sender.state == .on
    }

    @objc private func toggleTimestamps(_ sender: NSButton) {
        Preferences.showTimestamps = sender.state == .on
    }

    @objc private func changeDictationLanguage() {
        Preferences.dictationLanguage = dictationLanguage.selectedItem?.representedObject as? String
    }

    @objc private func changeAppearance() {
        switch appearance.indexOfSelectedItem {
        case 1: NSApp.appearance = NSAppearance(named: .aqua)
        case 2: NSApp.appearance = NSAppearance(named: .darkAqua)
        default: NSApp.appearance = nil
        }
    }
}

// MARK: - Devices

final class DevicesSettingsViewController: SettingsPaneViewController {
    private let section = SectionView(title: "Paired Devices")

    override func viewDidLoad() {
        super.viewDidLoad()
        title = "Devices"

        reload()
        addSection(section)

        let pair = NSButton(
            title: "Pair a Device…", target: NSApp.delegate,
            action: #selector(AppDelegate.pairDevice(_:)))
        pair.bezelStyle = .rounded
        column.addArrangedSubview(pair)

        addFootnote(
            "Pairing wraps the account key to the other machine's public key. The relay stores only public keys and ciphertext."
        )

        AppStore.shared.observe(self) { [weak self] event in
            switch event {
            case .rosterChanged, .snapshotReplaced: self?.reload()
            default: break
            }
        }
    }

    private func reload() {
        let store = AppStore.shared
        section.setRows(
            store.devices.map { device in
                let row = StatusRow()
                row.configure(
                    symbol: device.symbolName,
                    title: device.isThisDevice ? "\(device.name) (this Mac)" : device.name,
                    subtitle: device.isRunner
                        ? "\(device.model) · \(store.bots(on: device.id).count) bots"
                        : "\(device.model) · \(device.os.displayName) · not a Runner",
                    state: device.status == .online ? "Online" : Format.lastSeen(device.lastSeen),
                    stateColor: device.status == .online ? .systemGreen : .secondaryLabelColor
                )
                return row
            })
    }
}

// MARK: - Providers

final class ProvidersSettingsViewController: SettingsPaneViewController {
    private let section = SectionView(title: "Credentials on this Mac")

    override func viewDidLoad() {
        super.viewDidLoad()
        title = "Providers"

        reload()
        addSection(section)
        AppStore.shared.observe(self) { [weak self] event in
            switch event {
            case .rosterChanged, .snapshotReplaced: self?.reload()
            default: break
            }
        }

        addFootnote(
            """
            Keys stay on the Runner they were entered on, in the CLI's credential file. A bot assigned to another \
            Runner uses that machine's credentials — this one never sees them.
            """
        )
    }

    private func reload() {
        let store = AppStore.shared
        guard let device = store.thisDevice else {
            section.setRows([KeyValueRow(key: "Waiting for the CLI", value: "")])
            return
        }
        section.setRows(
            device.providers.map { credential in
                let row = StatusRow()
                row.configure(
                    symbol: credential.kind.symbolName,
                    title: credential.kind.rawValue,
                    subtitle: "\(credential.kind.subtitle) · \(credential.detail)",
                    state: credential.isConnected ? "Connected" : nil,
                    stateColor: .systemGreen,
                    actionTitle: credential.isConnected ? "Disconnect" : "Connect…"
                )
                row.onAction = { [weak self] in
                    guard let self else { return }
                    if credential.isConnected {
                        Task { try? await store.disconnectProvider(credential.kind) }
                    } else {
                        self.presentAsSheet(ConnectProviderViewController(kind: credential.kind))
                    }
                }
                return row
            })
    }
}

// MARK: - Advanced

final class AdvancedSettingsViewController: SettingsPaneViewController {
    private let relayField = NSTextField()
    private let portField = NSTextField()

    override func viewDidLoad() {
        super.viewDidLoad()
        title = "Advanced"

        relayField.stringValue = AppStore.shared.relayURL ?? Preferences.relayURL
        relayField.placeholderString = "https://relay.example.com"
        relayField.delegate = self
        portField.stringValue = "\(Preferences.cliPort)"
        portField.delegate = self

        column.addArrangedSubview(field("Relay URL", relayField))
        column.addArrangedSubview(field("CLI port", portField))

        let reset = NSButton(
            title: "Show Onboarding Again", target: NSApp.delegate,
            action: #selector(AppDelegate.showOnboarding(_:)))
        reset.bezelStyle = .rounded
        column.addArrangedSubview(reset)

        addFootnote(
            "Self-hosting the relay is a URL change: clients sign their requests and upload ciphertext, so the relay has nothing to trust. Leave it empty to run on this Mac alone."
        )
    }

    private func field(_ title: String, _ control: NSTextField) -> NSView {
        let container = NSView()
        container.translatesAutoresizingMaskIntoConstraints = false
        control.translatesAutoresizingMaskIntoConstraints = false
        control.font = .monospacedSystemFont(ofSize: 12, weight: .regular)

        let label = Build.label(title, font: .systemFont(ofSize: 12), color: .secondaryLabelColor)
        container.addSubview(label)
        container.addSubview(control)

        NSLayoutConstraint.activate([
            label.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            label.centerYAnchor.constraint(equalTo: control.centerYAnchor),
            label.widthAnchor.constraint(equalToConstant: 80),
            control.leadingAnchor.constraint(equalTo: label.trailingAnchor, constant: 12),
            control.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            control.topAnchor.constraint(equalTo: container.topAnchor),
            control.bottomAnchor.constraint(equalTo: container.bottomAnchor),
        ])
        container.widthAnchor.constraint(equalTo: column.widthAnchor).isActive = true
        return container
    }
}

extension AdvancedSettingsViewController: NSTextFieldDelegate {
    func controlTextDidEndEditing(_ obj: Notification) {
        let relay = relayField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        if relay != (AppStore.shared.relayURL ?? "") {
            Preferences.relayURL = relay
            AppStore.shared.setRelayURL(relay)
        }
        if let port = Int(portField.stringValue), port > 0, port < 65536 {
            if port != Preferences.cliPort {
                Preferences.cliPort = port
                AppStore.shared.reconnect()
            }
        } else {
            portField.stringValue = "\(Preferences.cliPort)"
        }
    }
}
