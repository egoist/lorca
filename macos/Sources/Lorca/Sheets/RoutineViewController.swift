import AppKit

/// One routine's details, after Grok Bot's routine page: how it stands, with what happened and
/// how to fix it when something went wrong; the schedule, its next run, what happens to runs its
/// Runner missed, and its last check and run; the task, the check when it has one, and the
/// actions on it. Run Now starts it on the bot's Runner; Pause and Resume flip the switch the
/// inspector shows; Edit in Chat hands the bot the change to make, since the bot owns its
/// routines (its timezone and missed-run policy too); Delete asks first.
final class RoutineViewController: SheetViewController {
    private let store = AppStore.shared
    private let routineID: Routine.ID
    private let bot: Bot

    private let schedule = SectionView(title: L("Schedule"))
    private let task = SectionView(title: L("Task"))
    private let prompt = NSTextView()
    private let checkSection = SectionView(title: L("Check"))
    private let webhook = SectionView(title: L("Webhook"))
    /// The webhook's key shows only after a click on its row.
    private var keyShown = false
    private let check = NSTextView()
    private let runButton = NSButton()
    private let pauseButton = NSButton()
    private let editButton = NSButton()
    private let deleteButton = NSButton()

    /// Called with the text to put in the composer when the user wants the bot to edit it.
    var onEditInChat: ((String) -> Void)?

    init(routineID: Routine.ID, bot: Bot) {
        self.routineID = routineID
        self.bot = bot
        let routine = AppStore.shared.routine(routineID)
        super.init(
            title: routine?.name ?? L("Routine"),
            subtitle: L("A task %@ runs on its own in this chat. %@ set it up and can change it: ask in chat.", bot.name, bot.name),
            width: 520
        )
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()

        let scroll = Self.readOnly(prompt, font: .systemFont(ofSize: 12))
        task.setRows([scroll])
        let checkScroll = Self.readOnly(check, font: .monospacedSystemFont(ofSize: 11, weight: .regular))
        checkSection.setRows([checkScroll])

        for (button, title, action) in [
            (runButton, L("Run Now"), #selector(runNow)),
            (pauseButton, L("Pause"), #selector(togglePaused)),
            (editButton, L("Edit in Chat…"), #selector(editInChat)),
            (deleteButton, L("Delete…"), #selector(confirmDelete)),
        ] {
            button.title = title
            button.bezelStyle = .rounded
            button.controlSize = .regular
            button.target = self
            button.action = action
        }
        let spacer = NSView()
        spacer.setContentHuggingPriority(.defaultLow, for: .horizontal)
        let actions = Build.stack([runButton, pauseButton, editButton, spacer, deleteButton], orientation: .horizontal, spacing: 8)

        contentStack.addArrangedSubview(schedule)
        contentStack.addArrangedSubview(webhook)
        contentStack.addArrangedSubview(task)
        contentStack.addArrangedSubview(checkSection)
        contentStack.addArrangedSubview(actions)
        NSLayoutConstraint.activate([
            schedule.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            webhook.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            task.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            checkSection.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            actions.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            scroll.heightAnchor.constraint(equalToConstant: 96),
            checkScroll.heightAnchor.constraint(equalToConstant: 120),
        ])

        setButtons(confirm: L("Done"), cancel: nil)
        refresh()
    }

    /// A text view that shows text to read and select, scrolling inside a fixed height.
    private static func readOnly(_ text: NSTextView, font: NSFont) -> NSScrollView {
        text.isEditable = false
        text.isRichText = false
        text.font = font
        text.textColor = .labelColor
        text.drawsBackground = false
        text.textContainerInset = NSSize(width: 8, height: 8)
        text.isVerticallyResizable = true
        text.isHorizontallyResizable = false
        text.autoresizingMask = [.width]
        text.textContainer?.widthTracksTextView = true
        let scroll = NSScrollView()
        scroll.documentView = text
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.drawsBackground = false
        scroll.translatesAutoresizingMaskIntoConstraints = false
        return scroll
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        store.observe(self) { [weak self] event in
            switch event {
            case .rosterChanged, .chatsChanged, .snapshotReplaced, .budgetsChanged:
                self?.refresh()
            default:
                break
            }
        }
    }

    /// Redraws from the store; the sheet closes when the routine is gone.
    private func refresh() {
        guard isViewLoaded else { return }
        guard let routine = store.routines(for: bot.id).first(where: { $0.id == routineID }) else {
            dismiss(nil)
            return
        }
        let runner = store.device(bot.runnerID)
        let runnerName = runner?.name ?? L("its Runner")
        let problem = routine.problem
        // A routine stopped at its limits runs again only once the user resumes it in Limits,
        // which comes before anything else that's wrong with it.
        let budget = store.budget("routine", routineID, runnerID: bot.runnerID)
        let stopped = budget.flatMap { $0.isStopped ? $0 : nil }
        let state: (String, NSColor) =
            if let stopped {
                (stopped.stoppedLabel, .systemOrange)
            } else if routine.isRunning {
                (L("Running…"), .controlAccentColor)
            } else if let problem {
                (problem.text, problem.needsUser ? .systemOrange : .secondaryLabelColor)
            } else if routine.isEnabled {
                (L("On"), .systemGreen)
            } else {
                (routine.pausedReason == "away" ? L("Paused while you were away") : L("Paused"), .secondaryLabelColor)
            }
        var rows: [NSView] = [KeyValueRow(key: L("State"), value: state.0, tint: state.1)]
        if let stopped {
            rows.append(NoteRow(text: stopped.stoppedDetail))
        } else if let problem {
            rows.append(NoteRow(text: problem.explanation(bot: bot.name, runner: runnerName)))
        }
        let scheduleRow = KeyValueRow(key: L("Schedule"), value: routine.scheduleSummary)
        scheduleRow.toolTip = routine.schedule.hasPrefix("every ") ? routine.schedule : "\(routine.schedule) · \(routine.timezone)"
        rows.append(scheduleRow)
        if let events = routine.events {
            rows += eventRows(events)
        }
        // The calendar names its account.
        if let calendar = routine.calendar, !calendar.account.isEmpty {
            rows.append(KeyValueRow(key: L("Calendar"), value: calendar.account))
        }
        // A routine on events has no next run: its events start it, held on the relay while its
        // Runner is off, so it has no missed runs either.
        if routine.events == nil {
            // "Tomorrow 9:00 AM" on a line of its own, as the last check and run read; a routine
            // around events names the event it runs for.
            var next = routine.nextRunAt.map { Format.upcoming($0) }.map { $0.prefix(1).uppercased() + $0.dropFirst() }
            if let title = routine.calendar?.nextEventTitle, let when = next { next = "\(when) · \(title)" }
            let none = routine.calendar != nil && routine.isEnabled ? L("None in the next day") : "—"
            rows.append(KeyValueRow(key: routine.looksFirst ? L("Next check") : L("Next run"), value: next ?? none))
        }
        // A one-time routine runs once its Runner is back, whatever the policy.
        if routine.onceAt == nil && routine.events == nil {
            let skips = routine.missedRunPolicy == "skip"
            let missedRow = KeyValueRow(key: L("Missed runs"), value: skips ? L("Skip") : L("Run once"))
            missedRow.toolTip = skips
                ? L("When %@ was off at a scheduled time, the routine waits for the next one.", runnerName)
                : L("When %@ was off at a scheduled time, the routine runs once when it’s back.", runnerName)
            rows.append(missedRow)
        }
        if let lastCheck = routine.lastCheckSummary {
            rows.append(KeyValueRow(key: L("Last check"), value: lastCheck))
            // Only a failing check has a success to tell apart from it.
            if ["failed", "blocked"].contains(routine.health.status) {
                rows.append(KeyValueRow(key: L("Last successful check"), value: routine.health.lastSuccessAt.map { Format.daySeparator($0) } ?? L("Never")))
            }
        }
        rows.append(KeyValueRow(key: L("Last run"), value: routine.lastRunSummary))
        rows.append(limitsRow(budget, routine: routine))
        schedule.setRows(rows)
        showWebhook(routine.events)
        if prompt.string != routine.prompt { prompt.string = routine.prompt }
        checkSection.isHidden = routine.check == nil
        if check.string != (routine.check ?? "") { check.string = routine.check ?? "" }
        pauseButton.title = routine.isEnabled ? L("Pause") : L("Resume")
        // A run needs its Runner online, a routine paused by failed sign-ins needs Resume, and one
        // stopped at its limits resumes in Limits.
        runButton.isEnabled = !routine.isRunning && routine.state != "waiting_for_runner" && routine.pausedReason != "authentication" && stopped == nil
        runButton.toolTip = runner.map { L("Runs on %@ now", $0.name) } ?? L("Runs on the bot's Runner now")
        fitSheetToContent()
    }

    /// What a routine on events listens to: its pull request, which opens on GitHub; how it hears
    /// GitHub, where an App that isn't installed yet installs from; and the latest event.
    private func eventRows(_ events: RoutineEvents) -> [NSView] {
        var rows: [NSView] = []
        if !events.subject.isEmpty {
            let row = KeyValueRow(key: events.isGitHub ? L("Pull request") : events.sourceName, value: events.title.isEmpty ? events.subject : events.title)
            if let url = events.url {
                row.toolTip = url.absoluteString
                row.addGestureRecognizer(ClickHandler { NSWorkspace.shared.open(url) })
            }
            rows.append(row)
        }
        let service = events.isGitHub ? "GitHub" : (events.sourceName.isEmpty ? events.receiver : events.sourceName)
        switch events.status {
        case "needs_setup":
            let row = KeyValueRow(key: service, value: events.isGitHub ? L("Install App…") : L("Set Up…"), tint: .systemOrange)
            row.toolTip = L("Opens the page that sets it up for this account")
            row.addGestureRecognizer(ClickHandler { [weak self] in self?.openSetup(events) })
            rows.append(row)
        case "gateway":
            rows.append(KeyValueRow(key: service, value: L("Your gateway")))
            rows.append(NoteRow(text: L("This relay doesn’t take %@’s events itself, so they come through a gateway you run. %@ said how to set it up in the chat.", service, bot.name)))
        case "pending" where !events.isWebhook || events.endpoint.isEmpty:
            rows.append(KeyValueRow(key: service, value: L("Connecting…")))
        default:
            if !events.isWebhook { rows.append(KeyValueRow(key: service, value: L("Connected"))) }
        }
        let last = events.lastEvent.map { L("%@ · %@", Format.daySeparator($0.at), $0.summary) } ?? L("None yet")
        rows.append(KeyValueRow(key: L("Last event"), value: last))
        return rows
    }

    /// A routine's webhook: its URL, its key (hidden until a click shows it), and the header a
    /// sender pastes, each copied by its row's Copy; Regenerate makes a new key.
    private func showWebhook(_ events: RoutineEvents?) {
        guard let events, !events.endpoint.isEmpty else {
            webhook.isHidden = true
            return
        }
        webhook.isHidden = false
        let hidden = String(repeating: "•", count: 12)
        let url = ActionRow(key: L("Webhook URL"), value: events.endpoint, tint: .labelColor, actionTitle: L("Copy"))
        url.toolTip = events.endpoint
        url.onAction = { [weak url] in Self.copy(events.endpoint); url?.showCopied() }
        let key = ActionRow(key: L("Webhook key"), value: keyShown ? events.key : hidden, tint: .labelColor, actionTitle: L("Copy"), secondActionTitle: L("Regenerate…"))
        key.toolTip = keyShown ? L("Click to hide the key") : L("Click to show the key")
        key.onAction = { [weak key] in Self.copy(events.key); key?.showCopied() }
        key.onSecondAction = { [weak self] in self?.confirmRegenerate() }
        key.addGestureRecognizer(ClickHandler { [weak self] in
            self?.keyShown.toggle()
            self?.refresh()
        })
        let header = ActionRow(key: L("Authorization header"), value: "Bearer \(keyShown ? events.key : hidden)", tint: .labelColor, actionTitle: L("Copy"))
        header.toolTip = L("The header line that carries the key in every request, ready to paste.")
        header.onAction = { [weak header] in Self.copy(events.authorizationHeader); header?.showCopied() }
        webhook.setRows([url, key, header, NoteRow(text: L("Each request to this URL runs the routine once, with what it sent. Senders include the key in an Authorization: Bearer header; share it only with the service that calls this routine."))])
    }

    private static func copy(_ text: String) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }

    private func openSetup(_ events: RoutineEvents) {
        Task { @MainActor in
            do {
                NSWorkspace.shared.open(try await store.receiverSetupURL(events.receiver, subject: events.subject))
            } catch {
                _ = presentError(error)
            }
        }
    }

    private func confirmRegenerate() {
        guard let window = view.window else { return }
        let alert = NSAlert()
        alert.messageText = L("Regenerate the webhook key?")
        alert.informativeText = L("Services that send the current key stop reaching this routine until you give them the new one.")
        alert.addButton(withTitle: L("Regenerate Key"))
        alert.addButton(withTitle: L("Cancel"))
        alert.beginSheetModal(for: window) { [weak self] response in
            guard let self, response == .alertFirstButtonReturn else { return }
            Task { @MainActor in
                do {
                    try await self.store.regenerateRoutineKey(self.routineID)
                    self.keyShown = true
                    self.refresh()
                } catch {
                    _ = self.presentError(error)
                }
            }
        }
    }

    /// What its runs may use; opens the routine's Limits, where a stopped routine resumes.
    private func limitsRow(_ budget: BudgetState?, routine: Routine) -> NSView {
        let row = DisclosureRow(key: L("Limits"))
        row.setValue(budget?.limits.summary ?? L("None", context: "limits"))
        row.onClick = { [weak self] in
            guard let self, let dm = self.store.chats.first(where: { $0.isBotDM && $0.botIDs.contains(self.bot.id) }) else { return }
            self.presentAsSheet(BudgetViewController(bot: self.bot, chatID: dm.id, routine: routine))
        }
        return row
    }

    @objc private func runNow() {
        store.runRoutine(routineID)
    }

    @objc private func togglePaused() {
        guard let routine = store.routine(routineID) else { return }
        store.setRoutineEnabled(routineID, !routine.isEnabled)
    }

    @objc private func editInChat() {
        guard let routine = store.routine(routineID) else { return }
        onEditInChat?(L("Edit my routine \"%@\": ", routine.name))
        dismiss(nil)
    }

    @objc private func confirmDelete() {
        guard let routine = store.routine(routineID), let window = view.window else { return }
        let alert = NSAlert()
        alert.messageText = L("Delete “%@”?", routine.name)
        alert.informativeText = L("This deletes the routine and stops its future runs. This can't be undone.")
        alert.addButton(withTitle: L("Delete routine"))
        alert.addButton(withTitle: L("Cancel"))
        alert.alertStyle = .warning
        alert.beginSheetModal(for: window) { [weak self] response in
            guard let self, response == .alertFirstButtonReturn else { return }
            self.store.deleteRoutine(self.routineID)
            self.dismiss(nil)
        }
    }
}
