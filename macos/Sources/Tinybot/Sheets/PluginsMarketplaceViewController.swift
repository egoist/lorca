import AppKit

/// The marketplace for one Runner, after Grok Bot's Plugins overlay: search, a row per plugin
/// with Install (or its state on this Runner), and Add MCP Server for a pasted config. Opened
/// from a DM's inspector, an install also enables the plugin for that bot.
final class PluginsMarketplaceViewController: SheetViewController {
    private let store = AppStore.shared
    private let runner: Device
    private let bot: Bot?

    private let search = NSSearchField()
    private let list = SectionView(title: "Popular plugins")
    private let custom = SectionView(title: "Add MCP Server")
    private let nameField = NSTextField()
    private let jsonView = NSTextView()
    private let addButton = NSButton()
    private let customToggle = NSButton()
    private let status = Build.label("", font: Theme.Font.caption, color: .secondaryLabelColor, lines: 0)

    private var plugins: [MarketplacePlugin] = []
    private var busy: Set<String> = []

    init(runner: Device, bot: Bot?) {
        self.runner = runner
        self.bot = bot
        super.init(
            title: "Plugins",
            subtitle: bot.map { "Installed on \(runner.name), used by \($0.name). Every bot on \(runner.name) can be given a plugin it has." }
                ?? "Installed on \(runner.name). Each bot there chooses which of them it uses.",
            width: 560
        )
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()
        search.placeholderString = "Search plugins"
        search.controlSize = .regular
        search.target = self
        search.action = #selector(searchChanged)
        search.translatesAutoresizingMaskIntoConstraints = false

        nameField.placeholderString = "Name, such as My Server"
        nameField.controlSize = .small
        nameField.translatesAutoresizingMaskIntoConstraints = false
        jsonView.isRichText = false
        jsonView.font = .monospacedSystemFont(ofSize: 11, weight: .regular)
        jsonView.isAutomaticQuoteSubstitutionEnabled = false
        jsonView.isAutomaticDashSubstitutionEnabled = false
        jsonView.isAutomaticTextReplacementEnabled = false
        jsonView.textContainerInset = NSSize(width: 6, height: 6)
        jsonView.isVerticallyResizable = true
        jsonView.isHorizontallyResizable = false
        jsonView.autoresizingMask = [.width]
        jsonView.textContainer?.widthTracksTextView = true
        jsonView.string = "{\n  \"mcpServers\": {\n    \"my-server\": { \"command\": \"npx\", \"args\": [\"-y\", \"@example/mcp\"] }\n  }\n}"
        let jsonScroll = NSScrollView()
        jsonScroll.documentView = jsonView
        jsonScroll.hasVerticalScroller = true
        jsonScroll.borderType = .bezelBorder
        jsonScroll.translatesAutoresizingMaskIntoConstraints = false
        addButton.title = "Add to \(runner.name)"
        addButton.bezelStyle = .rounded
        addButton.controlSize = .small
        addButton.target = self
        addButton.action = #selector(addCustom)
        let customStack = Build.stack([nameField, jsonScroll, addButton], spacing: 8)
        custom.setRows([customStack])
        custom.isHidden = true

        customToggle.title = "Add MCP Server…"
        customToggle.bezelStyle = .rounded
        customToggle.controlSize = .small
        customToggle.target = self
        customToggle.action = #selector(toggleCustom)

        contentStack.addArrangedSubview(search)
        contentStack.addArrangedSubview(list)
        contentStack.addArrangedSubview(custom)
        contentStack.addArrangedSubview(customToggle)
        contentStack.addArrangedSubview(status)
        NSLayoutConstraint.activate([
            search.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            list.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            custom.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            status.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            customStack.widthAnchor.constraint(equalTo: custom.widthAnchor, constant: -24),
            nameField.widthAnchor.constraint(equalTo: customStack.widthAnchor),
            jsonScroll.widthAnchor.constraint(equalTo: customStack.widthAnchor),
            jsonScroll.heightAnchor.constraint(equalToConstant: 110),
        ])
        setButtons(confirm: "Done", cancel: nil)
        list.setRows([KeyValueRow(key: "Loading the marketplace…", value: "", tint: .secondaryLabelColor)])
        load()
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        store.observe(self) { [weak self] event in
            if case .rosterChanged = event { self?.render() }
        }
    }

    private func load() {
        let query = search.stringValue
        Task { [weak self] in
            guard let self else { return }
            do {
                self.plugins = try await self.store.marketplace(query: query)
                self.render()
            } catch {
                self.list.setRows([KeyValueRow(key: "The marketplace isn't available right now.", value: "", tint: .secondaryLabelColor)])
                self.fitSheetToContent()
            }
        }
    }

    @objc private func searchChanged() {
        load()
    }

    private func render() {
        let installed = store.device(runner.id)?.plugins ?? []
        if plugins.isEmpty {
            list.setRows([KeyValueRow(key: search.stringValue.isEmpty ? "Nothing in the marketplace yet." : "No plugin matches.", value: "", tint: .secondaryLabelColor)])
        } else {
            list.setRows(plugins.map { plugin in
                let here = installed.first { $0.id == plugin.id }
                let row = StatusRow()
                let extras = [plugin.signsIn ? "Signs in" : nil, plugin.installedOn.isEmpty ? nil : "On \(plugin.installedOn.compactMap { self.store.device($0)?.name }.joined(separator: ", "))"].compactMap { $0 }
                row.configure(
                    symbol: plugin.symbolName,
                    title: plugin.name,
                    subtitle: ([plugin.description] + extras).joined(separator: " · "),
                    state: here?.detail,
                    stateColor: here?.stateColor ?? .secondaryLabelColor,
                    actionTitle: here == nil ? (busy.contains(plugin.id) ? "Installing…" : "Install") : nil
                )
                row.onAction = { [weak self] in self?.install(plugin) }
                return row
            })
        }
        fitSheetToContent()
    }

    private func install(_ plugin: MarketplacePlugin) {
        guard !busy.contains(plugin.id) else { return }
        busy.insert(plugin.id)
        render()
        Task { [weak self] in
            guard let self else { return }
            defer { self.busy.remove(plugin.id); self.render() }
            do {
                let status = try await self.store.installPlugin(plugin.id, on: self.runner.id)
                self.enableForBot(plugin.id)
                self.status.stringValue = self.nextStep(for: status)
            } catch {
                self.status.stringValue = "Couldn't install \(plugin.name): \(error.localizedDescription)"
            }
        }
    }

    @objc private func toggleCustom() {
        custom.isHidden.toggle()
        customToggle.title = custom.isHidden ? "Add MCP Server…" : "Hide"
        fitSheetToContent()
    }

    @objc private func addCustom() {
        let name = nameField.stringValue.trimmingCharacters(in: .whitespaces)
        guard !name.isEmpty else {
            status.stringValue = "Give the server a name."
            return
        }
        guard let data = jsonView.string.data(using: .utf8), let json = try? JSONSerialization.jsonObject(with: data) else {
            status.stringValue = "That is not valid JSON."
            return
        }
        addButton.isEnabled = false
        Task { [weak self] in
            guard let self else { return }
            defer { self.addButton.isEnabled = true }
            do {
                let installed = try await self.store.installMCPServer(named: name, json: json, on: self.runner.id)
                self.enableForBot(installed.id)
                self.status.stringValue = "\(installed.name) added to \(self.runner.name). \(self.nextStep(for: installed))"
                self.custom.isHidden = true
                self.customToggle.title = "Add MCP Server…"
                self.render()
            } catch {
                self.status.stringValue = "Couldn't add it: \(error.localizedDescription)"
                self.fitSheetToContent()
            }
        }
    }

    private func enableForBot(_ pluginID: String) {
        guard let bot, let current = store.bot(bot.id), !current.pluginIDs.contains(pluginID) else { return }
        store.setBotPlugins(bot.id, pluginIDs: current.pluginIDs + [pluginID])
    }

    private func nextStep(for plugin: InstalledPlugin) -> String {
        switch plugin.state {
        case .ready: "\(plugin.name) is ready" + (bot.map { " for \($0.name)." } ?? ".")
        case .needsAuth: "\(plugin.name) needs a sign-in: open it and click Sign in."
        case .needsSetup: "\(plugin.name) needs setup: \(plugin.detail)."
        case .connecting, .error, .unknown: "\(plugin.name): \(plugin.detail)"
        }
    }
}
