import AppKit

/// The marketplace for one Runner, after Grok Bot's Plugins overlay: search, a row per plugin
/// with Install (or its state on this Runner), and Add MCP Server for a pasted config. An
/// install is for every bot on the Runner.
final class PluginsMarketplaceViewController: SheetViewController {
    private let store = AppStore.shared
    private let runner: Device
    private let bot: Bot?

    private let search = NSSearchField()
    private let list = SectionView(title: L("Popular plugins"))
    private let custom = SectionView(title: L("Add MCP Server"))
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
            title: L("Plugins"),
            subtitle: bot.map { L("Installed on %@, for %@ and every other bot there.", runner.name, $0.name) }
                ?? L("Installed on %@, for every bot there.", runner.name),
            width: 560
        )
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()
        search.placeholderString = L("Search plugins")
        search.controlSize = .regular
        search.target = self
        search.action = #selector(searchChanged)
        search.translatesAutoresizingMaskIntoConstraints = false

        nameField.placeholderString = L("Name, such as My Server")
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
        addButton.title = L("Add to %@", runner.name)
        addButton.bezelStyle = .rounded
        addButton.controlSize = .small
        addButton.target = self
        addButton.action = #selector(addCustom)
        let customStack = Build.stack([nameField, jsonScroll, addButton], spacing: 8)
        custom.setRows([customStack])
        custom.isHidden = true

        customToggle.title = L("Add MCP Server…")
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
        setButtons(confirm: L("Done"), cancel: nil)
        list.setRows([KeyValueRow(key: L("Loading the marketplace…"), value: "", tint: .secondaryLabelColor)])
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
                self.list.setRows([KeyValueRow(key: L("The marketplace isn't available right now."), value: "", tint: .secondaryLabelColor)])
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
            list.setRows([KeyValueRow(key: search.stringValue.isEmpty ? L("Nothing in the marketplace yet.") : L("No plugin matches."), value: "", tint: .secondaryLabelColor)])
        } else {
            list.setRows(plugins.map { plugin in
                let here = installed.first { $0.id == plugin.id }
                let row = StatusRow()
                let extras = [plugin.signsIn ? L("Signs in") : nil, plugin.installedOn.isEmpty ? nil : L("On %@", plugin.installedOn.compactMap { self.store.device($0)?.name }.joined(separator: ", "))].compactMap { $0 }
                row.configure(
                    symbol: plugin.symbolName,
                    title: plugin.name,
                    subtitle: ([plugin.description] + extras).joined(separator: " · "),
                    state: here?.detail,
                    stateColor: here?.stateColor ?? .secondaryLabelColor,
                    actionTitle: here == nil ? (busy.contains(plugin.id) ? L("Installing…") : L("Install")) : nil
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
                self.status.stringValue = self.nextStep(for: status)
            } catch {
                self.status.stringValue = L("Couldn't install %@: %@", plugin.name, error.localizedDescription)
            }
        }
    }

    @objc private func toggleCustom() {
        custom.isHidden.toggle()
        customToggle.title = custom.isHidden ? L("Add MCP Server…") : L("Hide")
        fitSheetToContent()
    }

    @objc private func addCustom() {
        let name = nameField.stringValue.trimmingCharacters(in: .whitespaces)
        guard !name.isEmpty else {
            status.stringValue = L("Give the server a name.")
            return
        }
        guard let data = jsonView.string.data(using: .utf8), let json = try? JSONSerialization.jsonObject(with: data) else {
            status.stringValue = L("That is not valid JSON.")
            return
        }
        addButton.isEnabled = false
        Task { [weak self] in
            guard let self else { return }
            defer { self.addButton.isEnabled = true }
            do {
                let installed = try await self.store.installMCPServer(named: name, json: json, on: self.runner.id)
                self.status.stringValue = L("%@ added to %@.", installed.name, self.runner.name) + " " + self.nextStep(for: installed)
                self.custom.isHidden = true
                self.customToggle.title = L("Add MCP Server…")
                self.render()
            } catch {
                self.status.stringValue = L("Couldn't add it: %@", error.localizedDescription)
                self.fitSheetToContent()
            }
        }
    }

    private func nextStep(for plugin: InstalledPlugin) -> String {
        switch plugin.state {
        case .ready: L("%@ is ready for every bot on %@.", plugin.name, runner.name)
        case .needsAuth: L("%@ needs a sign-in: open it and click Sign in.", plugin.name)
        case .needsSetup: L("%@ needs setup: %@.", plugin.name, plugin.detail)
        case .connecting, .error, .unknown: "\(plugin.name): \(plugin.detail)"
        }
    }
}
