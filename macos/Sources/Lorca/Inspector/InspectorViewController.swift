import AppKit
import UniformTypeIdentifiers

final class InspectorViewController: NSViewController {
    private let store = AppStore.shared

    private let scrollView = NSScrollView()
    private let column = Build.stack([], spacing: 20)
    private let participants = SectionView(title: L("Bots in this chat"))
    private let group = SectionView(title: L("Group"))
    private let groupNameRow = EditableRow(key: L("Name"), placeholder: "")
    private let groupDescriptionRow = SummaryActionRow(key: L("Description"), value: "", actionTitle: L("Edit…"))
    private let project = SectionView(title: L("Project"))
    private let profile = SectionView(title: L("Profile"))
    private let nameRow = EditableRow(key: L("Name"), placeholder: L("Name"))
    private let descriptionRow = SummaryActionRow(key: L("Description"), value: "", actionTitle: L("Edit…"))
    private let accessRow = DisclosureRow(key: L("Access"))
    private let runtime = SectionView(title: L("Runs with"))
    private let memory = SectionView(title: L("Memory"))
    private let skills = SectionView(title: L("Skills"))
    private let routines = SectionView(title: L("Routines"))
    private let feedback = SectionView(title: L("Feedback"))
    private let reviews = SectionView(title: L("Waiting for review"))
    private let tasks = SectionView(title: L("Tasks"))
    private let plugins = SectionView(title: L("Plugins"))
    private let routing = SectionView(title: L("Where turns run"))
    private let outputs = SectionView(title: L("Outputs"))
    private lazy var allOutputsButton = ViewAllLabel(L("View all")) { [weak self] in self?.showAllOutputs() }
    private let addButton = NSButton()

    private var selection: Selection?
    /// What each bot's Runner last said about its memory; refreshed when the pane opens on a
    /// chat and after every turn in it.
    private var memoryByBot: [Bot.ID: BotMemory] = [:]
    /// Why the last fetch failed: the Runner is offline, or did not answer.
    private var memoryErrors: [Bot.ID: String] = [:]
    private var memoryFetches: Set<Bot.ID> = []
    /// What each bot's Runner last said about its feedback; refreshed with its memory and when
    /// the Runner says it changed.
    private var feedbackByBot: [Bot.ID: BotFeedback] = [:]
    private var feedbackFetches: Set<Bot.ID> = []
    /// The bot whose plugin rows are showing, for a click on one.
    private var pluginBotID: Bot.ID?
    /// Each group's project context as the CLI last listed it; fetched when the group shows and
    /// again when it changes.
    private var projectContexts: [Chat.ID: ProjectContext] = [:]
    private var projectFetches: Set<Chat.ID> = []
    /// Groups whose context changed while a fetch was on its way, to ask again once it lands.
    private var projectChangedMeanwhile: Set<Chat.ID> = []
    /// The group whose Project section shows every entry rather than the first few.
    private var projectShowingAll: Chat.ID?
    /// Whose skills are showing, for + and a click on one, and whose show every row.
    private var skillScope: PlaybookScope?
    private var skillsShowingAll: PlaybookScope?

    /// What each section last showed. The store sends events many times a turn, and a section
    /// they leave as it was keeps its rows: a new row brings new buttons, and each button sizes
    /// itself with a SwiftUI layout pass.
    private var shown: [ObjectIdentifier: [AnyHashable]] = [:]
    /// Rows kept for what they show (a bot, a Runner, a routine, a plugin), so a section that
    /// changed updates the rows it has instead of making new ones.
    private var keptRows: [String: NSView] = [:]
    /// The chat whose Tasks section shows every task rather than the first few.
    private var tasksShowingAll: Chat.ID?
    /// The usage rows under Runs with, which take new values after every turn.
    private var contextRow: ActionRow?
    private var spentRow: KeyValueRow?
    private var limitsRow: DisclosureRow?
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

    /// Opens a chat on one of its messages, for feedback's Show in Chat.
    var onShowMessage: ((Chat.ID, Message.ID) -> Void)?
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
        profile.setRows([nameRow, descriptionRow, accessRow])
        skills.setHeaderAccessory(HoverButton(symbol: "plus", pointSize: 11, tooltip: L("New Skill"), target: self, action: #selector(newSkill)))
        skills.isHidden = true
        groupNameRow.field.alignment = .right
        groupDescriptionRow.onAction = { [weak self] in self?.editGroupDescription() }
        group.setRows([groupNameRow, groupDescriptionRow])
        project.setHeaderAccessory(
            HoverButton(symbol: "plus", pointSize: 11, tooltip: L("Add to Project"), target: self, action: #selector(showProjectMenu(_:))))
        tasks.setHeaderAccessory(HoverButton(symbol: "plus", pointSize: 11, tooltip: L("New Task"), target: self, action: #selector(newTask)))
        tasks.isHidden = true
        outputs.isHidden = true

        column.addArrangedSubview(participants)
        column.addArrangedSubview(addButton)
        column.addArrangedSubview(group)
        column.addArrangedSubview(project)
        column.addArrangedSubview(reviews)
        column.addArrangedSubview(outputs)
        column.addArrangedSubview(profile)
        column.addArrangedSubview(runtime)
        column.addArrangedSubview(memory)
        column.addArrangedSubview(skills)
        column.addArrangedSubview(routines)
        column.addArrangedSubview(feedback)
        column.addArrangedSubview(tasks)
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
            group.widthAnchor.constraint(equalTo: column.widthAnchor, constant: -32),
            project.widthAnchor.constraint(equalTo: column.widthAnchor, constant: -32),
            outputs.widthAnchor.constraint(equalTo: column.widthAnchor, constant: -32),
            profile.widthAnchor.constraint(equalTo: column.widthAnchor, constant: -32),
            runtime.widthAnchor.constraint(equalTo: column.widthAnchor, constant: -32),
            memory.widthAnchor.constraint(equalTo: column.widthAnchor, constant: -32),
            skills.widthAnchor.constraint(equalTo: column.widthAnchor, constant: -32),
            routines.widthAnchor.constraint(equalTo: column.widthAnchor, constant: -32),
            feedback.widthAnchor.constraint(equalTo: column.widthAnchor, constant: -32),
            reviews.widthAnchor.constraint(equalTo: column.widthAnchor, constant: -32),
            tasks.widthAnchor.constraint(equalTo: column.widthAnchor, constant: -32),
            plugins.widthAnchor.constraint(equalTo: column.widthAnchor, constant: -32),
            routing.widthAnchor.constraint(equalTo: column.widthAnchor, constant: -32),
        ])

        view = container
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        store.observe(self) { [weak self] event in
            switch event {
            case .chatChanged, .chatsChanged, .snapshotReplaced, .rosterChanged, .reviewsChanged, .durableTasksChanged, .budgetsChanged:
                self?.reload()
            case let .feedbackChanged(botID):
                guard let self else { return }
                if self.shownBot?.id == botID { self.refreshFeedback(of: botID) } else { self.feedbackByBot[botID] = nil }
            case let .projectContextChanged(chatID):
                guard let self, case .chat(chatID) = self.selection else { return }
                self.refreshProject(of: chatID)
            case let .outputsChanged(chatID):
                guard let self, case .chat(chatID) = self.selection else { return }
                self.reload()
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
        refreshShownProject()
    }

    override func viewDidDisappear() {
        super.viewDidDisappear()
        isOnScreen = false
    }

    func show(selection newSelection: Selection) {
        selection = newSelection
        reload()
        refreshShownMemory()
        refreshShownProject()
    }

    /// The bot whose DM is showing.
    private var shownBot: Bot? {
        guard case let .chat(chatID) = selection, let chat = store.chat(chatID), chat.isDM else { return nil }
        return store.bots(in: chat).first
    }

    /// Asks for the memory and the feedback of the bot whose DM is showing.
    private func refreshShownMemory() {
        guard let bot = shownBot else { return }
        refreshMemory(of: bot.id)
        refreshFeedback(of: bot.id)
    }

    /// Asks the bot's Runner for its feedback. The section stays out while there is none, or
    /// while the Runner cannot be asked.
    private func refreshFeedback(of botID: Bot.ID) {
        guard isOnScreen else {
            isBehind = true
            return
        }
        guard feedbackFetches.insert(botID).inserted else { return }
        Task { [weak self] in
            defer { self?.feedbackFetches.remove(botID) }
            guard let feedback = try? await self?.store.feedback(of: botID), let self else { return }
            self.feedbackByBot[botID] = feedback
            self.reload()
        }
    }

    /// Asks for the project context of the group that is showing.
    private func refreshShownProject() {
        guard case let .chat(chatID) = selection, store.chat(chatID)?.isGroup == true else { return }
        refreshProject(of: chatID)
    }

    /// Asks the CLI for the group's project context and redraws the section when it answers. A
    /// change while a fetch is on its way asks again once it lands.
    private func refreshProject(of chatID: Chat.ID) {
        guard isOnScreen else {
            isBehind = true
            return
        }
        guard projectFetches.insert(chatID).inserted else {
            projectChangedMeanwhile.insert(chatID)
            return
        }
        Task { [weak self] in
            let context = try? await self?.store.projectContext(chatID)
            guard let self else { return }
            self.projectFetches.remove(chatID)
            if let context { self.projectContexts[chatID] = context }
            self.reload()
            if self.projectChangedMeanwhile.remove(chatID) != nil { self.refreshProject(of: chatID) }
        }
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
        // Add Bot sits close under the bots; without it the next section keeps the usual gap.
        column.setCustomSpacing(chat.isDM ? column.spacing : 10, after: participants)
        if addButton.isEnabled != canAdd { addButton.isEnabled = canAdd }

        // A group's name and what it is for.
        if group.isHidden == chat.isGroup { group.isHidden = !chat.isGroup }
        if chat.isGroup { showGroup(chat, members: members) }
        if project.isHidden == chat.isGroup { project.isHidden = !chat.isGroup }
        if chat.isGroup { showProject(of: chat) }
        showOutputs(in: chat)

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
            showFeedback(of: bot)
            showPlugins(of: bot)
        } else if !feedback.isHidden {
            feedback.isHidden = true
        }
        if let scope = Self.skillScope(of: chat, members: members) {
            showSkills(of: scope)
        } else if !skills.isHidden {
            skills.isHidden = true
        }
        showRouting(members)
        showReviews(in: chat)
        showTasks(in: chat)
    }

    /// What the chat's bots left for the user to approve, oldest first, while any waits or runs;
    /// a row opens it. How each ended stays in the chat, so the section goes once none is open.
    private func showReviews(in chat: Chat) {
        let items = store.reviews.filter { $0.origin.chatId == chat.id && $0.isOpen }.sorted { $0.createdAt < $1.createdAt }
        let runner = items.first.flatMap { store.device($0.runnerId) }
        guard changed(reviews, to: [chat.id, items.map { "\($0.id):\($0.revision)" }, runner?.plugins]) else { return }
        if reviews.isHidden != items.isEmpty { reviews.isHidden = items.isEmpty }
        reviews.setRows(
            items.map { item in
                let row: StatusRow = keptRow("review:\(item.id)") {
                    let row = StatusRow()
                    row.identifier = NSUserInterfaceItemIdentifier(item.id)
                    row.addGestureRecognizer(NSClickGestureRecognizer(target: self, action: #selector(openReview(_:))))
                    return row
                }
                let plugin = item.payload.pluginId.flatMap { id in store.device(item.runnerId)?.plugins.first { $0.id == id } }
                row.configure(
                    symbol: item.payload.isDraft ? "doc.text" : (item.payload.isShell ? "terminal" : plugin?.symbolName ?? "puzzlepiece.extension"),
                    image: plugin.flatMap { PluginLogo.tile(for: $0.marketplaceID, size: 18) },
                    title: item.payload.kind == "plugin" ? store.pluginName(of: item) : item.headline,
                    subtitle: item.rationale,
                    state: item.stateText,
                    subtitleLines: 2)
                row.toolTip = item.rationale
                return row
            })
    }

    @objc private func openReview(_ sender: NSClickGestureRecognizer) {
        guard let id = sender.view?.identifier?.rawValue, let item = store.review(id) else { return }
        presentAsSheet(ReviewViewController(item: item))
    }


    /// The chat's durable tasks, open work first; hidden while it has none. A row opens the
    /// task; the title's + starts a new one. Past five rows the rest wait behind Show All.
    private func showTasks(in chat: Chat) {
        let records = store.tasks(in: chat.id)
        let showsAll = tasksShowingAll == chat.id
        guard changed(tasks, to: [chat.id, chat.isGroup, records, showsAll, records.map { store.bot($0.ownerBotId)?.name }]) else { return }
        if tasks.isHidden != records.isEmpty { tasks.isHidden = records.isEmpty }
        let limit = 5
        let shown = showsAll || records.count <= limit ? records : Array(records.prefix(limit - 1))
        var rows: [NSView] = shown.map { task in
            let row = keptRow("task:\(task.id)") { SwitchRow() }
            var detail = task.state.title
            if chat.isGroup, let owner = store.bot(task.ownerBotId) { detail += " · \(owner.name)" }
            row.configure(symbol: task.state.symbol, tint: task.state.tint, title: task.goal, detail: detail, tooltip: task.goal)
            row.onClick = { [weak self] in self?.openTask(task.id, in: chat.id) }
            return row
        }
        if shown.count < records.count {
            let more = keptRow("tasks:all") { SwitchRow() }
            more.configure(symbol: "ellipsis", tint: .tertiaryLabelColor, title: L("Show %d More", records.count - shown.count), detail: "", tooltip: "")
            more.onClick = { [weak self] in
                self?.tasksShowingAll = chat.id
                self?.reload()
            }
            rows.append(more)
        }
        tasks.setRows(rows)
    }

    private func openTask(_ id: String, in chatID: Chat.ID) {
        presentAsSheet(DurableTaskViewController(chatID: chatID, task: store.durableTask(id)))
    }

    @objc private func newTask() {
        guard case let .chat(chatID) = selection else { return }
        presentAsSheet(DurableTaskViewController(chatID: chatID, task: nil))
    }

    /// Whose skills a chat shows: the bot's in a DM, the group's in a group.
    static func skillScope(of chat: Chat, members: [Bot]) -> PlaybookScope? {
        if chat.isGroup { return .group(chat.id) }
        return members.first.map { .bot($0.id) }
    }

    /// The skills, hidden while there are none: past five rows the rest wait behind Show More. A
    /// row opens its skill, and its menu exports or deletes it; + adds one.
    private func showSkills(of scope: PlaybookScope) {
        let list = store.skills(in: scope)
        let showsAll = skillsShowingAll == scope
        guard changed(skills, to: [scope, list, showsAll]) else { return }
        skillScope = scope
        if skills.isHidden != list.isEmpty { skills.isHidden = list.isEmpty }
        let shown = showsAll || list.count <= 5 ? list : Array(list.prefix(4))
        var rows: [NSView] = shown.map { skill in
            let row: StatusRow = keptRow("skill:\(skill.id)") {
                let row = StatusRow()
                row.identifier = NSUserInterfaceItemIdentifier(skill.id)
                row.addGestureRecognizer(NSClickGestureRecognizer(target: self, action: #selector(openSkill(_:))))
                return row
            }
            row.configure(skill: skill)
            row.menu = skillMenu(for: skill)
            return row
        }
        if shown.count < list.count {
            let more: StatusRow = keptRow("skills:more") {
                let row = StatusRow()
                row.addGestureRecognizer(NSClickGestureRecognizer(target: self, action: #selector(showAllSkills)))
                return row
            }
            more.configure(symbol: "ellipsis.circle", title: L("Show %d More", list.count - shown.count), subtitle: "", state: nil)
            rows.append(more)
        }
        skills.setRows(rows)
    }

    private func skillMenu(for skill: PlaybookSummary) -> NSMenu {
        let menu = NSMenu()
        let open = NSMenuItem(title: L("Open"), action: #selector(openSkillItem(_:)), keyEquivalent: "")
        let export = NSMenuItem(title: L("Export…"), action: #selector(exportSkill(_:)), keyEquivalent: "")
        export.isEnabled = !skill.isDraft
        let delete = NSMenuItem(title: L("Delete…"), action: #selector(deleteSkill(_:)), keyEquivalent: "")
        for item in [open, export, delete] {
            item.target = self
            item.representedObject = skill
        }
        menu.autoenablesItems = false
        menu.items = [open, export, .separator(), delete]
        return menu
    }

    @objc private func newSkill() {
        guard let skillScope else { return }
        presentAsSheet(PlaybookViewController(scope: skillScope))
    }

    @objc private func showAllSkills() {
        skillsShowingAll = skillScope
        reload()
    }

    @objc private func openSkill(_ sender: NSClickGestureRecognizer) {
        guard let id = sender.view?.identifier?.rawValue, let skill = store.playbooks.first(where: { $0.id == id }) else { return }
        PlaybookViewController.open(skill, from: self)
    }

    @objc private func openSkillItem(_ sender: NSMenuItem) {
        guard let skill = sender.representedObject as? PlaybookSummary else { return }
        PlaybookViewController.open(skill, from: self)
    }

    @objc private func exportSkill(_ sender: NSMenuItem) {
        guard let skill = sender.representedObject as? PlaybookSummary, let window = view.window else { return }
        Task { [weak self] in
            do {
                let data = try await AppStore.shared.exportPlaybook(skill.id, in: skill.scope)
                let panel = NSSavePanel()
                panel.nameFieldStringValue = skill.name + ".json"
                panel.allowedContentTypes = [.json]
                guard await panel.beginSheetModal(for: window) == .OK, let url = panel.url else { return }
                try data.write(to: url, options: .atomic)
            } catch {
                self?.showSkillError(L("Couldn't export the skill"), error)
            }
        }
    }

    @objc private func deleteSkill(_ sender: NSMenuItem) {
        guard let skill = sender.representedObject as? PlaybookSummary, let window = view.window else { return }
        let alert = NSAlert()
        alert.messageText = L("Delete “%@”?", skill.name)
        alert.informativeText = skill.isDraft ? L("The draft is deleted.") : L("Your bots stop using this skill. This can't be undone.")
        alert.addButton(withTitle: L("Delete"))
        alert.addButton(withTitle: L("Cancel"))
        alert.alertStyle = .warning
        alert.beginSheetModal(for: window) { [weak self] response in
            guard response == .alertFirstButtonReturn else { return }
            Task { [weak self] in
                do {
                    // The version the row shows: a skill edited elsewhere since is not deleted.
                    let record = try await AppStore.shared.playbook(skill.id, in: skill.scope)
                    guard record.revision == skill.revision else {
                        throw CLIClient.RequestError(message: L("This skill changed on another Device. Open it to see what changed."))
                    }
                    try await AppStore.shared.removePlaybook(record)
                } catch {
                    self?.showSkillError(L("Couldn't delete the skill"), error)
                }
            }
        }
    }

    private func showSkillError(_ message: String, _ error: Error) {
        guard let window = view.window else { return }
        let alert = NSAlert()
        alert.messageText = message
        alert.informativeText = error.localizedDescription
        alert.beginSheetModal(for: window, completionHandler: nil)
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
        guard changed(participants, to: [members, looks, hosts, chat.canRemoveBot, chat.owner]) else { return }
        participants.setRows(
            zip(members, hosts).map { bot, host in
                let row = keptRow("bot:\(bot.id)") { BotRow() }
                let isOwner = chat.owner == bot.id
                row.configure(
                    bot: bot,
                    detailText: (isOwner ? "\(L("Owner")) · " : "") + "\(bot.provider.name) · \(host)",
                    accessorySymbol: chat.canRemoveBot ? "minus.circle" : nil,
                    tooltip: L("Remove from chat")
                )
                row.onAccessory = { [weak self] in self?.onRemoveBot?(bot.id) }
                // The avatar is the way to a bot's look: symbol, color, or an image.
                row.onAvatarClick = { [weak self] in self?.presentAsSheet(BotLookViewController(botID: bot.id)) }
                // In a group of several, a click or a right-click on a member offers to make it
                // the owner.
                let menu = chat.isGroup && members.count > 1 ? memberMenu(for: bot.id, in: chat) : nil
                row.menu = menu
                row.onClick = menu.map { menu in
                    { [weak row] in
                        guard let row, let event = NSApp.currentEvent else { return }
                        NSMenu.popUpContextMenu(menu, with: event, for: row)
                    }
                }
                return row
            })
    }

    private func memberMenu(for botID: Bot.ID, in chat: Chat) -> NSMenu {
        let menu = NSMenu()
        let owner = NSMenuItem(title: L("Make Owner"), action: #selector(makeOwner(_:)), keyEquivalent: "")
        owner.target = self
        owner.representedObject = botID
        owner.state = chat.owner == botID ? .on : .off
        menu.addItem(owner)
        if chat.canRemoveBot {
            menu.addItem(.separator())
            let remove = NSMenuItem(title: L("Remove from Chat"), action: #selector(removeMember(_:)), keyEquivalent: "")
            remove.target = self
            remove.representedObject = botID
            menu.addItem(remove)
        }
        return menu
    }

    @objc private func makeOwner(_ sender: NSMenuItem) {
        guard case let .chat(chatID) = selection, let botID = sender.representedObject as? Bot.ID else { return }
        store.setOwner(botID, of: chatID)
    }

    @objc private func removeMember(_ sender: NSMenuItem) {
        guard let botID = sender.representedObject as? Bot.ID else { return }
        onRemoveBot?(botID)
    }

    private func showGroup(_ chat: Chat, members: [Bot]) {
        // The Name row keeps what the user is typing, and puts the name back after. Without a
        // name of its own, a group goes by its members' names.
        groupNameRow.field.placeholderString = members.map(\.name).joined(separator: ", ")
        groupNameRow.setValue(chat.customTitle ?? "")
        guard changed(group, to: [chat.id, chat.groupDescription]) else { return }
        groupDescriptionRow.setValue(chat.groupDescription)
        groupNameRow.onCommit = { [weak self] in self?.commitGroupName(of: chat.id) }
    }

    /// What every bot in the group can read: an entry a row, which opens it, the briefs and
    /// decisions first. Past five rows the rest wait behind Show More. The title's + adds one;
    /// with none yet, the title and its + are all the section shows.
    private func showProject(of chat: Chat) {
        let context = projectContexts[chat.id] ?? ProjectContext()
        let showsAll = projectShowingAll == chat.id
        guard changed(project, to: [chat.id, context.entries, context.conflicts, showsAll]) else { return }
        let entries = context.entries
        let shown = showsAll || entries.count <= 5 ? entries : Array(entries.prefix(4))
        var rows: [NSView] = shown.map { entry in
            // What needs the user is said after the kind, and marked in orange at the end; a
            // title keeps the row's width.
            let problem =
                !context.otherVersions(of: entry.id).isEmpty ? L("Two versions")
                : entry.freshness == "unavailable" ? L("Unavailable")
                : nil
            let detail = problem ?? (entry.isSuggestion ? L("Suggested by %@", entry.source.label) : entry.host)
            // A label keeps the vibrancy it had when it went into the window, so a row that gains
            // or loses its orange mark is a new row.
            let row: StatusRow = keptRow("project:\(entry.id):\(problem != nil)") {
                let row = StatusRow()
                row.identifier = NSUserInterfaceItemIdentifier(entry.id)
                row.addGestureRecognizer(NSClickGestureRecognizer(target: self, action: #selector(openProjectEntry(_:))))
                return row
            }
            row.configure(
                symbol: entry.kind.symbol, title: entry.title,
                subtitle: [entry.kind.title, detail].compactMap { $0 }.joined(separator: " · "),
                state: problem, stateSymbol: problem == nil ? nil : "exclamationmark.circle.fill", stateColor: .systemOrange)
            row.toolTip = entry.title
            return row
        }
        if shown.count < entries.count {
            let more: StatusRow = keptRow("project:more") {
                let row = StatusRow()
                row.addGestureRecognizer(NSClickGestureRecognizer(target: self, action: #selector(showAllProject)))
                return row
            }
            more.configure(symbol: "ellipsis.circle", title: L("Show %d More", entries.count - shown.count), subtitle: "", state: nil)
            rows.append(more)
        }
        project.setRows(rows)
    }

    @objc private func openProjectEntry(_ sender: NSClickGestureRecognizer) {
        guard let id = sender.view?.identifier?.rawValue, case let .chat(chatID) = selection,
            let context = projectContexts[chatID], let entry = context.entries.first(where: { $0.id == id })
        else { return }
        presentAsSheet(ProjectEntryViewController(chatID: chatID, entry: entry, kind: entry.kind, otherVersions: context.otherVersions(of: id)))
    }

    @objc private func showAllProject() {
        guard case let .chat(chatID) = selection else { return }
        projectShowingAll = chatID
        reload()
    }

    /// What can be added: a note of each kind, a link, or a file.
    @objc private func showProjectMenu(_ sender: NSButton) {
        let menu = NSMenu()
        for kind in ProjectEntry.Kind.allCases {
            let item = NSMenuItem(title: kind.title + "…", action: #selector(addProjectEntry(_:)), keyEquivalent: "")
            item.target = self
            item.representedObject = kind.rawValue
            if kind == .document { menu.addItem(.separator()) }
            menu.addItem(item)
        }
        menu.popUp(positioning: nil, at: NSPoint(x: 0, y: sender.bounds.height + 4), in: sender)
    }

    @objc private func addProjectEntry(_ sender: NSMenuItem) {
        guard case let .chat(chatID) = selection, let raw = sender.representedObject as? String,
            let kind = ProjectEntry.Kind(rawValue: raw)
        else { return }
        guard kind == .asset else {
            presentAsSheet(ProjectEntryViewController(chatID: chatID, entry: nil, kind: kind))
            return
        }
        guard let window = view.window else { return }
        let panel = NSOpenPanel()
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.prompt = L("Add")
        panel.beginSheetModal(for: window) { [weak self] response in
            guard response == .OK, let url = panel.url else { return }
            Task { [weak self] in
                do {
                    try await self?.store.addProjectFile(url, to: chatID)
                } catch {
                    guard let window = self?.view.window else { return }
                    let alert = NSAlert()
                    alert.messageText = L("Couldn't add “%@”", url.lastPathComponent)
                    alert.informativeText = error.localizedDescription
                    alert.beginSheetModal(for: window, completionHandler: nil)
                }
            }
        }
    }

    private func showProfile(of bot: Bot) {
        // The Name row keeps what the user is typing, and puts the name back after.
        nameRow.setValue(bot.name)
        guard changed(profile, to: [bot.id, bot.description, bot.permissions]) else { return }
        descriptionRow.setValue(bot.description)
        accessRow.setValue((bot.permissions ?? BotPermissions()).summary)
        accessRow.onClick = { [weak self] in self?.presentAsSheet(BotAccessViewController(botID: bot.id)) }
        nameRow.onCommit = { [weak self] in self?.commitProfile(of: bot.id) }
    }

    private func showRuntime(of bot: Bot, in chat: Chat) {
        // The account's providers name the Provider pop-up's items and a custom provider's models.
        // A turn stopped at a limit turns the Limits row orange; a label going from gray to a
        // plain color needs a fresh row to draw right.
        let stopped = store.stoppedTurn(in: chat.id, runnerID: bot.runnerID)
        if changed(runtime, to: [chat.id, bot.id, bot.provider, bot.model, bot.thinking, store.providers, chat.usage == nil, stopped == nil]) {
            runtime.setRows(runtimeRows(for: bot, in: chat))
        }
        // What the turns used changes after every turn; the rows take the new values in place.
        if let usage = chat.usage {
            contextRow?.setValue(usage.contextSummary)
            spentRow?.setValue(usage.spendSummary)
            spentRow?.toolTip = usage.spendNote
        }
        limitsRow?.setValue(
            stopped?.stoppedLabel ?? store.budget("chat", chat.id, runnerID: bot.runnerID)?.limits.summary ?? L("None", context: "limits"),
            tint: stopped == nil ? .secondaryLabelColor : .systemOrange)
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

    /// What the chat's bots published, the latest first: a few rows, and View all for the rest.
    /// Hidden while there is none.
    private func showOutputs(in chat: Chat) {
        let all = store.outputs(in: chat.id)
        let shown = Array(all.prefix(3))
        if outputs.isHidden != all.isEmpty { outputs.isHidden = all.isEmpty }
        guard changed(outputs, to: [chat.id, chat.isGroup, shown, all.count > shown.count, store.bots.map(\.name)]) else { return }
        outputs.setHeaderAccessory(all.count > shown.count ? allOutputsButton : nil)
        outputs.setRows(
            shown.map { series in
                let row: StatusRow = keptRow("output:\(series.id)") {
                    let row = StatusRow()
                    row.addGestureRecognizer(NSClickGestureRecognizer(target: self, action: #selector(openOutput(_:))))
                    return row
                }
                row.configure(output: series, showsBot: chat.isGroup)
                return row
            })
    }

    @objc private func openOutput(_ sender: NSClickGestureRecognizer) {
        guard let id = sender.view?.identifier?.rawValue, case let .chat(chatID) = selection,
            let series = store.outputs(in: chatID).first(where: { $0.id == id })
        else { return }
        presentAsSheet(OutputViewController(chatID: chatID, series: series))
    }

    private func showAllOutputs() {
        guard case let .chat(chatID) = selection else { return }
        presentAsSheet(OutputsViewController(chatID: chatID))
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
        presentAsSheet(DescriptionViewController.bot(bot.id))
    }

    private func commitGroupName(of chatID: Chat.ID) {
        guard let chat = store.chat(chatID), chat.isGroup, groupNameRow.value != (chat.customTitle ?? "") else { return }
        store.rename(chatID, to: groupNameRow.value)
    }

    private func editGroupDescription() {
        guard case let .chat(chatID) = selection, store.chat(chatID)?.isGroup == true else { return }
        presentAsSheet(DescriptionViewController.group(chatID))
    }

    private func runtimeRows(for bot: Bot, in chat: Chat) -> [NSView] {
        // A custom provider the account deleted stays listed while the bot is still on it.
        var kinds = store.providerKinds
        if !kinds.contains(bot.provider) { kinds.append(bot.provider) }
        let providerRow = PopUpRow(
            key: L("Provider"),
            items: kinds.map(\.name),
            selected: kinds.firstIndex(of: bot.provider) ?? 0)
        providerRow.onChange = { [weak self] index in
            guard let self, kinds.indices.contains(index), kinds[index] != bot.provider else { return }
            // A new provider starts on its default model and thinking level.
            self.store.setBotRuntime(bot.id, provider: kinds[index], model: nil, thinking: nil)
        }

        let models = store.models(for: bot.provider)
        let modelItems = [L("Default (%@)", models.first?.label ?? "")] + models.map(\.label)
        let selectedModel = bot.model.flatMap { id in models.firstIndex { $0.id == id } }.map { $0 + 1 } ?? 0
        let modelRow = PopUpRow(key: L("Model"), items: modelItems, selected: selectedModel)
        modelRow.onChange = { [weak self] index in
            guard let self else { return }
            let model: String? = index == 0 ? nil : models[index - 1].id
            guard model != bot.model else { return }
            // A level the new model does not take goes back to the default.
            let kept = self.store.thinkingLevels(for: bot.provider, model: model).contains { $0.id == bot.thinking }
            self.store.setBotRuntime(bot.id, provider: bot.provider, model: model, thinking: kept ? bot.thinking : nil)
        }

        let levels = store.thinkingLevels(for: bot.provider, model: bot.model)
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

        // What each turn may use, or the turn that stopped at a limit; its value comes in
        // place from showRuntime.
        let limits = DisclosureRow(key: L("Limits"))
        limits.onClick = { [weak self] in
            guard let self, let bot = self.store.bot(bot.id) else { return }
            self.presentAsSheet(BudgetViewController(bot: bot, chatID: chat.id))
        }
        limitsRow = limits

        // Only the levels this model takes; a model without any has no choice to make.
        return [providerRow, modelRow] + (levels.isEmpty ? [] : [thinkingRow]) + [status] + usageRows + [limits]
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
        let stopped = mine.map { store.budget("routine", $0.id, runnerID: bot.runnerID).flatMap { $0.isStopped ? $0.stoppedLabel : nil } }
        guard changed(routines, to: [bot.id, mine, mine.map(\.detail), stopped]) else { return }
        guard !mine.isEmpty else {
            routines.setRows([noRoutinesRow])
            return
        }
        routines.setRows(
            zip(mine, stopped).map { routine, stopped in
                let row = keptRow("routine:\(routine.id):\(stopped != nil)") { SwitchRow() }
                row.configure(routine: routine, stopped: stopped)
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

    /// What the user's feedback led to: the changes the bot suggests, each a click away from its
    /// diff, and the rest of its feedback. Left out until there is any.
    private func showFeedback(of bot: Bot) {
        let known = feedbackByBot[bot.id]
        let hidden = known?.isEmpty ?? true
        if feedback.isHidden != hidden { feedback.isHidden = hidden }
        guard let known, !hidden, changed(feedback, to: [bot.id, known]) else { return }
        var rows: [NSView] = known.suggestions.prefix(3).map { suggestion in
            let row = FeedbackRow(suggestion: suggestion, in: known)
            row.onClick = { [weak self] in
                guard let self, let bot = self.store.bot(bot.id) else { return }
                let sheet = FeedbackChangeViewController(bot: bot, feedback: known, item: .suggestion(suggestion))
                sheet.onShowMessage = self.onShowMessage
                self.presentAsSheet(sheet)
            }
            return row
        }
        var counts = [known.noteCount == 1 ? L("1 note") : L("%d notes", known.noteCount)]
        if !known.changes.isEmpty { counts.append(known.changes.count == 1 ? L("1 change") : L("%d changes", known.changes.count)) }
        let all = FeedbackRow(symbol: "bubble.left.and.bubble.right", title: L("All feedback"), detail: counts.joined(separator: " · "))
        all.onClick = { [weak self] in
            guard let self, let bot = self.store.bot(bot.id) else { return }
            let sheet = FeedbackListViewController(bot: bot, feedback: self.feedbackByBot[bot.id] ?? known)
            sheet.onShowMessage = self.onShowMessage
            self.presentAsSheet(sheet)
        }
        rows.append(all)
        feedback.setRows(rows)
    }

    /// The plugins the bot's Runner has, and a way to the marketplace. A plugin that needs setup
    /// says so, and one the bot's Access leaves out says it has none; clicking opens it.
    private func showPlugins(of bot: Bot) {
        let runner = store.device(bot.runnerID)
        guard changed(plugins, to: [bot.id, bot.name, runner?.id, runner?.name, runner?.plugins, bot.permissions]) else { return }
        pluginBotID = bot.id
        var rows: [NSView] = (runner?.plugins ?? []).map { plugin in
            // A row keeps the vibrancy its state label had when it went in, so one with no
            // access is a row of its own.
            let off = bot.permissions?.level(of: plugin.id) == AccessLevel.none
            let row: StatusRow = keptRow("plugin:\(plugin.id):\(off)") {
                let row = StatusRow()
                row.identifier = NSUserInterfaceItemIdentifier(plugin.id)
                row.addGestureRecognizer(NSClickGestureRecognizer(target: self, action: #selector(openPlugin(_:))))
                return row
            }
            row.configure(plugin: plugin, hasAccess: !off)
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
        var chatID: Chat.ID?
        if case let .chat(chat) = selection { chatID = chat }
        PluginViewController.present(pluginID: id, runner: runner, bot: bot, chatID: chatID, from: self)
    }

    @objc private func addBot() {
        onAddBot?()
    }

    @objc private func openDevice(_ sender: NSClickGestureRecognizer) {
        guard let id = sender.view?.identifier?.rawValue else { return }
        onOpenDevice?(id)
    }
}

/// A word that opens the rest of a section. A label, so its text ends on the rows' trailing text
/// edge as a row's state does; a button's cell pads its title differently.
private final class ViewAllLabel: NSTextField {
    private var onPress: (() -> Void)?

    convenience init(_ title: String, onPress: @escaping () -> Void) {
        self.init(labelWithString: title)
        self.onPress = onPress
        font = .systemFont(ofSize: 11)
        textColor = .secondaryLabelColor
        setContentCompressionResistancePriority(.required, for: .horizontal)
    }

    override func mouseDown(with event: NSEvent) { onPress?() }
    override func accessibilityRole() -> NSAccessibility.Role? { .button }
    override func accessibilityPerformPress() -> Bool {
        onPress?()
        return true
    }
}
