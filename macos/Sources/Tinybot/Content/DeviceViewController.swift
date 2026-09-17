import AppKit

final class DeviceViewController: NSViewController {
    private let store = AppStore.shared

    private let scrollView = NSScrollView()
    private let column = Build.stack([], spacing: 22)
    private let header = DeviceHeaderView()
    private let botsSection = SectionView(title: "Bots assigned here")
    private let providersSection = SectionView(title: "Provider credentials")
    private let pluginsSection = SectionView(title: "Plugins")
    private let machineSection = SectionView(title: "Machine")
    private let note = Build.label(
        "", font: Theme.Font.caption, color: .tertiaryLabelColor, lines: 0)

    private var deviceID: Device.ID?

    var onOpenChat: ((Chat.ID) -> Void)?

    override func loadView() {
        let container = BackgroundView()
        container.fillColor = Theme.transcriptBackground
        container.cornerRadius = 0

        column.orientation = .vertical
        column.alignment = .leading
        column.edgeInsets = NSEdgeInsets(top: 24, left: 28, bottom: 32, right: 28)
        column.addArrangedSubview(header)
        column.addArrangedSubview(botsSection)
        column.addArrangedSubview(providersSection)
        column.addArrangedSubview(pluginsSection)
        column.addArrangedSubview(machineSection)
        column.addArrangedSubview(note)

        let documentView = NSView()
        documentView.translatesAutoresizingMaskIntoConstraints = false
        documentView.addSubview(column)

        scrollView.documentView = documentView
        scrollView.drawsBackground = false
        scrollView.hasVerticalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.translatesAutoresizingMaskIntoConstraints = false

        container.addSubview(scrollView)
        scrollView.pin(to: container)

        NSLayoutConstraint.activate([
            documentView.widthAnchor.constraint(equalTo: scrollView.widthAnchor),
            column.topAnchor.constraint(equalTo: documentView.topAnchor),
            column.leadingAnchor.constraint(equalTo: documentView.leadingAnchor),
            column.trailingAnchor.constraint(equalTo: documentView.trailingAnchor),
            column.bottomAnchor.constraint(equalTo: documentView.bottomAnchor),
        ])

        for section in [header, botsSection, providersSection, pluginsSection, machineSection, note] as [NSView] {
            section.widthAnchor.constraint(equalTo: column.widthAnchor, constant: -56).isActive = true
        }

        view = container
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        store.observe(self) { [weak self] event in
            switch event {
            case .snapshotReplaced, .chatsChanged, .rosterChanged:
                self?.reload()
            default:
                break
            }
        }
    }

    func show(deviceID newID: Device.ID) {
        deviceID = newID
        guard isViewLoaded else { return }
        reload()
        scrollView.contentView.scroll(to: .zero)
    }

    override func viewWillAppear() {
        super.viewWillAppear()
        reload()
    }

    private func reload() {
        guard let deviceID, let device = store.device(deviceID) else { return }

        header.configure(device: device)

        botsSection.isHidden = !device.isRunner
        providersSection.isHidden = !device.isRunner
        pluginsSection.isHidden = !device.isRunner
        if device.isRunner {
            pluginsSection.setRows(pluginRows(on: device))
        }

        if device.isRunner {
            let bots = store.bots(on: device.id)
            if bots.isEmpty {
                let empty = KeyValueRow(key: "No bots assigned", value: "")
                botsSection.setRows([empty])
            } else {
                botsSection.setRows(
                    bots.map { bot in
                        let row = BotRow()
                        row.configure(
                            bot: bot,
                            detailText: "\(bot.label) · \(bot.provider.rawValue)",
                            accessorySymbol: "bubble.left",
                            tooltip: "Open chat"
                        )
                        row.onAccessory = { [weak self] in self?.openChat(with: bot) }
                        row.onClick = { [weak self] in self?.openChat(with: bot) }
                        return row
                    })
            }

            providersSection.setRows(
                device.providers.map { credential in
                    let row = StatusRow()
                    row.configure(
                        symbol: credential.kind.symbolName,
                        title: credential.kind.rawValue,
                        subtitle: "\(credential.kind.subtitle) · \(credential.detail)",
                        state: credential.isConnected ? "Connected" : nil,
                        stateColor: .systemGreen,
                        actionTitle: credential.isConnected ? (device.isThisDevice ? "Disconnect" : nil) : "Connect…",
                        destructive: credential.isConnected
                    )
                    row.onAction = { [weak self] in
                        guard let self else { return }
                        if credential.isConnected {
                            Task { try? await self.store.disconnectProvider(credential.kind) }
                        } else {
                            self.connectProvider(credential.kind, on: device)
                        }
                    }
                    return row
                })
        }

        var machineRows: [NSView] = [
            KeyValueRow(key: "Machine key", value: device.machineKey, monospaced: true),
            KeyValueRow(key: "OS", value: "\(device.os.rawValue) · \(device.osVersion)"),
            KeyValueRow(
                key: "Role",
                value: device.isRunner ? "Runner · runs bots with its own credentials" : "Device · never runs bots"),
            KeyValueRow(
                key: "Last seen",
                value: device.status == .online ? "Active now" : Format.lastSeen(device.lastSeen)),
            KeyValueRow(
                key: "Relay",
                value: store.relayURL.map { store.relayConnected ? $0 : "\($0) · offline" } ?? "Not configured",
                monospaced: true),
        ]
        if !device.isThisDevice {
            let unpair = ActionRow(key: "Pairing", value: "Paired to this account", tint: .secondaryLabelColor, actionTitle: "Unpair…")
            unpair.onAction = { [weak self] in UnpairDevice.confirm(device, in: self?.view.window) }
            machineRows.append(unpair)
        }
        machineSection.setRows(machineRows)

        note.stringValue =
            if !device.isRunner {
                "\(device.os.displayName) Devices hold your keys and chats but never run a bot. Assign bots to a Runner: a Device running macOS, Linux, or Windows."
            } else if device.isThisDevice {
                "Keys for this Runner live in the CLI on this Mac. Connecting a provider here never leaves the machine."
            } else {
                "Provider credentials live on \(device.name). Connect DeepSeek, Anthropic, ChatGPT, or Grok from the Tinybot app running there — this Mac only sends encrypted job envelopes."
            }
    }

    /// What this Runner has installed, with each plugin's state, and the marketplace.
    private func pluginRows(on device: Device) -> [NSView] {
        var rows: [NSView] = device.plugins.map { plugin in
            let row = StatusRow()
            row.configure(
                symbol: plugin.symbolName,
                title: plugin.name,
                subtitle: plugin.description,
                state: plugin.detail,
                stateColor: plugin.stateColor
            )
            row.identifier = NSUserInterfaceItemIdentifier(plugin.id)
            row.addGestureRecognizer(NSClickGestureRecognizer(target: self, action: #selector(openPlugin(_:))))
            return row
        }
        if rows.isEmpty {
            rows.append(KeyValueRow(key: "No plugins installed", value: "", tint: .secondaryLabelColor))
        }
        let add = ActionRow(key: "Marketplace", value: "", tint: .secondaryLabelColor, actionTitle: "Add from Plugins…")
        add.onAction = { [weak self] in self?.presentAsSheet(PluginsMarketplaceViewController(runner: device, bot: nil)) }
        rows.append(add)
        return rows
    }

    @objc private func openPlugin(_ sender: NSClickGestureRecognizer) {
        guard let id = sender.view?.identifier?.rawValue, let deviceID, let device = store.device(deviceID) else { return }
        presentAsSheet(PluginViewController(pluginID: id, runner: device, bot: nil))
    }

    private func openChat(with bot: Bot) {
        onOpenChat?(store.dm(with: bot.id))
    }

    private func connectProvider(_ kind: ProviderCredential.Kind, on device: Device) {
        guard device.isThisDevice else {
            explainProviderSetup(on: device)
            return
        }
        let current = device.providers.first { $0.kind == kind }
        presentAsSheet(ConnectProviderViewController(kind: kind, baseURL: current?.baseURL))
    }

    private func explainProviderSetup(on device: Device) {
        let alert = NSAlert()
        alert.messageText =
            device.isThisDevice
            ? "Connect a provider on this Mac" : "Connect it on \(device.name)"
        alert.informativeText =
            device.isThisDevice
            ? "The CLI stores the key in the keychain on this Runner. Bots assigned here use it directly."
            : "Credentials never sync. Open Tinybot on \(device.name) and connect the provider there; bots assigned to it pick it up on the next turn."
        alert.addButton(withTitle: "OK")
        if let window = view.window {
            alert.beginSheetModal(for: window)
        } else {
            alert.runModal()
        }
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
                    "This Mac · CLI running"
                } else if device.isRunner {
                    "Online · paired"
                } else {
                    "Online · paired · not a Runner"
                }
            status.textColor = .systemGreen
        case .pairing:
            status.stringValue = "Pairing…"
            status.textColor = .systemOrange
        case .offline:
            status.stringValue =
                Format.lastSeen(device.lastSeen) + (device.isRunner ? " · jobs wait on the relay" : "")
            status.textColor = .secondaryLabelColor
        }
    }
}
