import AppKit

final class InspectorViewController: NSViewController {
    private let store = AppStore.shared

    private let scrollView = NSScrollView()
    private let column = Build.stack([], spacing: 20)
    private let participants = SectionView(title: "Bots in this chat")
    private let profile = SectionView(title: "Profile")
    private let nameRow = EditableRow(key: "Name", placeholder: "Name")
    private let labelRow = EditableRow(key: "Label", placeholder: "What it is for")
    private let descriptionRow = EditableRow(key: "Description", placeholder: "A sentence or two about what it does", multiline: true)
    private let runtime = SectionView(title: "Runs with")
    private let routing = SectionView(title: "Where turns run")
    private let security = SectionView(title: "Encryption")
    private let addButton = NSButton()

    private var selection: Selection?

    var onOpenDevice: ((Device.ID) -> Void)?
    var onRemoveBot: ((Bot.ID) -> Void)?
    var onAddBot: (() -> Void)?

    override func loadView() {
        let container = NSView()

        column.orientation = .vertical
        column.alignment = .leading
        column.edgeInsets = NSEdgeInsets(top: 18, left: 16, bottom: 20, right: 16)

        addButton.title = "Add Bot…"
        addButton.bezelStyle = .rounded
        addButton.controlSize = .regular
        addButton.target = self
        addButton.action = #selector(addBot)
        addButton.translatesAutoresizingMaskIntoConstraints = false

        profile.setRows([nameRow, labelRow, descriptionRow])

        column.addArrangedSubview(participants)
        column.addArrangedSubview(addButton)
        column.addArrangedSubview(profile)
        column.addArrangedSubview(runtime)
        column.addArrangedSubview(routing)
        column.addArrangedSubview(security)
        column.setCustomSpacing(10, after: participants)

        // Flipped so short content sits at the top of the pane, not the bottom.
        let documentView = FlippedView()
        documentView.translatesAutoresizingMaskIntoConstraints = false
        documentView.addSubview(column)

        scrollView.documentView = documentView
        scrollView.drawsBackground = false
        scrollView.hasVerticalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.translatesAutoresizingMaskIntoConstraints = false
        // Kept below the header rather than under it: a scroll view that runs under the glass
        // header gets AppKit's scroll pocket, which draws a hard edge line whenever the pointer
        // rests on the header.
        scrollView.automaticallyAdjustsContentInsets = false

        container.addSubview(scrollView)

        NSLayoutConstraint.activate([
            scrollView.topAnchor.constraint(equalTo: container.safeAreaLayoutGuide.topAnchor),
            scrollView.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            scrollView.bottomAnchor.constraint(equalTo: container.bottomAnchor),
            documentView.widthAnchor.constraint(equalTo: scrollView.widthAnchor),
            column.topAnchor.constraint(equalTo: documentView.topAnchor),
            column.leadingAnchor.constraint(equalTo: documentView.leadingAnchor),
            column.trailingAnchor.constraint(equalTo: documentView.trailingAnchor),
            column.bottomAnchor.constraint(equalTo: documentView.bottomAnchor),
            participants.widthAnchor.constraint(equalTo: column.widthAnchor, constant: -32),
            profile.widthAnchor.constraint(equalTo: column.widthAnchor, constant: -32),
            runtime.widthAnchor.constraint(equalTo: column.widthAnchor, constant: -32),
            routing.widthAnchor.constraint(equalTo: column.widthAnchor, constant: -32),
            security.widthAnchor.constraint(equalTo: column.widthAnchor, constant: -32),
        ])

        view = container
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        store.observe(self) { [weak self] event in
            switch event {
            case .chatChanged, .chatsChanged, .snapshotReplaced:
                self?.reload()
            default:
                break
            }
        }
    }

    func show(selection newSelection: Selection) {
        selection = newSelection
        reload()
    }

    func reload() {
        guard isViewLoaded, case let .chat(chatID) = selection, let chat = store.chat(chatID) else {
            return
        }

        let members = store.bots(in: chat)

        participants.setRows(
            members.map { bot in
                let host = store.device(bot.runnerID)
                let row = BotRow()
                row.configure(
                    bot: bot,
                    detailText: "\(bot.provider.rawValue) · \(host?.name ?? "unassigned")",
                    accessorySymbol: chat.canRemoveBot ? "minus.circle" : nil,
                    tooltip: "Remove from chat"
                )
                row.onAccessory = { [weak self] in self?.onRemoveBot?(bot.id) }
                return row
            })

        // A DM never takes another bot; a group does until it is full or every bot is in it.
        addButton.isHidden = chat.isDM
        addButton.isEnabled = chat.canAddBot && members.count < store.bots.count

        // A direct chat is one bot, so its profile, provider, and model are edited right here.
        let single = chat.isDM && members.count == 1
        profile.isHidden = !single
        runtime.isHidden = !single
        if single, let bot = members.first {
            nameRow.setValue(bot.name)
            labelRow.setValue(bot.label)
            descriptionRow.setValue(bot.description)
            let commit: () -> Void = { [weak self] in self?.commitProfile(of: bot.id) }
            nameRow.onCommit = commit
            labelRow.onCommit = commit
            descriptionRow.onCommit = commit
            runtime.setRows(runtimeRows(for: bot, in: chat))
        }

        let hosts = Dictionary(grouping: members, by: \.runnerID)
        routing.setRows(
            hosts.keys.sorted().compactMap { runnerID in
                guard let runner = store.device(runnerID) else { return nil }
                let botNames = (hosts[runnerID] ?? []).map(\.name).joined(separator: ", ")
                let row = StatusRow()
                row.configure(
                    symbol: runner.symbolName,
                    title: runner.name,
                    subtitle: botNames,
                    state: runner.status == .online ? "Online" : Format.lastSeen(runner.lastSeen),
                    stateColor: runner.status == .online ? .systemGreen : .secondaryLabelColor
                )
                let click = NSClickGestureRecognizer(target: self, action: #selector(openDevice(_:)))
                row.addGestureRecognizer(click)
                row.identifier = NSUserInterfaceItemIdentifier(runner.id)
                return row
            })

        security.setRows([
            KeyValueRow(key: "Transcript", value: "Encrypted on device"),
            KeyValueRow(key: "Relay sees", value: "Ciphertext only", tint: .systemGreen),
            KeyValueRow(key: "Chat blob", value: "chat · seq \(chat.messages.count)", monospaced: true),
        ])
    }

    /// Saves the profile rows when one of them finishes editing. An emptied name or label keeps
    /// the old value; the description may be cleared.
    private func commitProfile(of id: Bot.ID) {
        guard let bot = store.bot(id) else { return }
        let name = nameRow.value.isEmpty ? bot.name : nameRow.value
        let label = labelRow.value.isEmpty ? bot.label : labelRow.value
        let description = descriptionRow.value
        nameRow.setValue(name)
        labelRow.setValue(label)
        guard name != bot.name || label != bot.label || description != bot.description else { return }
        store.updateBot(id, name: name, label: label, description: description)
    }

    private func runtimeRows(for bot: Bot, in chat: Chat) -> [NSView] {
        let kinds = ProviderCredential.Kind.allCases
        let providerRow = PopUpRow(
            key: "Provider",
            items: kinds.map(\.rawValue),
            selected: kinds.firstIndex(of: bot.provider) ?? 0)
        providerRow.onChange = { [weak self] index in
            guard let self, kinds.indices.contains(index), kinds[index] != bot.provider else { return }
            // A new provider starts on its default model and thinking level.
            self.store.setBotRuntime(bot.id, provider: kinds[index], model: nil, thinking: nil)
        }

        let models = bot.provider.models
        let modelItems = ["Default (\(models.first?.label ?? ""))"] + models.map(\.label)
        let selectedModel = bot.model.flatMap { id in models.firstIndex { $0.id == id } }.map { $0 + 1 } ?? 0
        let modelRow = PopUpRow(key: "Model", items: modelItems, selected: selectedModel)
        modelRow.onChange = { [weak self] index in
            guard let self else { return }
            let model: String? = index == 0 ? nil : models[index - 1].id
            guard model != bot.model else { return }
            self.store.setBotRuntime(bot.id, provider: bot.provider, model: model, thinking: bot.thinking)
        }

        let levels = bot.provider.thinkingLevels
        let thinkingItems = ["Default"] + levels.map(\.label)
        let selectedThinking = bot.thinking.flatMap { id in levels.firstIndex { $0.id == id } }.map { $0 + 1 } ?? 0
        let thinkingRow = PopUpRow(key: "Thinking", items: thinkingItems, selected: selectedThinking)
        thinkingRow.onChange = { [weak self] index in
            guard let self else { return }
            let thinking: String? = index == 0 ? nil : levels[index - 1].id
            guard thinking != bot.thinking else { return }
            self.store.setBotRuntime(bot.id, provider: bot.provider, model: bot.model, thinking: thinking)
        }

        // What the turns here have used, and a way to shorten the context by hand.
        var usageRows: [NSView] = []
        if let usage = chat.usage {
            let context = ActionRow(key: "Context", value: usage.contextSummary, tint: .labelColor, actionTitle: "Compact")
            context.onAction = { [weak self] in self?.store.compactChat(chat.id) }
            usageRows.append(context)
            usageRows.append(KeyValueRow(key: "Spent", value: usage.spendSummary))
        }

        let runner = store.device(bot.runnerID)
        let credential = runner?.credential(for: bot.provider)
        let connected = credential?.isConnected ?? false
        let isHere = runner?.isThisDevice ?? false
        // Connected: the masked key and a Change link. Not connected: just the Connect link.
        let status = ActionRow(
            key: "Credential",
            value: connected ? (credential?.detail ?? "Connected") : (isHere ? "" : "Connect on \(runner?.name ?? "its Runner")"),
            tint: connected ? .labelColor : .secondaryLabelColor,
            actionTitle: isHere ? (connected ? "Change" : "Connect") : nil
        )
        status.onAction = { [weak self] in
            self?.presentAsSheet(ConnectProviderViewController(kind: bot.provider))
        }

        return [providerRow, modelRow, thinkingRow, status] + usageRows
    }

    @objc private func addBot() {
        onAddBot?()
    }

    @objc private func openDevice(_ sender: NSClickGestureRecognizer) {
        guard let id = sender.view?.identifier?.rawValue else { return }
        onOpenDevice?(id)
    }
}

/// Plain container whose y grows downward, for scroll views whose content is shorter than the pane.
private final class FlippedView: NSView {
    override var isFlipped: Bool { true }
}
