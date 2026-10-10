import AppKit

/// One durable task. A card on top says where it stands, why when it is blocked or cancelled,
/// its result and what supports it, and offers the one step that fits: Start, Resume, Mark
/// Complete, or Reopen. Below it the goal, owner, next step, and what done looks like are a form
/// that Save writes back, then the tasks it waits for and its links, when it has them.
///
/// Edits start from the version the sheet showed. A change from elsewhere replaces the form while
/// the user has not touched it. When Save finds the task changed underneath, the sheet takes the
/// newer version under the user's edits and says so; the next Save writes them. A new task is the
/// same form, and once created the sheet shows it.
final class DurableTaskViewController: SheetViewController, NSTextFieldDelegate {
    private let store = AppStore.shared
    private let chatID: Chat.ID
    /// The version the form's edits start from; nil until a new task is created.
    private var base: DurableTask?
    private let creationID = "task-\(UUID().uuidString.lowercased())"

    private let status = SectionView(title: "")
    private let statusRow = StatusRow()
    /// What the task's runs may use, last in its card; opens the task's Limits.
    private let limitsRow = DisclosureRow(key: L("Limits"))
    private let goal = NSTextField()
    private let owner = NSPopUpButton()
    private var ownerRow: NSView!
    private let nextStep = NSTextField()
    private let criteria = WrappingTextField()
    private let waitsFor = SectionView(title: L("Waits For"))
    private let linksSection = SectionView(title: L("Links"))
    private let message = Build.label("", font: .systemFont(ofSize: 12), color: .secondaryLabelColor, lines: 0)
    private let cancelTaskButton = NSButton()
    /// The bots of the task's chats, which the owner is one of.
    private var owners: [Bot] = []
    /// A request and its id: a retry after an unclear failure sends the same id, so the CLI
    /// answers it once.
    private var sent: (body: Data, id: String)?
    private var isBusy = false

    init(chatID: Chat.ID, task: DurableTask?) {
        self.chatID = chatID
        base = task
        super.init(
            title: task == nil ? L("New Task") : L("Task"),
            subtitle: task == nil ? L("A task stays with its bot across turns, until it’s done.") : "",
            width: 520)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()
        statusRow.onAction = { [weak self] in self?.statusAction() }
        limitsRow.onClick = { [weak self] in self?.openLimits() }

        goal.placeholderString = L("What should get done")
        nextStep.placeholderString = L("What the bot does first")
        criteria.placeholderString = L("One check per line")
        criteria.usesSingleLineMode = false
        criteria.maximumNumberOfLines = 0
        criteria.lineBreakMode = .byWordWrapping
        criteria.cell?.wraps = true
        criteria.cell?.isScrollable = false
        criteria.heightAnchor.constraint(greaterThanOrEqualToConstant: 54).isActive = true
        for field in [goal, nextStep, criteria] { field.delegate = self }
        owner.target = self
        owner.action = #selector(ownerChanged)
        ownerRow = formRow(L("Owner"), owner)
        message.isSelectable = true

        let rows: [NSView] = [
            status,
            formRow(L("Goal"), goal),
            ownerRow,
            formRow(L("Next step"), nextStep),
            formRow(L("Done when"), criteria, topAligned: true),
            waitsFor, linksSection, message,
        ]
        for row in rows {
            contentStack.addArrangedSubview(row)
            row.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
        }
        contentStack.setCustomSpacing(16, after: status)
        contentStack.setCustomSpacing(16, after: rows[4])

        // Destructive, and apart from Cancel and Save. Its title is attributed: a rounded bezel
        // ignores tint colors.
        cancelTaskButton.bezelStyle = .rounded
        cancelTaskButton.attributedTitle = NSAttributedString(
            string: L("Cancel Task…"), attributes: [.foregroundColor: NSColor.systemRed, .font: NSFont.systemFont(ofSize: NSFont.systemFontSize)])
        cancelTaskButton.target = self
        cancelTaskButton.action = #selector(confirmCancelTask)
        setButtons(confirm: base == nil ? L("Create Task") : L("Save"), leading: cancelTaskButton)

        if let base { populate(base) } else { refreshOwners(select: store.chat(chatID)?.owner) }
        refresh()
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        store.observe(self) { [weak self] event in
            switch event {
            case .durableTasksChanged, .rosterChanged, .chatsChanged, .snapshotReplaced, .budgetsChanged: self?.refresh()
            default: break
            }
        }
    }

    // MARK: - The form

    private struct Form: Equatable {
        var goal: String
        var nextStep: String
        var criteria: [String]
        var owner: Bot.ID?
    }

    private var form: Form {
        Form(
            goal: goal.stringValue.trimmingCharacters(in: .whitespacesAndNewlines),
            nextStep: nextStep.stringValue.trimmingCharacters(in: .whitespacesAndNewlines),
            criteria: criteria.stringValue.components(separatedBy: .newlines)
                .map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty },
            owner: owners.indices.contains(owner.indexOfSelectedItem) ? owners[owner.indexOfSelectedItem].id : nil)
    }

    private static func form(of task: DurableTask) -> Form {
        Form(
            goal: task.goal.trimmingCharacters(in: .whitespacesAndNewlines),
            nextStep: task.nextAction.trimmingCharacters(in: .whitespacesAndNewlines),
            criteria: task.acceptanceCriteria, owner: task.ownerBotId)
    }

    private var hasEdits: Bool { base.map { form != Self.form(of: $0) } ?? false }

    private var canSave: Bool {
        let form = form
        return !isBusy && form.owner != nil && !form.goal.isEmpty && !form.nextStep.isEmpty && !form.criteria.isEmpty
    }

    private func populate(_ task: DurableTask) {
        goal.stringValue = task.goal
        nextStep.stringValue = task.nextAction
        criteria.stringValue = task.acceptanceCriteria.joined(separator: "\n")
        criteria.invalidateIntrinsicContentSize()
        refreshOwners(select: task.ownerBotId)
    }

    /// Takes `latest` under the user's edits: a field they changed keeps their words, the others
    /// take the newer version's.
    private func rebase(onto latest: DurableTask) {
        guard let base else { return }
        let before = Self.form(of: base), now = form
        if now.goal == before.goal { goal.stringValue = latest.goal }
        if now.nextStep == before.nextStep { nextStep.stringValue = latest.nextAction }
        if now.criteria == before.criteria { criteria.stringValue = latest.acceptanceCriteria.joined(separator: "\n") }
        self.base = latest
        refreshOwners(select: now.owner == before.owner ? latest.ownerBotId : now.owner)
    }

    private func refreshOwners(select id: Bot.ID?) {
        let chats = base?.chatIds ?? [chatID]
        let bots = store.bots.filter { bot in chats.contains { store.chat($0)?.botIDs.contains(bot.id) == true } }
        if bots.map(\.id) != owners.map(\.id) || bots.map(\.name) != owners.map(\.name) {
            owners = bots
            owner.removeAllItems()
            owner.addItems(withTitles: bots.map(\.name))
        }
        owner.selectItem(at: owners.firstIndex { $0.id == id } ?? 0)
    }

    @objc private func ownerChanged() {
        updateConfirm()
    }

    func controlTextDidChange(_ obj: Notification) {
        updateConfirm()
    }

    /// Return starts a new line in Done when, one check per line, rather than saving.
    func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
        guard control === criteria, selector == #selector(NSResponder.insertNewline(_:)) else { return false }
        textView.insertNewlineIgnoringFieldEditor(nil)
        return true
    }

    private func updateConfirm() {
        confirmButton.isEnabled = canSave
    }

    // MARK: - Showing the task

    /// The newest version the store has, which the status and the sections show.
    private var latest: DurableTask? { base.flatMap { store.durableTask($0.id) } ?? base }

    private func refresh() {
        guard isViewLoaded else { return }
        if let base, let newer = store.durableTask(base.id), newer.revision > base.revision, !hasEdits {
            self.base = newer
            populate(newer)
        } else {
            refreshOwners(select: form.owner ?? base?.ownerBotId)
        }
        ownerRow.isHidden = owners.count < 2
        let task = latest
        status.isHidden = task == nil
        if let task { status.setRows([statusRow] + resultRows(of: task) + [limitsRow]) }
        if let task { showStatus(of: task) }
        if let task {
            let budget = store.budget("task", task.id, runnerID: task.runnerId)
            let stopped = budget?.isStopped == true
            limitsRow.setValue(
                stopped ? budget?.stoppedLabel ?? "" : budget?.limits.summary ?? L("None", context: "limits"),
                tint: stopped ? .systemOrange : .secondaryLabelColor)
        }
        showDependencies(of: task)
        showLinks(of: task)
        message.isHidden = message.stringValue.isEmpty
        cancelTaskButton.isHidden = task == nil || task?.state.isFinished == true
        // A running task keeps its owner and its definition of done until the run ends.
        let running = task?.activeRun != nil
        owner.isEnabled = !running && !isBusy
        criteria.isEnabled = !running
        cancelTaskButton.isEnabled = !isBusy
        updateConfirm()
        fitSheetToContent()
    }

    private func showStatus(of task: DurableTask) {
        var detail = ""
        var action: String?
        switch task.state {
        case .queued:
            action = L("Start")
        case .blocked:
            detail = task.reason ?? ""
            action = L("Resume")
        case .working:
            if task.activeRun != nil, let bot = store.bot(task.ownerBotId) { detail = L("%@ is working on it.", bot.name) }
        case .awaitingReview:
            if task.result?.isEmpty == false && !task.evidence.isEmpty { action = L("Mark Complete") }
        case .completed:
            action = L("Reopen")
        case .cancelled:
            detail = task.reason == DurableTask.cancelledByUser ? "" : task.reason ?? ""
            action = L("Reopen")
        }
        if task.canStart, task.dependencies.contains(where: { store.durableTask($0)?.state != .completed }) {
            detail = L("It starts once the tasks it waits for are completed.")
            action = nil
        }
        statusRow.configure(
            symbol: task.state.symbol, title: task.state.title, subtitle: detail, state: nil,
            actionTitle: isBusy ? nil : action, symbolTint: task.state.tint)
    }

    /// The result, then what supports it.
    private func resultRows(of task: DurableTask) -> [NSView] {
        let result = task.result ?? ""
        let evidence = task.evidence
        var rows: [NSView] = []
        if !result.isEmpty { rows.append(TextRow(text: result)) }
        let limit = 5
        for item in evidence.prefix(evidence.count > limit ? limit - 1 : limit) {
            rows.append(evidenceRow(item))
        }
        if evidence.count > limit {
            rows.append(NoteRow(text: L("%d more", evidence.count - (limit - 1))))
        }
        return rows
    }

    /// A message by who wrote it and when, an output by its name and version (a click opens that
    /// version), a link by where it goes; the CLI's label otherwise.
    private func evidenceRow(_ item: DurableTask.Evidence) -> NSView {
        let row = StatusRow()
        var title = item.label
        var subtitle = ""
        let message = item.chatId.flatMap { store.chat($0) }?.messages.first { $0.id == item.messageId }
        if item.kind == "output", let version = item.version {
            subtitle = L("Version %d", Int(version))
            if let message {
                row.toolTip = L("Open %@", item.label)
                row.addGestureRecognizer(ClickHandler { [weak self] in OutputActions.open(message, window: self?.view.window) })
            }
        } else if item.kind == "message", let message {
            switch message.author {
            case .you: title = L("Your message")
            case let .bot(id): title = L("Message from %@", store.bot(id)?.name ?? L("a bot"))
            case .system: title = L("Message")
            }
            subtitle = Format.daySeparator(message.createdAt)
        } else if item.kind == "message" {
            title = L("Message")
        }
        if let link = item.url.flatMap(URL.init(string:)) {
            subtitle = link.host ?? ""
            row.toolTip = item.url
            row.addGestureRecognizer(ClickHandler { NSWorkspace.shared.open(link) })
        }
        row.configure(symbol: item.symbol, title: title, subtitle: subtitle, state: nil)
        return row
    }

    private func showDependencies(of task: DurableTask?) {
        let ids = task?.dependencies ?? []
        waitsFor.isHidden = ids.isEmpty
        guard !ids.isEmpty else { return }
        waitsFor.setRows(ids.map { id in
            let row = StatusRow()
            let dependency = store.durableTask(id)
            row.configure(
                symbol: dependency?.state.symbol ?? "circle", title: dependency?.goal ?? L("A task on another Device"),
                subtitle: dependency?.state.title ?? "", state: nil, symbolTint: dependency?.state.tint ?? .tertiaryLabelColor)
            return row
        })
    }

    private func showLinks(of task: DurableTask?) {
        let links = task?.links ?? []
        linksSection.isHidden = links.isEmpty
        guard !links.isEmpty else { return }
        linksSection.setRows(links.map { link in
            let row = StatusRow()
            let url = URL(string: link.url)
            row.configure(symbol: "link", title: link.label, subtitle: link.label == link.url ? "" : url?.host ?? "", state: nil)
            row.toolTip = link.url
            if let url { row.addGestureRecognizer(ClickHandler { NSWorkspace.shared.open(url) }) }
            return row
        })
    }

    // MARK: - Writing it back

    override func confirmTapped() {
        guard canSave else { return }
        guard let base else {
            let form = form
            send("tasks.create", [
                "id": creationID, "owner_bot_id": form.owner ?? "", "goal": form.goal, "next_action": form.nextStep,
                "acceptance_criteria": form.criteria, "chat_ids": [chatID],
            ]) { [weak self] task in
                guard let self else { return }
                self.base = task
                setSheetTitle(L("Task"), subtitle: "")
                confirmButton.title = L("Save")
                refresh()
            }
            return
        }
        let edits = changes(from: base)
        guard !edits.isEmpty else { return dismiss(nil) }
        update(edits) { [weak self] _ in self?.dismiss(nil) }
    }

    /// The fields the user changed from `task`.
    private func changes(from task: DurableTask) -> [String: Any] {
        let before = Self.form(of: task), now = form
        var params: [String: Any] = [:]
        if now.goal != before.goal { params["goal"] = now.goal }
        if now.nextStep != before.nextStep { params["next_action"] = now.nextStep }
        if now.criteria != before.criteria { params["acceptance_criteria"] = now.criteria }
        if let owner = now.owner, owner != before.owner { params["owner_bot_id"] = owner }
        return params
    }

    /// Writes the form's edits, and `params` with them, over the version they start from.
    private func update(_ params: [String: Any], then done: @escaping (DurableTask) -> Void) {
        guard let base else { return }
        let edits = changes(from: base).merging(params) { _, new in new }
        send("tasks.update", edits.merging(["id": base.id, "expected_revision": base.revision]) { _, new in new }, then: done)
    }

    private func statusAction() {
        guard let task = latest else { return }
        switch task.state {
        // A task its limits stopped resumes in Limits: a run would only be refused again.
        case .blocked where store.budget("task", task.id, runnerID: task.runnerId)?.isStopped == true:
            openLimits()
        case .queued, .blocked, .working:
            start()
        case .awaitingReview:
            update(["state": DurableTask.State.completed.rawValue]) { [weak self] _ in self?.dismiss(nil) }
        case .completed, .cancelled:
            update(["state": DurableTask.State.queued.rawValue]) { [weak self] _ in self?.refresh() }
        }
    }

    /// Saves what the user changed, then starts a run in this chat when the owner is in it, or
    /// in the first of the task's chats it is in, and closes on the chat where it works.
    private func start() {
        let run: (DurableTask) -> Void = { [weak self] task in
            guard let self else { return }
            var params: [String: Any] = ["id": task.id, "expected_revision": task.revision]
            if store.chat(chatID)?.botIDs.contains(task.ownerBotId) == true { params["chat_id"] = chatID }
            send("tasks.run", params) { [weak self] _ in self?.dismiss(nil) }
        }
        guard let base else { return }
        if changes(from: base).isEmpty { run(base) } else { update([:], then: run) }
    }

    private func openLimits() {
        guard let task = latest, let owner = store.bot(task.ownerBotId) else { return }
        presentAsSheet(BudgetViewController(bot: owner, task: task))
    }

    @objc private func confirmCancelTask() {
        guard let task = latest, let window = view.window else { return }
        let alert = NSAlert()
        alert.messageText = L("Cancel this task?")
        alert.informativeText = task.activeRun != nil
            ? L("%@ stops working on it. You can reopen it later.", store.bot(task.ownerBotId)?.name ?? L("The bot"))
            : L("You can reopen it later.")
        alert.addButton(withTitle: L("Cancel Task")).hasDestructiveAction = true
        alert.addButton(withTitle: L("Keep Task"))
        alert.beginSheetModal(for: window) { [weak self] response in
            guard response == .alertFirstButtonReturn else { return }
            self?.update(["state": DurableTask.State.cancelled.rawValue, "reason": DurableTask.cancelledByUser]) { [weak self] _ in
                self?.dismiss(nil)
            }
        }
    }

    private func send(_ method: String, _ params: [String: Any], then done: @escaping (DurableTask) -> Void) {
        guard !isBusy else { return }
        var params = params
        let body = (try? JSONSerialization.data(withJSONObject: params.merging(["method": method]) { old, _ in old }, options: [.sortedKeys])) ?? Data()
        if sent?.body != body { sent = (body, UUID().uuidString.lowercased()) }
        params["request_id"] = sent?.id
        isBusy = true
        message.stringValue = ""
        refresh()
        Task { [weak self] in
            do {
                let task = try await AppStore.shared.taskRequest(method, params: params)
                guard let self else { return }
                isBusy = false
                sent = nil
                done(task)
            } catch {
                await self?.failed(error)
            }
        }
    }

    /// A stale version takes the newer one under the user's edits; anything else says what the
    /// CLI said.
    private func failed(_ error: Error) async {
        let text = error.localizedDescription
        if text.hasPrefix("Task revision conflict"), let base {
            if let newer = try? await store.taskRequest("tasks.get", params: ["id": base.id, "refresh": true]) { rebase(onto: newer) }
            message.textColor = .secondaryLabelColor
            message.stringValue = L("This task changed since you opened it. Your edits are still here; save again to keep them.")
        } else {
            message.textColor = .systemRed
            message.stringValue = text
        }
        isBusy = false
        refresh()
    }
}

/// A block of text inside a section card, to read and select, on the column of the titles of
/// the rows with a symbol: a task's result.
private final class TextRow: NSView {
    init(text: String) {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        let label = Build.label(text, font: .systemFont(ofSize: 12), lines: 8)
        label.cell?.truncatesLastVisibleLine = true
        label.isSelectable = true
        label.toolTip = text.count > 400 ? text : nil
        addSubview(label)
        NSLayoutConstraint.activate([
            label.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 40),
            label.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
            label.topAnchor.constraint(equalTo: topAnchor, constant: 10),
            label.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -10),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }
}

/// A click anywhere on a row, for rows that open a link.
final class ClickHandler: NSClickGestureRecognizer {
    private let handler: () -> Void

    init(handler: @escaping () -> Void) {
        self.handler = handler
        super.init(target: nil, action: nil)
        target = self
        action = #selector(clicked)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    @objc private func clicked() { handler() }
}
