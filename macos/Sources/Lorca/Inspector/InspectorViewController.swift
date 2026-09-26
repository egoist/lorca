import AppKit

final class InspectorViewController: NSViewController {
    private let store = AppStore.shared

    private let scrollView = NSScrollView()
    private let column = Build.stack([], spacing: 20)
    private let participants = SectionView(title: L("Bots in this chat"))
    private let profile = SectionView(title: L("Profile"))
    private let nameRow = EditableRow(key: L("Name"), placeholder: L("Name"))
    private let descriptionRow = SummaryActionRow(key: L("Description"), value: "", actionTitle: L("Edit…"))
    private let runtime = SectionView(title: L("Runs with"))
    private let memory = SectionView(title: L("Memory"))
    private let routines = SectionView(title: L("Routines"))
    private let plugins = SectionView(title: L("Plugins"))
    private let routing = SectionView(title: L("Where turns run"))
    private let addButton = NSButton()

    private var selection: Selection?
    /// What each bot's Runner last said about its memory; refreshed when the pane opens on a
    /// chat and after every turn in it.
    private var memoryByBot: [Bot.ID: BotMemory] = [:]
    /// Why the last fetch failed: the Runner is offline, or did not answer.
    private var memoryErrors: [Bot.ID: String] = [:]
    private var memoryFetches: Set<Bot.ID> = []
    /// The bot whose plugin rows are showing, for a click on one.
    private var pluginBotID: Bot.ID?

    /// What each section last showed. The store sends events many times a turn, and a section
    /// they leave as it was keeps its rows: a new row brings new buttons, and each button sizes
    /// itself with a SwiftUI layout pass.
    private var shown: [ObjectIdentifier: [AnyHashable]] = [:]
    /// Rows kept for what they show (a bot, a Runner, a routine, a plugin), so a section that
    /// changed updates the rows it has instead of making new ones.
    private var keptRows: [String: NSView] = [:]
    /// The usage rows under Runs with, which take new values after every turn.
    private var contextRow: ActionRow?
    private var spentRow: KeyValueRow?
    private lazy var noRoutinesRow = NoteRow(
        text: L("Routines are recurring tasks this bot runs on a schedule. Ask it in chat to set one up."))
    private lazy var marketplaceRow: ActionRow = {
        let row = ActionRow(key: L("Marketplace"), value: "", tint: .secondaryLabelColor, actionTitle: L("Add from Plugins…"))
        row.onAction = { [weak self] in
            guard let self, let bot = self.pluginBotID.flatMap(self.store.bot) else { return }
            self.onOpenMarketplace?(bot.runnerID)
        }
        return row
    }()
    /// Off screen (a collapsed inspector, a closed window), the pane skips reloads and memory
    /// fetches, and catches up when it appears.
    private var isOnScreen = false
    private var isBehind = false

    var onOpenDevice: ((Device.ID) -> Void)?
    var onRemoveBot: ((Bot.ID) -> Void)?
    var onAddBot: (() -> Void)?
    /// Puts text in the chat's composer, for "Edit in chat" on a routine.
    var onComposePrompt: ((String) -> Void)?
    /// Opens the marketplace on a Runner, for the Plugins section's Add from Plugins.
    var onOpenMarketplace: ((Device.ID) -> Void)?

    override func loadView() {
        let container = NSView()

        column.orientation = .vertical
        column.alignment = .leading
        column.edgeInsets = NSEdgeInsets(top: 18, left: 16, bottom: 20, right: 16)

        addButton.title = L("Add Bot…")
        addButton.bezelStyle = .rounded
        addButton.controlSize = .regular
        addButton.target = self
        addButton.action = #selector(addBot)
        addButton.translatesAutoresizingMaskIntoConstraints = false

        nameRow.field.alignment = .right
        descriptionRow.onAction = { [weak self] in self?.editDescription() }
        profile.setRows([nameRow, descriptionRow])

        column.addArrangedSubview(participants)
        column.addArrangedSubview(addButton)
        column.addArrangedSubview(profile)
        column.addArrangedSubview(runtime)
        column.addArrangedSubview(memory)
        column.addArrangedSubview(routines)
        column.addArrangedSubview(plugins)
        column.addArrangedSubview(routing)
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
            memory.widthAnchor.constraint(equalTo: column.widthAnchor, constant: -32),
            routines.widthAnchor.constraint(equalTo: column.widthAnchor, constant: -32),
            plugins.widthAnchor.constraint(equalTo: column.widthAnchor, constant: -32),
            routing.widthAnchor.constraint(equalTo: column.widthAnchor, constant: -32),
        ])

        view = container
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        store.observe(self) { [weak self] event in
            switch event {
            case .chatChanged, .chatsChanged, .snapshotReplaced, .rosterChanged:
                self?.reload()
            case let .respondingChanged(chatID):
                // A turn ended (or started): what the bot remembers may have moved.
                guard let self, case .chat(chatID) = self.selection, !self.store.isResponding(in: chatID) else { return }
                self.refreshShownMemory()
            default:
                break
            }
        }
    }

    // The view is still hidden here when the inspector expands, so this marks the pane as on
    // screen rather than asking the view.
    override func viewWillAppear() {
        super.viewWillAppear()
        isOnScreen = true
        guard isBehind else { return }
        isBehind = false
        reload()
        refreshShownMemory()
    }

    override func viewDidDisappear() {
        super.viewDidDisappear()
        isOnScreen = false
    }

    func show(selection newSelection: Selection) {
        selection = newSelection
        reload()
        refreshShownMemory()
    }

    /// Asks for the memory of the bot whose DM is showing.
    private func refreshShownMemory() {
        guard case let .chat(chatID) = selection, let chat = store.chat(chatID), chat.isDM,
            let bot = store.bots(in: chat).first
        else { return }
        refreshMemory(of: bot.id)
    }

    /// Asks the CLI for the bot's memory and redraws the section when it answers.
    private func refreshMemory(of botID: Bot.ID) {
        guard isOnScreen else {
            isBehind = true
            return
        }
        guard !store.isMock || memoryByBot[botID] == nil, memoryFetches.insert(botID).inserted else { return }
        Task { [weak self] in
            defer { self?.memoryFetches.remove(botID) }
            do {
                let memory = try await self?.store.botMemory(botID)
                guard let self, let memory else { return }
                self.memoryByBot[botID] = memory
                self.memoryErrors[botID] = nil
                self.reload()
            } catch {
                guard let self else { return }
                self.memoryErrors[botID] = error.localizedDescription
                self.reload()
            }
        }
    }

    /// Brings each section in line with the store. A section whose state is what it last showed
    /// is left alone, and one that changed reconfigures the rows it keeps.
    func reload() {
        guard isViewLoaded, case let .chat(chatID) = selection, let chat = store.chat(chatID) else {
            return
        }
        guard isOnScreen else {
            isBehind = true
            return
        }

        let members = store.bots(in: chat)
        showParticipants(members, in: chat)

        // A DM never takes another bot; a group does until it is full or every bot is in it.
        let canAdd = chat.canAddBot && members.count < store.bots.count
        if addButton.isHidden != chat.isDM { addButton.isHidden = chat.isDM }
        if addButton.isEnabled != canAdd { addButton.isEnabled = canAdd }

        // A direct chat is one bot, so its profile, provider, and model are edited right here.
        let single = chat.isDM && members.count == 1
        for section in [profile, runtime, memory, routines, plugins] where section.isHidden == single {
            section.isHidden = !single
        }
        if single, let bot = members.first {
            showProfile(of: bot)
            showRuntime(of: bot, in: chat)
            showMemory(of: bot)
            showRoutines(of: bot)
            showPlugins(of: bot)
        }
        showRouting(members)
    }

    /// Whether `state` differs from what `section` last showed; records it when it does.
    private func changed(_ section: SectionView, to state: [AnyHashable]) -> Bool {
        let key = ObjectIdentifier(section)
        guard shown[key] != state else { return false }
        shown[key] = state
        return true
    }

    /// The row kept under `key`, made the first time it is asked for.
    private func keptRow<Row: NSView>(_ key: String, make: () -> Row) -> Row {
        if let row = keptRows[key] as? Row { return row }
        let row = make()
        keptRows[key] = row
        return row
    }

    private func showParticipants(_ members: [Bot], in chat: Chat) {
        let hosts = members.map { store.device($0.runnerID)?.name ?? L("unassigned") }
        let looks = members.map { AvatarView.content(for: $0) }
        guard changed(participants, to: [members, looks, hosts, chat.canRemoveBot]) else { return }
        participants.setRows(
            zip(members, hosts).map { bot, host in
                let row = keptRow("bot:\(bot.id)") { BotRow() }
                row.configure(
                    bot: bot,
                    detailText: "\(bot.runtimeLabel) · \(host)",
                    accessorySymbol: chat.canRemoveBot ? "minus.circle" : nil,
                    tooltip: L("Remove from chat")
                )
                row.onAccessory = { [weak self] in self?.onRemoveBot?(bot.id) }
                // The avatar is the way to a bot's look: symbol, color, or an image.
                row.onAvatarClick = { [weak self] in self?.presentAsSheet(BotLookViewController(botID: bot.id)) }
                return row
            })
    }

    private func showProfile(of bot: Bot) {
        // The Name row keeps what the user is typing, and puts the name back after.
        nameRow.setValue(bot.name)
        guard changed(profile, to: [bot.id, bot.description]) else { return }
        descriptionRow.setValue(bot.description)
        nameRow.onCommit = { [weak self] in self?.commitProfile(of: bot.id) }
    }

    private func showRuntime(of bot: Bot, in chat: Chat) {
        let credential = store.credential(for: bot.provider)
        if changed(runtime, to: [chat.id, bot.id, bot.harness, bot.codexOptions, bot.runnerID, bot.provider, bot.model, bot.thinking, credential, chat.usage == nil]) {
            runtime.setRows(runtimeRows(for: bot, in: chat))
        }
        // What the turns used changes after every turn; the rows take the new values in place.
        if let usage = chat.usage {
            contextRow?.setValue(usage.contextSummary)
            spentRow?.setValue(usage.spendSummary)
        }
    }

    private func showMemory(of bot: Bot) {
        // The bot's memory, or why it could not be read, or whether it is on its way.
        let state: [AnyHashable] =
            if let known = memoryByBot[bot.id] { [bot.id, known] }
            else if let error = memoryErrors[bot.id] { [bot.id, error] }
            else { [bot.id, memoryFetches.contains(bot.id)] }
        guard changed(memory, to: state) else { return }
        memory.setRows(memoryRows(for: bot))
    }

    private func showRouting(_ members: [Bot]) {
        let hosts = Dictionary(grouping: members, by: \.runnerID)
        let runners = hosts.keys.sorted().compactMap { store.device($0) }
        let botNames = runners.map { runner in (hosts[runner.id] ?? []).map(\.name).joined(separator: ", ") }
        let states = runners.map { $0.status == .online ? L("Online") : Format.lastSeen($0.lastSeen) }
        guard changed(routing, to: [runners, botNames, states]) else { return }
        routing.setRows(
            runners.indices.map { index in
                let runner = runners[index]
                // A label keeps the vibrancy it had when it went into the window, and one turned
                // from Online's green to the gray of Last seen draws too dark; an online and an
                // offline Runner each get a row of their own.
                let online = runner.status == .online
                let row: StatusRow = keptRow("runner:\(runner.id):\(online)") {
                    let row = StatusRow()
                    row.addGestureRecognizer(NSClickGestureRecognizer(target: self, action: #selector(openDevice(_:))))
                    row.identifier = NSUserInterfaceItemIdentifier(runner.id)
                    return row
                }
                row.configure(
                    symbol: runner.symbolName,
                    title: runner.name,
                    subtitle: botNames[index],
                    state: states[index],
                    stateColor: online ? .systemGreen : .secondaryLabelColor
                )
                return row
            })
    }

    /// Saves the compact Name row when it finishes editing. An emptied value keeps the old one;
    /// Description has its own sheet.
    private func commitProfile(of id: Bot.ID) {
        guard let bot = store.bot(id) else { return }
        let name = nameRow.value.isEmpty ? bot.name : nameRow.value
        nameRow.setValue(name)
        guard name != bot.name else { return }
        store.updateBot(id, name: name)
    }

    private func editDescription() {
        guard case let .chat(chatID) = selection, let chat = store.chat(chatID), chat.isDM,
            let bot = store.bots(in: chat).first
        else { return }
        presentAsSheet(BotDescriptionViewController(botID: bot.id))
    }

    private func runtimeRows(for bot: Bot, in chat: Chat) -> [NSView] {
        let harnesses = Bot.Harness.allCases
        let harnessRow = PopUpRow(key: L("Runtime"), items: harnesses.map(\.title), selected: harnesses.firstIndex(of: bot.harness) ?? 0)
        harnessRow.onChange = { [weak self] index in
            guard harnesses.indices.contains(index) else { return }
            self?.store.setBotHarness(bot.id, harness: harnesses[index])
        }
        if bot.harness == .codex {
            contextRow = nil
            spentRow = nil
            let settings = CodexSettingsView(runnerID: bot.runnerID, botID: bot.id,
                selection: .init(model: bot.model, thinking: bot.thinking, options: bot.codexOptions))
            settings.onChange = { [weak self] selection in self?.store.setCodexOptions(bot.id, selection: selection) }
            return [harnessRow, settings]
        }
        let kinds = ProviderCredential.Kind.allCases
        let providerRow = PopUpRow(
            key: L("Provider"),
            items: kinds.map(\.rawValue),
            selected: kinds.firstIndex(of: bot.provider) ?? 0)
        providerRow.onChange = { [weak self] index in
            guard let self, kinds.indices.contains(index), kinds[index] != bot.provider else { return }
            // A new provider starts on its default model and thinking level.
            self.store.setBotRuntime(bot.id, provider: kinds[index], model: nil, thinking: nil)
        }

        let models = bot.provider.models
        let modelItems = [L("Default (%@)", models.first?.label ?? "")] + models.map(\.label)
        let selectedModel = bot.model.flatMap { id in models.firstIndex { $0.id == id } }.map { $0 + 1 } ?? 0
        let modelRow = PopUpRow(key: L("Model"), items: modelItems, selected: selectedModel)
        modelRow.onChange = { [weak self] index in
            guard let self else { return }
            let model: String? = index == 0 ? nil : models[index - 1].id
            guard model != bot.model else { return }
            self.store.setBotRuntime(bot.id, provider: bot.provider, model: model, thinking: bot.thinking)
        }

        let levels = bot.provider.thinkingLevels
        let thinkingItems = [L("Default")] + levels.map(\.label)
        let selectedThinking = bot.thinking.flatMap { id in levels.firstIndex { $0.id == id } }.map { $0 + 1 } ?? 0
        let thinkingRow = PopUpRow(key: L("Thinking"), items: thinkingItems, selected: selectedThinking)
        thinkingRow.onChange = { [weak self] index in
            guard let self else { return }
            let thinking: String? = index == 0 ? nil : levels[index - 1].id
            guard thinking != bot.thinking else { return }
            self.store.setBotRuntime(bot.id, provider: bot.provider, model: bot.model, thinking: thinking)
        }

        // What the turns here have used, and a way to shorten the context by hand.
        var usageRows: [NSView] = []
        contextRow = nil
        spentRow = nil
        if let usage = chat.usage {
            let context = ActionRow(key: L("Context"), value: usage.contextSummary, tint: .labelColor, actionTitle: L("Compact"))
            context.onAction = { [weak self] in self?.store.compactChat(chat.id) }
            let spent = KeyValueRow(key: L("Spent"), value: usage.spendSummary)
            contextRow = context
            spentRow = spent
            usageRows = [context, spent]
        }

        let credential = store.credential(for: bot.provider)
        let connected = credential?.isConnected ?? false
        // Connected: the masked key and a Change link. Not connected: just the Connect link.
        let status = ActionRow(
            key: L("Credential"),
            value: connected ? (credential?.detail ?? L("Connected")) : "",
            tint: connected ? .labelColor : .secondaryLabelColor,
            actionTitle: connected ? L("Change") : L("Connect")
        )
        status.onAction = { [weak self] in
            guard let self else { return }
            ConnectProviderViewController.present(kind: bot.provider, from: self)
        }

        return [harnessRow, providerRow, modelRow, thinkingRow, status] + usageRows
    }

    /// What the bot remembers, as its Runner reports it: the index against its load budget with
    /// an editor, and the folder of topic files and daily logs. A bot on another Runner is
    /// read and edited through the relay; only the folder cannot be opened from here.
    private func memoryRows(for bot: Bot) -> [NSView] {
        guard let memory = memoryByBot[bot.id] else {
            if let error = memoryErrors[bot.id] {
                let row = ActionRow(key: L("Notes"), value: error, tint: .secondaryLabelColor, actionTitle: L("Retry"))
                row.onAction = { [weak self] in self?.refreshMemory(of: bot.id) }
                return [row]
            }
            return [KeyValueRow(key: L("Notes"), value: memoryFetches.contains(bot.id) ? L("Loading…") : "", tint: .secondaryLabelColor)]
        }
        let notes = ActionRow(
            key: L("Notes"),
            value: memory.budgetSummary,
            tint: memory.truncated ? .systemOrange : .labelColor,
            actionTitle: L("Edit…")
        )
        notes.toolTip = memory.truncated
            ? L("Only the first %d lines or %@ open each turn; the rest is not read.", memory.maxLines, Format.kilobytes(memory.maxBytes))
            : L("MEMORY.md opens at the start of every turn.")
        notes.onAction = { [weak self] in
            // The rows outlive a rename, so the sheet takes the bot as it is now.
            guard let self, let bot = self.store.bot(bot.id) else { return }
            let editor = MemoryViewController(bot: bot, memory: memory)
            editor.onSaved = { [weak self] in self?.refreshMemory(of: bot.id) }
            self.presentAsSheet(editor)
        }
        let folder = ActionRow(
            key: L("Folder"),
            value: memory.here ? memory.filesSummary : L("%@ · on %@", memory.filesSummary, memory.runner),
            tint: .secondaryLabelColor,
            actionTitle: memory.here ? L("Show") : nil
        )
        folder.toolTip = memory.path
        folder.onAction = {
            let url = URL(fileURLWithPath: memory.path).appendingPathComponent("MEMORY.md")
            NSWorkspace.shared.activateFileViewerSelecting([url])
        }
        return [notes, folder]
    }

    /// The bot's routines, after Grok Bot's panel: a row per routine with a pause switch, and
    /// the details in a sheet. With none, the sentence that says how to get one.
    private func showRoutines(of bot: Bot) {
        let mine = store.routines(for: bot.id)
        guard changed(routines, to: [bot.id, mine, mine.map(\.detail)]) else { return }
        guard !mine.isEmpty else {
            routines.setRows([noRoutinesRow])
            return
        }
        routines.setRows(
            mine.map { routine in
                let row = keptRow("routine:\(routine.id)") { SwitchRow() }
                row.configure(routine: routine)
                row.onToggle = { [weak self] enabled in self?.store.setRoutineEnabled(routine.id, enabled) }
                row.onClick = { [weak self] in
                    guard let self, let bot = self.store.bot(bot.id) else { return }
                    let sheet = RoutineViewController(routineID: routine.id, bot: bot)
                    sheet.onEditInChat = { [weak self] text in self?.onComposePrompt?(text) }
                    self.presentAsSheet(sheet)
                }
                return row
            })
    }

    /// The plugins the bot's Runner has, which every bot there may use, and a way to the
    /// marketplace. A plugin that needs setup says so; clicking opens it.
    private func showPlugins(of bot: Bot) {
        let runner = store.device(bot.runnerID)
        guard changed(plugins, to: [bot.id, bot.name, runner?.id, runner?.name, runner?.plugins]) else { return }
        pluginBotID = bot.id
        var rows: [NSView] = (runner?.plugins ?? []).map { plugin in
            let row: StatusRow = keptRow("plugin:\(plugin.id)") {
                let row = StatusRow()
                row.identifier = NSUserInterfaceItemIdentifier(plugin.id)
                row.addGestureRecognizer(NSClickGestureRecognizer(target: self, action: #selector(openPlugin(_:))))
                return row
            }
            row.configure(plugin: plugin)
            row.toolTip = L("Open %@", plugin.name)
            return row
        }
        if rows.isEmpty {
            let runnerName = runner?.name ?? L("its Runner")
            rows.append(NoteRow(text: L("No plugins on %@ yet. Add one from the marketplace, or ask %@ to find one.", runnerName, bot.name)))
        }
        rows.append(marketplaceRow)
        plugins.setRows(rows)
    }

    @objc private func openPlugin(_ sender: NSClickGestureRecognizer) {
        guard let id = sender.view?.identifier?.rawValue, let bot = pluginBotID.flatMap(store.bot), let runner = store.device(bot.runnerID) else { return }
        presentAsSheet(PluginViewController(pluginID: id, runner: runner, bot: bot))
    }

    @objc private func addBot() {
        onAddBot?()
    }

    @objc private func openDevice(_ sender: NSClickGestureRecognizer) {
        guard let id = sender.view?.identifier?.rawValue else { return }
        onOpenDevice?(id)
    }
}
