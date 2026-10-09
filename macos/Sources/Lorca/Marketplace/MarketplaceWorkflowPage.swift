import AppKit

extension MarketplacePage {
    /// A workflow's row: its symbol on a tile, its name, and its outcome. A click opens its page.
    func workflowRow(_ pack: WorkflowPack) -> MarketplaceRow {
        let row = MarketplaceRow(
            media: PluginIconView(pluginID: pack.id, symbol: pack.symbolName, size: 40), title: pack.name, byline: nil,
            subtitle: pack.outcome, accessory: nil)
        row.onOpen = { [weak self] in self?.market.openWorkflow(pack) }
        return row
    }

    func workflowSection(_ packs: [WorkflowPack]) -> NSView {
        MarketplaceSection(title: L("Workflows"), rows: packs.map(workflowRow))
    }
}

/// The workflows alone: the first page of the sheet onboarding's Choose a Workflow opens.
final class MarketplaceWorkflowListPage: MarketplacePage {
    override func reload() {
        let packs = market.catalog.packs
        let body: NSView
        if !packs.isEmpty {
            body = MarketplaceGrid(rows: packs.map(workflowRow))
        } else if market.loading == .failed {
            let retry = ActionButton(title: L("Try Again")) { [weak self] in self?.market.load() }
            body = Build.stack([statusLine(L("The marketplace isn't available right now.")), retry], spacing: 10)
        } else {
            body = statusLine(market.loading == .loaded ? L("Nothing here yet.") : L("Loading…"))
        }
        setContent([pageTitle(L("Choose a Workflow")), body], spacing: 12)
    }
}

/// A workflow's setup on the picked Runner, after the plugin page: its questions and its bot, the
/// accounts it needs, a sample run to read, and its schedule, which stays off until the user
/// turns it on. The CLI keeps the setup, so leaving the page keeps it too.
final class MarketplaceWorkflowPage: MarketplacePage {
    private let pack: WorkflowPack
    private var progress: WorkflowProgress?
    /// The answers and bots as typed and picked, until running the sample saves them. A bot is
    /// its id, or empty for the new one setup adds.
    private var answers: [String: String] = [:]
    private var bots: [String: String] = [:]
    private var busy = false
    private var fetching = false
    private var fetchAgain = false

    private let actions = Build.stack([], orientation: .horizontal, spacing: 8)
    private let byline = Build.label("", font: .systemFont(ofSize: 12.5), color: .secondaryLabelColor)
    private let setupCard = SectionView(title: L("Setup", context: "workflow questions"))
    private let accountsCard = SectionView(title: L("Accounts"))
    private let sampleCard = SectionView(title: L("Sample"))
    private let scheduleCard = SectionView(title: L("Schedule"))
    private let footer = NSView()
    // Rows that live as long as the page, so a field keeps its text and focus through updates.
    private var answerRows: [String: EditableRow] = [:]
    private var botRows: [String: PopUpRow] = [:]
    private var accountPickers: [String: PopUpRow] = [:]
    private var accountRows: [String: StatusRow] = [:]
    private let sampleText = Build.label("", font: .systemFont(ofSize: 13), lines: 0)

    init(market: MarketplaceViewController, pack: WorkflowPack) {
        self.pack = pack
        super.init(market: market)
    }

    override func loadView() {
        super.loadView()
        for card in [setupCard, accountsCard, sampleCard, scheduleCard] {
            card.style = .heading
            card.isHidden = true
        }
        sampleText.isSelectable = true
        sampleText.allowsEditingTextAttributes = true
        footer.translatesAutoresizingMaskIntoConstraints = false
        let outcome = Build.label(pack.outcome, font: .systemFont(ofSize: 13), color: .secondaryLabelColor, lines: 0)
        let outcomeRow = NSView()
        outcomeRow.translatesAutoresizingMaskIntoConstraints = false
        outcomeRow.addSubview(outcome)
        outcome.pin(to: outcomeRow, insets: NSEdgeInsets(top: 0, left: 12, bottom: 0, right: 12))
        let top = Build.stack([header(), outcomeRow], spacing: 14)
        for view in top.arrangedSubviews { view.widthAnchor.constraint(equalTo: top.widthAnchor).isActive = true }
        setContent([top, setupCard, accountsCard, sampleCard, scheduleCard, footer], spacing: 22)
        store.observe(self) { [weak self] event in
            switch event {
            case .rosterChanged, .snapshotReplaced, .turnFinished, .connectionChanged: self?.refresh()
            default: break
            }
        }
        start()
    }

    private func header() -> NSView {
        let icon = PluginIconView(pluginID: pack.id, symbol: pack.symbolName, size: 56)
        let name = Build.label(pack.name, font: .systemFont(ofSize: 20, weight: .semibold))
        let titles = Build.stack([name, byline], spacing: 3)
        let top = NSView()
        top.translatesAutoresizingMaskIntoConstraints = false
        for view in [icon, titles, actions] as [NSView] { top.addSubview(view) }
        NSLayoutConstraint.activate([
            icon.leadingAnchor.constraint(equalTo: top.leadingAnchor, constant: 12),
            icon.topAnchor.constraint(equalTo: top.topAnchor),
            icon.bottomAnchor.constraint(equalTo: top.bottomAnchor),
            titles.leadingAnchor.constraint(equalTo: icon.trailingAnchor, constant: 14),
            titles.centerYAnchor.constraint(equalTo: icon.centerYAnchor),
            actions.trailingAnchor.constraint(equalTo: top.trailingAnchor, constant: -12),
            actions.centerYAnchor.constraint(equalTo: icon.centerYAnchor),
            titles.trailingAnchor.constraint(lessThanOrEqualTo: actions.leadingAnchor, constant: -12),
        ])
        return top
    }

    override func reload() {
        // The page follows the Runner picked in the top bar; each Runner has its own setup.
        if let progress, let runnerID = market.runnerID, progress.setup.runnerId != runnerID, !busy {
            bots = [:]
            start()
        }
        render()
    }

    // MARK: - Requests

    private func start() {
        guard let runnerID = market.runnerID else { return render() }
        perform { [pack] store in try await store.workflow("start", ["pack_id": pack.id, "runner_id": runnerID]) }
    }

    /// Reads the setup again after something it shows changed: a sign-in, a sample's end.
    private func refresh() {
        guard let id = progress?.setup.id else { return }
        guard !busy, !fetching else {
            fetchAgain = true
            return
        }
        fetching = true
        Task { [weak self] in
            guard let self else { return }
            let result = try? await self.store.workflow("get", ["id": id])
            self.fetching = false
            if let result, result.setup.id == self.progress?.setup.id { self.apply(result) }
            if self.fetchAgain {
                self.fetchAgain = false
                self.refresh()
            }
        }
    }

    /// One request, or a few in a row, while the page's actions wait. A failure's reason shows
    /// at the foot of the sheet.
    private func perform(_ work: @escaping (AppStore) async throws -> WorkflowProgress, then: ((WorkflowProgress) -> Void)? = nil) {
        guard !busy else { return }
        busy = true
        render()
        Task { [weak self] in
            guard let self else { return }
            var finished: WorkflowProgress?
            do {
                finished = try await work(self.store)
            } catch {
                self.market.showNotice(error.localizedDescription, isError: true)
            }
            self.busy = false
            if let finished {
                self.apply(finished)
                then?(finished)
            } else {
                self.render()
            }
            if self.fetchAgain {
                self.fetchAgain = false
                self.refresh()
            }
        }
    }

    private func apply(_ next: WorkflowProgress) {
        progress = next
        for question in next.setup.pack.questions where answers[question.id] == nil {
            answers[question.id] = next.setup.answers[question.id] ?? ""
        }
        for specialist in next.specialists where bots[specialist.id] == nil {
            bots[specialist.id] = specialist.selectedId ?? ""
        }
        render()
    }

    private var trimmedAnswers: [String: String] { answers.mapValues { $0.trimmingCharacters(in: .whitespacesAndNewlines) } }

    /// Whether the page shows answers or bots that running the sample has not saved yet.
    private var isEdited: Bool {
        guard let progress else { return false }
        let saved = progress.setup.answers
        return progress.setup.pack.questions.contains { trimmedAnswers[$0.id] ?? "" != saved[$0.id] ?? "" }
            || progress.specialists.contains { (bots[$0.id] ?? "") != ($0.selectedId ?? "") }
    }

    private func runSample() {
        guard let id = progress?.setup.id else { return }
        let answers = trimmedAnswers
        let bots = bots.filter { !$0.value.isEmpty }
        perform({ store in
            _ = try await store.workflow("configure", ["id": id, "answers": answers, "bot_ids": bots])
            return try await store.workflow("sample", ["id": id])
        }, then: { [weak self] saved in
            // Saved now: a new bot is a bot of its own, and the menus show what setup holds.
            self?.bots = [:]
            self?.apply(saved)
        })
    }

    private func turnOn() {
        guard let setup = progress?.setup, let sample = setup.sample else { return }
        perform({ store in
            if sample.state != "reviewed" { _ = try await store.workflow("review", ["id": setup.id, "job_id": sample.jobId]) }
            return try await store.workflow("enable", ["id": setup.id])
        }, then: { [weak self] done in
            self?.market.finishWorkflow(chatID: done.setup.sample?.chatId)
        })
    }

    private func cancel() {
        guard let setup = progress?.setup else { return }
        let wasOn = setup.phase == "enabled"
        perform({ store in try await store.workflow("cancel", ["id": setup.id]) }, then: { [weak self] _ in
            guard let self else { return }
            self.market.showNotice(wasOn ? L("%@ is off.", self.pack.name) : L("Setup cancelled. Your answers are kept for next time."))
            self.market.goBack()
        })
    }

    private func choose(_ accountID: String, for connection: WorkflowProgress.Connection) {
        guard let id = progress?.setup.id else { return }
        perform { store in
            try await store.workflow("connection", ["id": id, "service_id": connection.serviceId, "plugin_id": accountID])
        }
    }

    /// Installs the service's plugin on the Runner for this workflow.
    private func add(_ connection: WorkflowProgress.Connection) {
        guard let id = progress?.setup.id else { return }
        perform { store in try await store.workflow("connection", ["id": id, "service_id": connection.serviceId]) }
    }

    private func manage(_ account: WorkflowProgress.Account) {
        guard let runner = market.runner else { return }
        PluginViewController.present(pluginID: account.plugin.id, runner: runner, bot: nil, from: self)
    }

    // MARK: - Rendering

    private func render() {
        guard isViewLoaded else { return }
        byline.stringValue = market.runner.map { L("Runs on %@", $0.name) } ?? L("Pair a Runner first.")
        renderActions()
        guard let progress else {
            for card in [setupCard, accountsCard, sampleCard, scheduleCard] { card.isHidden = true }
            setFooter(nil)
            return
        }
        let setup = progress.setup
        let locked = setup.phase == "enabled" || progress.isRunning || busy
        renderSetup(progress, locked: locked)
        renderAccounts(progress, locked: locked)
        renderSample(progress)
        scheduleCard.isHidden = progress.routines.isEmpty
        scheduleCard.setRows(progress.routines.map { routine in
            let row = StatusRow()
            row.configure(
                symbol: routine.isEnabled ? "clock" : "pause.circle", title: routine.name, subtitle: Format.schedule(routine.scheduleText),
                state: routine.isEnabled ? L("On") : L("Off"))
            return row
        })
        let isSetUp = !setup.botIds.isEmpty || setup.sample != nil
        setFooter(isSetUp ? (setup.phase == "enabled" ? L("Turn Off Workflow") : L("Cancel Setup")) : nil)
    }

    /// What can be done next, at the header's trailing end: run the sample, then turn its
    /// schedule on or leave it off.
    private func renderActions() {
        for view in actions.arrangedSubviews {
            actions.removeArrangedSubview(view)
            view.removeFromSuperview()
        }
        if busy {
            let spinner = NSProgressIndicator()
            spinner.style = .spinning
            spinner.controlSize = .small
            spinner.startAnimation(nil)
            actions.addArrangedSubview(spinner)
        }
        guard let progress else {
            if !busy, market.runner != nil {
                actions.addArrangedSubview(ActionButton(title: L("Try Again")) { [weak self] in self?.start() })
            }
            return
        }
        let setup = progress.setup
        if setup.phase == "enabled" { return }
        let hasResult = ["ready", "reviewed"].contains(setup.sample?.state) && !progress.sampleMessages.isEmpty && !progress.isRunning
        if hasResult, !isEdited {
            let later = ActionButton(title: L("Not Now")) { [weak self] in self?.market.finishWorkflow(chatID: setup.sample?.chatId) }
            later.controlSize = .large
            later.isEnabled = !busy
            let on = ActionButton(title: L("Turn On Schedule"), prominent: true) { [weak self] in self?.turnOn() }
            on.isEnabled = !busy
            actions.addArrangedSubview(later)
            actions.addArrangedSubview(on)
            return
        }
        let filled = setup.pack.questions.allSatisfy { !(trimmedAnswers[$0.id] ?? "").isEmpty }
        let connected = progress.connections.allSatisfy { $0.account?.plugin.state == .ready }
        let run = ActionButton(title: L("Run Sample"), prominent: true) { [weak self] in self?.runSample() }
        run.isEnabled = !busy && !progress.isRunning && filled && connected
        run.toolTip = !filled ? L("Fill in the setup first.") : !connected ? L("Connect the accounts first.") : nil
        actions.addArrangedSubview(run)
    }

    private func renderSetup(_ progress: WorkflowProgress, locked: Bool) {
        var rows: [NSView] = []
        for question in progress.setup.pack.questions {
            let row = answerRows[question.id] ?? {
                let row = EditableRow(key: question.label, placeholder: question.placeholder)
                row.field.stringValue = answers[question.id] ?? ""
                row.field.setAccessibilityLabel(question.label)
                NotificationCenter.default.addObserver(self, selector: #selector(answerChanged(_:)), name: NSControl.textDidChangeNotification, object: row.field)
                answerRows[question.id] = row
                return row
            }()
            row.field.isEditable = !locked
            row.field.textColor = locked ? .secondaryLabelColor : .labelColor
            rows.append(row)
        }
        for specialist in progress.specialists {
            let row = botRows[specialist.id] ?? {
                let row = PopUpRow(key: progress.specialists.count == 1 ? L("Bot") : specialist.name, items: [], selected: 0, fitsTitles: true)
                row.onChange = { [weak self, weak row] _ in
                    self?.bots[specialist.id] = row?.popUp.selectedItem?.representedObject as? String ?? ""
                    self?.renderActions()
                }
                botRows[specialist.id] = row
                return row
            }()
            let menu = row.popUp
            menu.removeAllItems()
            // The new bot is a choice only while setup would add one; once it has, it is a bot.
            if specialist.selectedId == nil {
                menu.addItem(withTitle: L("%@ (new)", specialist.name))
                menu.lastItem?.representedObject = ""
                if !specialist.choices.isEmpty { menu.menu?.addItem(.separator()) }
            }
            for bot in specialist.choices {
                menu.addItem(withTitle: bot.name)
                menu.lastItem?.representedObject = bot.id
            }
            let picked = bots[specialist.id] ?? ""
            menu.select(menu.itemArray.first { $0.representedObject as? String == picked } ?? menu.itemArray.first)
            menu.isEnabled = !locked
            rows.append(row)
        }
        // An account of several is the user's to pick; one is simply used.
        for connection in progress.connections where connection.choices.count > 1 {
            let row = accountPickers[connection.serviceId] ?? {
                let row = PopUpRow(key: connection.name, items: [], selected: 0, fitsTitles: true)
                accountPickers[connection.serviceId] = row
                return row
            }()
            row.onChange = { [weak self, weak row] _ in
                guard let id = row?.popUp.selectedItem?.representedObject as? String else { return }
                self?.choose(id, for: connection)
            }
            let menu = row.popUp
            menu.removeAllItems()
            if connection.account == nil { menu.addItem(withTitle: L("Choose…")) }
            for choice in connection.choices {
                menu.addItem(withTitle: choice.accountName ?? choice.plugin.name)
                menu.lastItem?.representedObject = choice.plugin.id
            }
            menu.select(menu.itemArray.first { $0.representedObject as? String == connection.account?.plugin.id } ?? menu.itemArray.first)
            menu.isEnabled = !locked
            rows.append(row)
        }
        setupCard.isHidden = rows.isEmpty
        setupCard.setRows(rows)
    }

    private func renderAccounts(_ progress: WorkflowProgress, locked: Bool) {
        let rows = progress.connections.map { connection -> StatusRow in
            let row = accountRows[connection.serviceId] ?? {
                let row = StatusRow()
                row.addGestureRecognizer(NSClickGestureRecognizer(target: self, action: #selector(openAccount(_:))))
                row.identifier = NSUserInterfaceItemIdentifier(connection.serviceId)
                accountRows[connection.serviceId] = row
                return row
            }()
            let account = connection.account
            var state: String?
            var color = NSColor.secondaryLabelColor
            var detail: String?
            var action: String?
            switch account?.plugin.state {
            // Just added: the Runner has yet to report it.
            case nil where connection.selectedId != nil: state = L("Connecting…")
            case nil where connection.choices.count > 1: state = L("Not chosen")
            case nil where connection.available: action = locked ? nil : L("Add")
            case nil:
                state = L("Not available")
                detail = L("%@ isn't in the marketplace yet. This setup waits for it.", connection.name)
            case .ready?: state = L("Connected")
            case .needsAuth?, .insufficientAccess?: action = L("Sign In")
            case .needsSetup?: action = L("Set Up")
            case .connecting?: state = L("Connecting…")
            case .error?, .unknown?:
                state = L("Can't connect")
                color = .systemRed
                detail = account?.plugin.detail
            }
            row.configure(
                symbol: "puzzlepiece.extension", image: PluginLogo.tile(for: connection.serviceId, size: 18), title: connection.name,
                subtitle: account?.accountName ?? "", state: state, stateColor: color, stateDetail: detail, actionTitle: action)
            row.onAction = { [weak self] in
                if let account { self?.manage(account) } else { self?.add(connection) }
            }
            row.toolTip = account.map { L("Open %@", $0.plugin.name) }
            return row
        }
        accountsCard.isHidden = rows.isEmpty
        accountsCard.setRows(rows)
    }

    private func renderSample(_ progress: WorkflowProgress) {
        let sample = progress.setup.sample
        let bot = progress.specialists.lazy.flatMap(\.choices).first { $0.id == sample?.botId }?.name ?? ""
        if progress.isRunning {
            let spinner = NSProgressIndicator()
            spinner.style = .spinning
            spinner.controlSize = .small
            spinner.startAnimation(nil)
            let line = Build.stack([spinner, Build.label(L("%@ is working on it…", bot), font: Theme.Font.caption, color: .secondaryLabelColor)],
                orientation: .horizontal, spacing: 8)
            sampleCard.setRows([padded(line, vertical: 12)])
        } else if !progress.sampleMessages.isEmpty, ["ready", "reviewed"].contains(sample?.state) {
            sampleText.attributedStringValue = Self.text(progress.sampleMessages.compactMap(\.body.text))
            sampleCard.setRows([padded(sampleText, vertical: 12)])
        } else if sample != nil {
            sampleCard.setRows([NoteRow(text: L("The sample didn't finish. %@'s chat says what happened.", bot))])
        }
        sampleCard.isHidden = sample == nil
    }

    /// The footer's one button, apart from the rest: Cancel Setup, or Turn Off once it is on.
    private func setFooter(_ title: String?) {
        footer.subviews.forEach { $0.removeFromSuperview() }
        footer.isHidden = title == nil
        guard let title else { return }
        let button = ActionButton(title: "") { [weak self] in self?.cancel() }
        button.attributedTitle = NSAttributedString(string: title, attributes: [.foregroundColor: NSColor.systemRed, .font: NSFont.systemFont(ofSize: 13)])
        button.isEnabled = !busy
        footer.addSubview(button)
        NSLayoutConstraint.activate([
            button.leadingAnchor.constraint(equalTo: footer.leadingAnchor, constant: 12),
            button.topAnchor.constraint(equalTo: footer.topAnchor),
            button.bottomAnchor.constraint(equalTo: footer.bottomAnchor),
        ])
    }

    private func padded(_ view: NSView, vertical: CGFloat) -> NSView {
        let row = NSView()
        row.translatesAutoresizingMaskIntoConstraints = false
        row.addSubview(view)
        view.pin(to: row, insets: NSEdgeInsets(top: vertical, left: 12, bottom: vertical, right: 12))
        return row
    }

    /// The sample's replies as a chat shows their Markdown, a table's cells on lines of their own.
    private static func text(_ replies: [String]) -> NSAttributedString {
        let out = NSMutableAttributedString()
        for segment in replies.flatMap({ Markdown.segments($0, textColor: .labelColor) }) {
            if out.length > 0 { out.append(NSAttributedString(string: "\n\n")) }
            switch segment {
            case let .text(text, _, _):
                out.append(text)
            case let .table(table):
                let lines = table.cells.map { $0.map(\.string).joined(separator: "  ·  ") }.joined(separator: "\n")
                out.append(NSAttributedString(string: lines, attributes: [.font: NSFont.systemFont(ofSize: 13), .foregroundColor: NSColor.labelColor]))
            }
        }
        return out
    }

    @objc private func answerChanged(_ notification: Notification) {
        guard let field = notification.object as? NSTextField,
            let id = answerRows.first(where: { $0.value.field === field })?.key
        else { return }
        answers[id] = field.stringValue
        renderActions()
    }

    @objc private func openAccount(_ sender: NSClickGestureRecognizer) {
        guard let id = sender.view?.identifier?.rawValue, let account = progress?.connections.first(where: { $0.serviceId == id })?.account
        else { return }
        manage(account)
    }
}
