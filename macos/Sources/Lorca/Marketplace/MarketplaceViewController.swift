import AppKit

/// The marketplace, after Grok Bot's: featured plugins and bots, everything else by category,
/// one search over both, and a page for each plugin and bot. A sheet with a way back: the home
/// page leads to a plugin, a bot, a full list, or the plugins the Runner has. Plugins install
/// on the Runner picked in the top bar, for every bot there; a bot is added to that Runner, and
/// the sheet closes on the new bot's chat, where the bot sets itself up.
final class MarketplaceViewController: NSViewController {
    enum Loading { case loading, loaded, failed }

    let store = AppStore.shared
    private(set) var catalog = Marketplace()
    private(set) var loading = Loading.loading
    /// Where plugins install and bots are added.
    private(set) var runnerID: Device.ID?
    /// Plugins being installed, by id.
    private(set) var installing: Set<MarketplacePlugin.ID> = []
    /// What an install answered, by Runner and plugin, until the Runner's own list has it: a
    /// Runner elsewhere reports its plugins through the relay a moment later.
    private var installed: [Device.ID: [MarketplacePlugin.ID: InstalledPlugin]] = [:]

    private let size: NSSize
    private let onOpenChat: (Chat.ID) -> Void
    private var pages: [MarketplacePage] = []
    private let pageHost = NSView()
    private lazy var backButton = HoverButton(symbol: "chevron.left", tooltip: L("Back"), target: self, action: #selector(goBack))
    private lazy var closeButton = HoverButton(symbol: "xmark", pointSize: 14, tooltip: L("Close"), target: self, action: #selector(close))
    private let runnerPopup = SettingsPopUpButton()
    private let notice = BackgroundView()
    private let noticeLabel = Build.label("", font: .systemFont(ofSize: 12.5), lines: 2)
    private var noticeTask: Task<Void, Never>?

    init(runnerID: Device.ID?, size: NSSize, onOpenChat: @escaping (Chat.ID) -> Void) {
        self.size = size
        self.onOpenChat = onOpenChat
        super.init(nibName: nil, bundle: nil)
        let runners = store.runners
        self.runnerID = runners.first { $0.id == runnerID }?.id ?? store.thisDevice.flatMap { this in runners.first { $0.id == this.id } }?.id
            ?? runners.first?.id
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    var runner: Device? { runnerID.flatMap(store.device) }

    override func loadView() {
        let container = NSView()
        container.translatesAutoresizingMaskIntoConstraints = false

        runnerPopup.target = self
        runnerPopup.action = #selector(runnerPicked)
        runnerPopup.toolTip = L("Plugins install here, and bots are added here.")
        runnerPopup.translatesAutoresizingMaskIntoConstraints = false
        backButton.translatesAutoresizingMaskIntoConstraints = false
        closeButton.translatesAutoresizingMaskIntoConstraints = false
        // Escape closes the sheet from any page, wherever the keyboard is, the search field too.
        closeButton.keyEquivalent = "\u{1b}"
        backButton.isHidden = true
        pageHost.translatesAutoresizingMaskIntoConstraints = false

        notice.fillColor = .windowBackgroundColor
        notice.borderColor = .separatorColor
        notice.cornerRadius = 10
        notice.shadow = {
            let shadow = NSShadow()
            shadow.shadowBlurRadius = 12
            shadow.shadowOffset = NSSize(width: 0, height: -2)
            shadow.shadowColor = NSColor.black.withAlphaComponent(0.18)
            return shadow
        }()
        notice.isHidden = true
        notice.addSubview(noticeLabel)

        for view in [pageHost, backButton, runnerPopup, closeButton, notice] as [NSView] {
            container.addSubview(view)
        }
        NSLayoutConstraint.activate([
            container.widthAnchor.constraint(equalToConstant: size.width),
            container.heightAnchor.constraint(equalToConstant: size.height),

            backButton.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 10),
            backButton.centerYAnchor.constraint(equalTo: container.topAnchor, constant: 24),
            closeButton.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -10),
            closeButton.centerYAnchor.constraint(equalTo: backButton.centerYAnchor),
            runnerPopup.trailingAnchor.constraint(equalTo: closeButton.leadingAnchor, constant: -8),
            runnerPopup.centerYAnchor.constraint(equalTo: backButton.centerYAnchor),

            pageHost.topAnchor.constraint(equalTo: container.topAnchor, constant: 44),
            pageHost.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            pageHost.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            pageHost.bottomAnchor.constraint(equalTo: container.bottomAnchor),

            notice.centerXAnchor.constraint(equalTo: container.centerXAnchor),
            notice.bottomAnchor.constraint(equalTo: container.bottomAnchor, constant: -18),
            notice.widthAnchor.constraint(lessThanOrEqualTo: container.widthAnchor, constant: -96),
            noticeLabel.leadingAnchor.constraint(equalTo: notice.leadingAnchor, constant: 14),
            noticeLabel.trailingAnchor.constraint(equalTo: notice.trailingAnchor, constant: -14),
            noticeLabel.topAnchor.constraint(equalTo: notice.topAnchor, constant: 9),
            noticeLabel.bottomAnchor.constraint(equalTo: notice.bottomAnchor, constant: -9),
        ])
        view = container
        updateRunnerPopup()
        show(MarketplaceHomePage(market: self), animated: false)
        load()
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        store.observe(self) { [weak self] event in
            guard let self else { return }
            if case .rosterChanged = event {
                self.updateRunnerPopup()
                self.reloadPages()
            }
        }
    }

    // MARK: - Loading

    func load() {
        loading = .loading
        reloadPages()
        Task { [weak self] in
            guard let self else { return }
            do {
                self.catalog = try await self.store.marketplace()
                self.loading = .loaded
            } catch {
                self.loading = .failed
            }
            self.reloadPages()
        }
    }

    private func reloadPages() {
        for page in pages where page.isViewLoaded { page.reload() }
    }

    func plugin(_ id: MarketplacePlugin.ID) -> MarketplacePlugin? {
        catalog.plugins.first { $0.id == id }
    }

    /// The plugin as the picked Runner has it, nil when it is not installed there.
    func installedPlugin(_ id: MarketplacePlugin.ID) -> InstalledPlugin? {
        guard let runner else { return nil }
        return runner.plugins.first { $0.id == id } ?? installed[runner.id]?[id]
    }

    /// Everything the picked Runner has, the marketplace's and the rest, in its own order.
    var installedPlugins: [InstalledPlugin] {
        guard let runner else { return [] }
        let extra = (installed[runner.id] ?? [:]).values.filter { plugin in !runner.plugins.contains { $0.id == plugin.id } }
        return runner.plugins + extra.sorted { $0.name < $1.name }
    }

    // MARK: - Runner

    private func updateRunnerPopup() {
        let runners = store.runners
        if let runnerID, !runners.contains(where: { $0.id == runnerID }) {
            self.runnerID = runners.first?.id
        }
        // One Runner is no choice; the pages name it where it matters.
        runnerPopup.isHidden = runners.count < 2
        runnerPopup.removeAllItems()
        for device in runners {
            runnerPopup.addItem(withTitle: device.isThisDevice ? L("%@ (this computer)", device.name) : device.name)
            runnerPopup.lastItem?.representedObject = device.id
        }
        if let index = runners.firstIndex(where: { $0.id == runnerID }) {
            runnerPopup.selectItem(at: index)
        }
        runnerPopup.invalidateIntrinsicContentSize()
    }

    @objc private func runnerPicked() {
        runnerID = runnerPopup.selectedItem?.representedObject as? Device.ID
        reloadPages()
    }

    // MARK: - Pages

    func show(_ page: MarketplacePage, animated: Bool = true) {
        let old = pages.last
        pages.append(page)
        swap(from: old, to: page, animated: animated)
    }

    @objc func goBack() {
        guard pages.count > 1 else { return }
        let old = pages.removeLast()
        swap(from: old, to: pages.last, animated: true)
    }

    private func swap(from old: MarketplacePage?, to new: MarketplacePage?, animated: Bool) {
        backButton.isHidden = pages.count < 2
        if let new {
            if new.parent == nil { addChild(new) }
            new.view.translatesAutoresizingMaskIntoConstraints = false
            pageHost.addSubview(new.view)
            new.view.pin(to: pageHost)
            new.reload()
            new.view.alphaValue = animated ? 0 : 1
        }
        let finish = { [weak self] in
            old?.view.removeFromSuperview()
            if let old, self?.pages.contains(where: { $0 === old }) == false { old.removeFromParent() }
            new?.pageDidAppear()
        }
        guard animated else { return finish() }
        NSAnimationContext.runAnimationGroup({ context in
            context.duration = 0.18
            old?.view.animator().alphaValue = 0
            new?.view.animator().alphaValue = 1
        }, completionHandler: finish)
    }

    @objc private func close() {
        dismiss(nil)
    }

    override func cancelOperation(_ sender: Any?) {
        close()
    }

    func openPlugin(_ plugin: MarketplacePlugin) {
        show(MarketplacePluginPage(market: self, pluginID: plugin.id))
    }

    func openBot(_ template: BotTemplate) {
        show(MarketplaceBotPage(market: self, templateID: template.id))
    }

    func openList(title: String, items: @escaping (Marketplace) -> [MarketplaceItem]) {
        show(MarketplaceListPage(market: self, heading: title, items: items))
    }

    func openInstalled() {
        show(MarketplaceInstalledPage(market: self))
    }

    // MARK: - Actions

    /// Installs a plugin on the picked Runner, for every bot there.
    func install(_ plugin: MarketplacePlugin) {
        guard let runner, !installing.contains(plugin.id) else { return }
        installing.insert(plugin.id)
        reloadPages()
        Task { [weak self] in
            guard let self else { return }
            do {
                let status = try await self.store.installPlugin(plugin.id, on: runner.id)
                self.installed[runner.id, default: [:]][plugin.id] = status
                self.showNotice(self.nextStep(for: status, on: runner))
            } catch {
                self.showNotice(L("Couldn't install %@: %@", plugin.name, error.localizedDescription), isError: true)
            }
            self.installing.remove(plugin.id)
            self.reloadPages()
        }
    }

    private func nextStep(for plugin: InstalledPlugin, on runner: Device) -> String {
        switch plugin.state {
        case .ready: L("Added %@. Every bot on %@ can use it.", plugin.name, runner.name)
        case .needsAuth: L("Added %@. It needs a sign-in: click Connect.", plugin.name)
        case .needsSetup: L("Added %@. It needs setup: click Set Up.", plugin.name)
        case .connecting, .error, .unknown: "\(plugin.name): \(plugin.detail)"
        }
    }

    /// The plugin's own sheet on the picked Runner: its sign-in, its setup, and Remove.
    func manage(_ pluginID: MarketplacePlugin.ID) {
        guard let runner else { return }
        presentAsSheet(PluginViewController(pluginID: pluginID, runner: runner, bot: nil))
    }

    /// Adds the bot on the picked Runner and opens its chat, where it greets the user.
    func add(_ template: BotTemplate) {
        guard let runner else { return }
        let chatID = store.addBot(from: template, runnerID: runner.id)
        dismiss(nil)
        onOpenChat(chatID)
    }

    // MARK: - Notices

    /// A line at the foot of the sheet for what an install did, gone after a few seconds.
    func showNotice(_ text: String, isError: Bool = false) {
        noticeLabel.stringValue = text
        noticeLabel.textColor = isError ? .systemRed : .labelColor
        notice.isHidden = false
        notice.alphaValue = 1
        noticeTask?.cancel()
        noticeTask = Task { [weak self] in
            try? await Task.sleep(nanoseconds: isError ? 7_000_000_000 : 4_000_000_000)
            guard !Task.isCancelled, let self else { return }
            NSAnimationContext.runAnimationGroup({ context in
                context.duration = 0.25
                self.notice.animator().alphaValue = 0
            }, completionHandler: { [weak self] in
                guard let self, !Task.isCancelled else { return }
                self.notice.isHidden = true
            })
        }
    }
}

/// One page of the marketplace sheet: a scroll view over a column of content.
class MarketplacePage: NSViewController {
    unowned let market: MarketplaceViewController
    let content = Build.stack([], spacing: 0)
    private let scrollView = NSScrollView()

    init(market: MarketplaceViewController) {
        self.market = market
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    var store: AppStore { market.store }

    override func loadView() {
        let document = FlippedView()
        document.translatesAutoresizingMaskIntoConstraints = false
        content.alignment = .leading
        document.addSubview(content)
        scrollView.drawsBackground = false
        scrollView.hasVerticalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.documentView = document
        NSLayoutConstraint.activate([
            document.leadingAnchor.constraint(equalTo: scrollView.contentView.leadingAnchor),
            document.trailingAnchor.constraint(equalTo: scrollView.contentView.trailingAnchor),
            document.topAnchor.constraint(equalTo: scrollView.contentView.topAnchor),
            content.topAnchor.constraint(equalTo: document.topAnchor, constant: 2),
            content.leadingAnchor.constraint(equalTo: document.leadingAnchor, constant: 32),
            content.trailingAnchor.constraint(equalTo: document.trailingAnchor, constant: -32),
            content.bottomAnchor.constraint(equalTo: document.bottomAnchor, constant: -32),
        ])
        view = scrollView
    }

    /// Builds the page again from the marketplace's state.
    func reload() {}

    /// The page is on screen and settled: a good time to take the keyboard.
    func pageDidAppear() {}

    /// Replaces the content with `views`, each as wide as the column, `spacing` apart.
    func setContent(_ views: [NSView], spacing: CGFloat = 24) {
        for view in content.arrangedSubviews {
            content.removeArrangedSubview(view)
            view.removeFromSuperview()
        }
        content.spacing = spacing
        for view in views {
            content.addArrangedSubview(view)
            view.widthAnchor.constraint(equalTo: content.widthAnchor).isActive = true
        }
    }
}
