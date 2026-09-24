import AppKit

/// The marketplace for one Runner, after Grok Bot's Plugins overlay: search and a row per plugin
/// with Install (or its state on this Runner). An install is for every bot on the Runner.
final class PluginsMarketplaceViewController: SheetViewController {
    private let store = AppStore.shared
    private let runner: Device
    private let bot: Bot?

    private let search = NSSearchField()
    private let list = SectionView(title: L("Popular plugins"))
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

        contentStack.addArrangedSubview(search)
        contentStack.addArrangedSubview(list)
        contentStack.addArrangedSubview(status)
        NSLayoutConstraint.activate([
            search.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            list.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            status.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
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

    private func nextStep(for plugin: InstalledPlugin) -> String {
        switch plugin.state {
        case .ready: L("%@ is ready for every bot on %@.", plugin.name, runner.name)
        case .needsAuth: L("%@ needs a sign-in: open it and click Sign in.", plugin.name)
        case .needsSetup: L("%@ needs setup: %@.", plugin.name, plugin.detail)
        case .connecting, .error, .unknown: "\(plugin.name): \(plugin.detail)"
        }
    }
}
