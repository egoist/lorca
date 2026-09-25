import AppKit

/// A pane that shows one Device, picked from the pop-up in the window's toolbar; the pick holds
/// across these panes. Bots and plugins live on a Runner, so a Device that never runs bots gets
/// a note instead.
class DevicePaneViewController: SettingsPaneViewController {
    let store = AppStore.shared
    private(set) var deviceID: Device.ID?

    var device: Device? { deviceID.flatMap { store.device($0) } }

    override func viewDidLoad() {
        super.viewDidLoad()
        store.observe(self) { [weak self] event in
            switch event {
            case .snapshotReplaced, .chatsChanged, .rosterChanged: self?.reload()
            default: break
            }
        }
        reload()
    }

    func show(deviceID newID: Device.ID?) {
        guard deviceID != newID else { return }
        deviceID = newID
        guard isViewLoaded else { return }
        reload()
        scrollToTop()
    }

    func reload() {}

    /// The one row a Runner's section shows for a Device that is not one, or before the CLI answers.
    func placeholderRows(for device: Device?) -> [NSView]? {
        guard let device else { return [KeyValueRow(key: L("Waiting for the CLI"), value: "")] }
        guard !device.isRunner else { return nil }
        return [
            NoteRow(
                text: L("%@ Devices hold your keys and chats but never run a bot. Pick a Runner: a Device running macOS, Linux, or Windows.", device.os.displayName))
        ]
    }
}

// MARK: - Bots

final class BotsSettingsViewController: DevicePaneViewController {
    private let section = SectionView(title: L("Bots"))
    var onOpenChat: ((Chat.ID) -> Void)?

    override func viewDidLoad() {
        title = L("Bots")
        addSection(section)
        addFootnote(L("A bot runs on the Runner it is assigned to, with your account's credentials and that Runner's plugins."))
        super.viewDidLoad()
    }

    override func reload() {
        section.title = device.map { L("Bots on %@", $0.name) } ?? L("Bots")
        if let rows = placeholderRows(for: device) {
            section.setRows(rows)
            return
        }
        guard let device else { return }
        var rows: [NSView] = store.bots(on: device.id).map { bot in
            let row = BotRow()
            row.configure(
                bot: bot,
                detailText: bot.provider.rawValue,
                accessorySymbol: "bubble.left",
                tooltip: L("Open chat")
            )
            row.onAccessory = { [weak self] in self?.openChat(with: bot) }
            row.onClick = { [weak self] in self?.openChat(with: bot) }
            return row
        }
        if rows.isEmpty {
            rows.append(KeyValueRow(key: L("No bots assigned"), value: "", tint: .secondaryLabelColor))
        }
        let add = ActionRow(key: L("New"), value: "", tint: .secondaryLabelColor, actionTitle: L("New Bot…"))
        add.onAction = { NSApp.sendAction(#selector(AppDelegate.newBot(_:)), to: nil, from: nil) }
        rows.append(add)
        section.setRows(rows)
    }

    private func openChat(with bot: Bot) {
        onOpenChat?(store.dm(with: bot.id))
    }
}

// MARK: - Providers

/// The account's provider credentials: connected on any Device, used by every Runner.
final class ProvidersSettingsViewController: SettingsPaneViewController {
    private let store = AppStore.shared
    private let section = SectionView(title: L("Credentials"))

    override func viewDidLoad() {
        title = L("Providers")
        addSection(section)
        addFootnote(
            L("Credentials belong to your account. They reach your paired Devices encrypted with the account key, so a bot uses them on whichever Runner it is assigned to; the relay stores ciphertext."))
        super.viewDidLoad()
        store.observe(self) { [weak self] event in
            switch event {
            case .snapshotReplaced, .rosterChanged: self?.reload()
            default: break
            }
        }
        reload()
    }

    private func reload() {
        guard !store.providers.isEmpty else {
            section.setRows([KeyValueRow(key: L("Waiting for the CLI"), value: "")])
            return
        }
        section.setRows(
            store.providers.map { credential in
                let row = StatusRow()
                row.configure(
                    symbol: credential.kind.symbolName,
                    title: credential.kind.rawValue,
                    subtitle: "\(credential.kind.subtitle) · \(credential.detail)",
                    state: credential.isConnected ? L("Connected") : nil,
                    stateColor: .systemGreen,
                    actionTitle: credential.isConnected ? (credential.kind.usesAPIKey ? L("Edit…") : L("Disconnect")) : L("Connect…"),
                    destructive: credential.isConnected && !credential.kind.usesAPIKey
                )
                row.onAction = { [weak self] in
                    guard let self else { return }
                    if credential.isConnected && !credential.kind.usesAPIKey {
                        Task { try? await self.store.disconnectProvider(credential.kind) }
                    } else {
                        ConnectProviderViewController.present(kind: credential.kind, from: self, baseURL: credential.baseURL)
                    }
                }
                return row
            })
    }
}

// MARK: - Plugins

/// What the picked Runner has installed, with each plugin's state, and the marketplace.
final class PluginsSettingsViewController: DevicePaneViewController {
    private let section = SectionView(title: L("Plugins"))
    var onOpenMarketplace: ((Device.ID) -> Void)?

    override func viewDidLoad() {
        title = L("Plugins")
        addSection(section)
        addFootnote(L("Plugins are installed on a Runner, and the bots assigned to it use them. An action that changes something goes through Auto-review first."))
        super.viewDidLoad()
    }

    override func reload() {
        section.title = device.map { L("Plugins on %@", $0.name) } ?? L("Plugins")
        if let rows = placeholderRows(for: device) {
            section.setRows(rows)
            return
        }
        guard let device else { return }
        var rows: [NSView] = device.plugins.map { plugin in
            let row = StatusRow()
            row.configure(plugin: plugin)
            row.identifier = NSUserInterfaceItemIdentifier(plugin.id)
            row.addGestureRecognizer(NSClickGestureRecognizer(target: self, action: #selector(openPlugin(_:))))
            return row
        }
        if rows.isEmpty {
            rows.append(KeyValueRow(key: L("No plugins installed"), value: "", tint: .secondaryLabelColor))
        }
        let add = ActionRow(key: L("Marketplace"), value: "", tint: .secondaryLabelColor, actionTitle: L("Add from Plugins…"))
        add.onAction = { [weak self] in self?.onOpenMarketplace?(device.id) }
        rows.append(add)
        section.setRows(rows)
    }

    @objc private func openPlugin(_ sender: NSClickGestureRecognizer) {
        guard let id = sender.view?.identifier?.rawValue, let device else { return }
        presentAsSheet(PluginViewController(pluginID: id, runner: device, bot: nil))
    }
}

// MARK: - Devices

/// The picked Device itself: what it is, whether it is online, its machine key, and Unpair.
final class AboutDeviceSettingsViewController: DevicePaneViewController {
    private let header = DeviceHeaderView()
    private let machineSection = SectionView(title: L("Machine"))

    override func viewDidLoad() {
        title = L("Devices")
        add(header)
        addSection(machineSection)
        super.viewDidLoad()
    }

    override func reload() {
        guard let device else { return }
        header.configure(device: device)

        var rows: [NSView] = [
            KeyValueRow(key: SettingsEntry.machineKey.row, value: device.machineKey, monospaced: true),
            KeyValueRow(key: L("OS"), value: "\(device.os.rawValue) · \(device.osVersion)"),
            KeyValueRow(
                key: L("Role"),
                value: device.isRunner ? L("Runner · runs bots with its own credentials") : L("Device · never runs bots")),
            KeyValueRow(
                key: L("Last seen"),
                value: device.status == .online ? L("Active now") : Format.lastSeen(device.lastSeen)),
            KeyValueRow(
                key: L("Relay"),
                value: store.relayURL.map { url in
                    if store.relayUpdateRequired { return L("%@ · update Lorca to sync", url) }
                    if store.relayConnected { return url }
                    // Why the last try to connect failed, in the CLI's words.
                    return store.relayError.map { "\(url) · \($0)" } ?? L("%@ · offline", url)
                } ?? L("Not configured"),
                monospaced: true),
        ]
        if !device.isThisDevice {
            let unpair = ActionRow(
                key: SettingsEntry.pairing.row, value: L("Paired to this account"), tint: .secondaryLabelColor,
                actionTitle: L("Unpair…"))
            unpair.onAction = { [weak self] in UnpairDevice.confirm(device, in: self?.view.window) }
            rows.append(unpair)
        }
        machineSection.setRows(rows)
    }
}

final class DeviceHeaderView: NSView {
    private let icon = NSImageView()
    private let name = Build.label("", font: .systemFont(ofSize: 22, weight: .semibold))
    private let model = Build.label("", font: .systemFont(ofSize: 12.5), color: .secondaryLabelColor)
    private let dot = StatusDotView(size: 8)
    private let status = Build.label("", font: .systemFont(ofSize: 11.5, weight: .medium))

    init() {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        icon.translatesAutoresizingMaskIntoConstraints = false
        icon.contentTintColor = .secondaryLabelColor

        let statusLine = Build.stack([dot, status], orientation: .horizontal, spacing: 6)
        let text = Build.stack([name, model, statusLine], spacing: 3)
        text.setCustomSpacing(8, after: model)

        addSubview(icon)
        addSubview(text)

        NSLayoutConstraint.activate([
            icon.leadingAnchor.constraint(equalTo: leadingAnchor),
            icon.topAnchor.constraint(equalTo: topAnchor, constant: 4),
            icon.widthAnchor.constraint(equalToConstant: 44),
            icon.heightAnchor.constraint(equalToConstant: 44),
            text.leadingAnchor.constraint(equalTo: icon.trailingAnchor, constant: 14),
            text.topAnchor.constraint(equalTo: topAnchor),
            text.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor),
            text.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func configure(device: Device) {
        icon.image = NSImage(systemSymbolName: device.symbolName, accessibilityDescription: nil)
        icon.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 34, weight: .regular)
        name.stringValue = device.name
        model.stringValue = "\(device.model) · \(device.osVersion)"
        dot.status = device.status

        switch device.status {
        case .online:
            status.stringValue =
                if device.isThisDevice {
                    L("This computer · CLI running")
                } else if device.isRunner {
                    L("Online · paired")
                } else {
                    L("Online · paired · not a Runner")
                }
            status.textColor = .systemGreen
        case .pairing:
            status.stringValue = L("Pairing…")
            status.textColor = .systemOrange
        case .offline:
            status.stringValue =
                Format.lastSeen(device.lastSeen) + (device.isRunner ? L(" · jobs wait on the relay") : "")
            status.textColor = .secondaryLabelColor
        }
    }
}
