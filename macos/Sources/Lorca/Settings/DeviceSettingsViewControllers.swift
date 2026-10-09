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
        if device.os == .unknown { return [NoteRow(text: Device.unknownNote)] }
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
                detailText: bot.provider.name,
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

/// The account's provider credentials: connected on any Device, used by every Runner. Then each
/// connected provider's review model, the one Auto-review runs on it.
final class ProvidersSettingsViewController: SettingsPaneViewController {
    private let store = AppStore.shared
    private let section = SectionView(title: L("Credentials"))
    private let reviewSection = SectionView(title: SettingsEntry.reviewModels.row)

    override func viewDidLoad() {
        title = L("Providers")
        addSection(section)
        addFootnote(
            L("Credentials belong to your account. They reach your paired Devices encrypted with the account key, so a bot uses them on whichever Runner it is assigned to; the relay stores ciphertext."))
        addSection(reviewSection)
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
            reviewSection.isHidden = true
            return
        }
        renderReviewModels()
        var rows: [NSView] = store.providers.map { credential in
            let row = StatusRow()
            // A subscription disconnects right here; an API key or a custom provider opens its sheet.
            let disconnects = credential.isConnected && !credential.kind.usesAPIKey && !credential.kind.isCustom
            row.configure(
                symbol: credential.kind.symbolName,
                title: credential.kind.name,
                subtitle: "\(credential.kind.subtitle) · \(credential.detail)",
                state: credential.isConnected ? L("Connected") : nil,
                stateColor: .systemGreen,
                actionTitle: credential.isConnected ? (disconnects ? L("Disconnect") : L("Edit…")) : L("Connect…"),
                destructive: disconnects
            )
            row.onAction = { [weak self] in
                guard let self else { return }
                if credential.kind.isCustom {
                    CustomProviderViewController.present(kind: credential.kind, from: self)
                } else if disconnects {
                    Task { try? await self.store.disconnectProvider(credential.kind) }
                } else {
                    ConnectProviderViewController.present(kind: credential.kind, from: self, baseURL: credential.baseURL)
                }
            }
            return row
        }
        let add = ActionRow(key: L("Custom"), value: "", tint: .secondaryLabelColor, actionTitle: L("Add Provider…"))
        add.onAction = { [weak self, weak add] in
            guard let self, let button = add?.actionView else { return }
            self.addMenu().popUp(positioning: nil, at: NSPoint(x: 0, y: button.bounds.height + 4), in: button)
        }
        rows.append(add)
        section.setRows(rows)
    }

    /// The servers people often add, the decision APIs Auto-review can use, then any other. One
    /// the account has already opens it.
    private func addMenu() -> NSMenu {
        let menu = NSMenu()
        for (index, presets) in [CustomProviderPreset.cloud, CustomProviderPreset.local, CustomProviderPreset.decisions].enumerated() {
            if index > 0 { menu.addItem(.separator()) }
            for preset in presets {
                let item = NSMenuItem(title: preset.name, action: #selector(addPreset(_:)), keyEquivalent: "")
                item.target = self
                item.representedObject = preset
                item.state = existing(named: preset.name) == nil ? .off : .on
                menu.addItem(item)
            }
        }
        menu.addItem(.separator())
        let other = NSMenuItem(title: L("Other Server…"), action: #selector(addOther), keyEquivalent: "")
        other.target = self
        menu.addItem(other)
        return menu
    }

    /// A pop-up for each connected provider: its default review model, then every model it has,
    /// decision models among them. A picked model the provider no longer lists stays listed.
    private func renderReviewModels() {
        let review = store.autoReview
        let rows: [NSView] = store.reviewProviderKinds.map { kind in
            let credential = store.credential(for: kind)
            var models = store.reviewModels(for: kind)
            if let picked = review.models[kind], !models.contains(where: { $0.id == picked }) {
                models.append(ProviderModel(provider: kind, id: picked, label: picked, levels: []))
            }
            let popUp = SettingsPopUpButton()
            let fallback = credential?.reviewModel.map { id in models.first { $0.id == id }?.label ?? id }
            popUp.addItem(withTitle: fallback.map { L("Default (%@)", $0) } ?? L("Default"))
            popUp.menu?.addItem(.separator())
            for model in models {
                popUp.addItem(withTitle: model.label)
                popUp.lastItem?.representedObject = model.id
            }
            popUp.select(popUp.itemArray.first { $0.representedObject as? String == review.models[kind] } ?? popUp.item(at: 0))
            popUp.target = self
            popUp.action = #selector(reviewModelPicked(_:))
            popUp.identifier = NSUserInterfaceItemIdentifier(kind.wireValue)
            return AccessoryRow(key: credential?.name ?? kind.name, accessory: popUp)
        }
        reviewSection.isHidden = rows.isEmpty
        reviewSection.setRows(rows + [
            NoteRow(text: L("Auto-review runs the review model of the bot's provider, or of the provider picked in Auto-review. A decision model writes no rule, so a card it pauses offers Allow once and Deny."))
        ])
    }

    @objc private func reviewModelPicked(_ sender: NSPopUpButton) {
        guard let wire = sender.identifier?.rawValue, let kind = ProviderCredential.Kind(wireValue: wire) else { return }
        let model = sender.selectedItem?.representedObject as? String
        guard model != store.autoReview.models[kind] else { return }
        store.setReviewModel(model, for: kind)
    }

    private func existing(named name: String) -> ProviderCredential? {
        store.providers.first { $0.kind.isCustom && $0.name?.caseInsensitiveCompare(name) == .orderedSame }
    }

    @objc private func addPreset(_ sender: NSMenuItem) {
        guard let preset = sender.representedObject as? CustomProviderPreset else { return }
        CustomProviderViewController.present(kind: existing(named: preset.name)?.kind, preset: preset, from: self)
    }

    @objc private func addOther() {
        CustomProviderViewController.present(kind: nil, from: self)
    }
}

// MARK: - Plugins

/// What the picked Runner has installed, with each plugin's state, and the marketplace; then the
/// MCP servers of its mcp.json.
final class PluginsSettingsViewController: DevicePaneViewController {
    private let section = SectionView(title: L("Plugins"))
    private let mcpSection = SectionView(title: L("MCP Servers"))
    private let mcpFootnote = Build.label("", font: Theme.Font.caption, color: .tertiaryLabelColor, lines: 0)
    /// The picked Runner's mcp.json as it last answered, and which Runner that was.
    private var mcpFile: McpFile?
    private var mcpFileDeviceID: Device.ID?
    private var mcpFailure: String?
    /// What the list was last asked for: the Runner, and how its servers stood.
    private var mcpKey = ""
    private var mcpLoads = 0
    private var mcpReload: DispatchWorkItem?
    var onOpenMarketplace: ((Device.ID) -> Void)?

    override func viewDidLoad() {
        title = L("Plugins")
        addSection(section)
        addFootnote(L("Plugins are installed on a Runner, and the bots assigned to it use them. An action that changes something goes through Auto-review first."))
        addSection(mcpSection)
        add(mcpFootnote)
        super.viewDidLoad()
        // This Mac's file, which the CLI reads again on every change, is asked again whenever the
        // CLI says something changed: an edit that broke it moves no server's state.
        store.observe(self) { [weak self] event in
            guard case .rosterChanged = event, let self, self.device?.isThisDevice == true else { return }
            self.mcpReload?.cancel()
            let work = DispatchWorkItem { [weak self] in
                guard let self, let device = self.device else { return }
                self.loadMcpServers(on: device)
            }
            self.mcpReload = work
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.3, execute: work)
        }
    }

    override func reload() {
        section.title = device.map { L("Plugins on %@", $0.name) } ?? L("Plugins")
        if let rows = placeholderRows(for: device) {
            section.setRows(rows)
            mcpSection.isHidden = true
            mcpFootnote.isHidden = true
            return
        }
        guard let device else { return }
        reloadMcpServers(on: device)
        // Its mcp.json's servers are listed in their own section.
        var rows: [NSView] = device.plugins.filter { !$0.isMcpServer }.map { plugin in
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
        PluginViewController.present(pluginID: id, runner: device, bot: nil, from: self)
    }

    // MARK: - MCP servers

    /// Asks again when another Device is picked, or a server of this one changes how it stands.
    private func reloadMcpServers(on device: Device) {
        mcpSection.isHidden = false
        mcpFootnote.isHidden = false
        mcpSection.title = L("MCP Servers on %@", device.name)
        mcpFootnote.stringValue = L(
            "Servers you add yourself live in mcp.json on %@, in the format Claude Desktop and Cursor use. Edit them here or with the lorca mcp command; after editing the file itself, click Reload.",
            device.name)
        let states = device.plugins.filter(\.isMcpServer).map { "\($0.id):\($0.state.rawValue):\($0.detail)" }
        let key = "\(device.id)|\(states.joined(separator: ","))"
        if key != mcpKey {
            mcpKey = key
            loadMcpServers(on: device)
        }
        renderMcpServers(on: device)
    }

    private func loadMcpServers(on device: Device) {
        mcpLoads += 1
        let load = mcpLoads
        let id = device.id
        Task { [weak self] in
            guard let self else { return }
            do {
                let file = try await self.store.mcpServers(on: id)
                guard load == self.mcpLoads else { return }
                self.mcpFile = file
                self.mcpFileDeviceID = id
                self.mcpFailure = nil
            } catch {
                guard load == self.mcpLoads else { return }
                self.mcpFailure = error.localizedDescription
            }
            if let current = self.device, current.id == id { self.renderMcpServers(on: current) }
        }
    }

    /// Each server with how it stands and a switch, a row to add one, and on this Mac, the file.
    private func renderMcpServers(on device: Device) {
        guard let file = mcpFile, mcpFileDeviceID == device.id else {
            if let failure = mcpFailure {
                mcpSection.setRows([NoteRow(text: failure, tint: .systemRed)])
            } else {
                mcpSection.setRows([KeyValueRow(key: L("Loading…"), value: "")])
            }
            return
        }
        var rows: [NSView] = []
        if let error = file.error {
            rows.append(NoteRow(text: L("%@ Lorca keeps the servers it read before.", error), tint: .systemRed))
        }
        for server in file.servers {
            let row = SwitchRow()
            row.configure(server: server)
            row.onToggle = { [weak self] on in self?.setMcpServer(server, enabled: on, on: device) }
            row.onClick = { [weak self] in
                guard let self else { return }
                McpServerViewController.present(runner: device, name: server.name, from: self)
            }
            rows.append(row)
        }
        if file.servers.isEmpty, file.error == nil {
            rows.append(NoteRow(text: L("No MCP servers yet. Add one by the command that starts it or its URL, or paste the JSON from its README.")))
        }
        let add = ActionRow(key: L("Custom"), value: "", tint: .secondaryLabelColor, actionTitle: L("Add Server…"))
        add.onAction = { [weak self] in
            guard let self else { return }
            McpServerViewController.present(runner: device, from: self)
        }
        rows.append(add)
        // Reload reads an edit made outside Lorca; Open shows the file here once it is there,
        // which it is once it holds a server.
        let opens = device.isThisDevice && (!file.servers.isEmpty || file.error != nil)
        let fileRow = ActionRow(key: "mcp.json", value: file.path, tint: .secondaryLabelColor, actionTitle: L("Reload"), secondActionTitle: opens ? L("Open") : nil)
        fileRow.toolTip = file.path
        fileRow.onAction = { [weak self] in self?.reloadMcpFile(on: device) }
        fileRow.onSecondAction = {
            let url = URL(fileURLWithPath: file.path)
            if !NSWorkspace.shared.open(url) { NSWorkspace.shared.activateFileViewerSelecting([url]) }
        }
        rows.append(fileRow)
        mcpSection.setRows(rows)
    }

    /// Has the Runner read its mcp.json again, after an edit made outside Lorca.
    private func reloadMcpFile(on device: Device) {
        mcpLoads += 1
        Task { [weak self] in
            guard let self else { return }
            do {
                let file = try await self.store.reloadMcpFile(on: device.id)
                self.mcpFile = file
                self.mcpFileDeviceID = device.id
                self.mcpFailure = nil
            } catch {
                if let window = self.view.window {
                    let alert = NSAlert()
                    alert.messageText = L("Couldn't reload mcp.json")
                    alert.informativeText = error.localizedDescription
                    await alert.beginSheetModal(for: window)
                }
            }
            if let current = self.device, current.id == device.id { self.renderMcpServers(on: current) }
        }
    }

    private func setMcpServer(_ server: McpServer, enabled: Bool, on device: Device) {
        Task { [weak self] in
            guard let self else { return }
            do {
                let answered = try await self.store.setMcpServer(server.name, enabled: enabled, on: device.id)
                if var file = self.mcpFile, let index = file.servers.firstIndex(where: { $0.name == server.name }) {
                    file.servers[index] = answered
                    self.mcpFile = file
                }
            } catch {
                if let window = self.view.window {
                    let alert = NSAlert()
                    alert.messageText = enabled ? L("Couldn't turn %@ on", server.name) : L("Couldn't turn %@ off", server.name)
                    alert.informativeText = error.localizedDescription
                    await alert.beginSheetModal(for: window)
                }
            }
            if let current = self.device, current.id == device.id { self.renderMcpServers(on: current) }
        }
    }
}

// MARK: - Devices

/// The picked Device itself: what it is, whether it is online, its machine key, whether
/// `lorca service` keeps a Runner's CLI running, and Unpair.
final class AboutDeviceSettingsViewController: DevicePaneViewController {
    private let header = DeviceHeaderView()
    private let machineSection = SectionView(title: L("Machine"))
    /// How to keep a Runner available without the app, under the card while it has no service.
    private let serviceNote = Build.label("", font: Theme.Font.caption, color: .tertiaryLabelColor, lines: 0)
    /// What the Runner shown said of its service; asked again each time the pane shows.
    private var service: Wire.ServiceStatus?
    private var serviceAsked: Device.ID?
    private var isAskingService = false

    override func viewDidLoad() {
        title = L("Devices")
        add(header)
        addSection(machineSection)
        add(serviceNote)
        super.viewDidLoad()
    }

    override func viewWillAppear() {
        super.viewWillAppear()
        guard !isAskingService else { return }
        serviceAsked = nil
        reload()
    }

    override func reload() {
        guard let device else { return }
        header.configure(device: device)

        // A machine that never said what it is has no system to show, only why it is listed.
        let unknown = device.os == .unknown
        var rows: [NSView] = []
        if unknown {
            rows.append(NoteRow(text: Device.unknownNote))
        }
        rows.append(KeyValueRow(key: SettingsEntry.machineKey.row, value: device.machineKey, monospaced: true))
        if !unknown {
            rows.append(KeyValueRow(key: L("OS"), value: "\(device.os.displayName) · \(device.osVersion)"))
        }
        if let update = device.update {
            rows.append(updateRow(device, update))
        }
        rows.append(
            KeyValueRow(
                key: L("Role"),
                value: device.isRunner ? L("Runner · runs bots with its own credentials") : L("Device · never runs bots")))
        askService(device)
        let service = device.isRunner && device.status == .online ? service : nil
        if let service {
            rows.append(
                KeyValueRow(
                    key: L("Background service"),
                    value: service.running ? L("Running") : service.installed ? L("Installed, not running") : L("Not installed")))
        }
        serviceNote.stringValue = L("Run lorca service install in Terminal on %@ to keep its bots running while Lorca is closed.", device.name)
        serviceNote.isHidden = service?.installed != false
        rows.append(
            KeyValueRow(
                key: L("Last seen"),
                value: device.status == .online ? L("Active now") : Format.lastSeen(device.lastSeen)))
        rows.append(
            KeyValueRow(
                key: L("Relay"),
                value: store.relayURL.map { url in
                    if store.relayUpdateRequired { return L("%@ · update Lorca to sync", url) }
                    if store.relayConnected { return url }
                    // Why the last try to connect failed, in the CLI's words.
                    return store.relayError.map { "\(url) · \($0)" } ?? L("%@ · offline", url)
                } ?? L("Not configured"),
                monospaced: true))
        let unpair = ActionRow(
            key: SettingsEntry.pairing.row, value: L("Paired to this account"), tint: .secondaryLabelColor,
            actionTitle: L("Unpair…"))
        unpair.onAction = { [weak self] in UnpairDevice.confirm(device, in: self?.view.window) }
        rows.append(unpair)
        machineSection.setRows(rows)
    }

    /// Asks an online Runner whether `lorca service` keeps its CLI running, once per Device shown.
    private func askService(_ device: Device) {
        guard serviceAsked != device.id else { return }
        service = nil
        guard device.isRunner, device.status == .online else { return }
        serviceAsked = device.id
        isAskingService = true
        Task { @MainActor [weak self] in
            let status = try? await self?.store.serviceStatus(device.id)
            guard let self else { return }
            self.isAskingService = false
            guard self.serviceAsked == device.id else { return }
            self.service = status
            self.reload()
        }
    }

    /// The version of a CLI that updates itself, how its update goes, and Update while a newer
    /// release waits and the Runner is online.
    private func updateRow(_ device: Device, _ update: Device.CLIUpdate) -> NSView {
        let version = device.version
        let latest = update.latest ?? ""
        let value: String
        var offersUpdate = false
        switch update.state {
        case "installing": value = L("%@ · Installing %@…", version, latest)
        case "restarting": value = L("%@ · Restarts into %@ once no bot is at work", version, latest)
        case "installed": value = L("%@ · %@ is installed; restart lorca serve to run it", version, latest)
        default:
            if !latest.isEmpty {
                value = L("%@ · %@ is available", version, latest)
                offersUpdate = device.status == .online
            } else if let error = update.error {
                value = "\(version) · \(error)"
            } else if update.auto {
                value = L("%@ · Up to date", version)
            } else {
                value = L("%@ · Up to date · automatic updates off", version)
            }
        }
        let row = ActionRow(
            key: L("Lorca CLI"), value: value, tint: .secondaryLabelColor, actionTitle: offersUpdate ? L("Update") : nil)
        row.toolTip = update.error
        row.onAction = { [weak self] in
            Task { @MainActor in
                do {
                    try await self?.store.updateDevice(device.id)
                } catch {
                    let failed = NSAlert()
                    failed.messageText = L("Couldn’t update %@", device.name)
                    failed.informativeText = error.localizedDescription
                    failed.addButton(withTitle: L("OK"))
                    if let window = self?.view.window {
                        failed.beginSheetModal(for: window) { _ in }
                    } else {
                        failed.runModal()
                    }
                }
            }
        }
        return row
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
        model.isHidden = device.os == .unknown
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
