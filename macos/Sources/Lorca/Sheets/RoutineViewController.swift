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
        contentStack.addArrangedSubview(task)
        contentStack.addArrangedSubview(checkSection)
        contentStack.addArrangedSubview(actions)
        NSLayoutConstraint.activate([
            schedule.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
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
            case .rosterChanged, .chatsChanged, .snapshotReplaced:
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
        let state: (String, NSColor) =
            if routine.isRunning {
                (L("Running…"), .controlAccentColor)
            } else if let problem {
                (problem.text, problem.needsUser ? .systemOrange : .secondaryLabelColor)
            } else if routine.isEnabled {
                (L("On"), .systemGreen)
            } else {
                (routine.pausedReason == "away" ? L("Paused while you were away") : L("Paused"), .secondaryLabelColor)
            }
        var rows: [NSView] = [KeyValueRow(key: L("State"), value: state.0, tint: state.1)]
        if let problem {
            rows.append(NoteRow(text: problem.explanation(bot: bot.name, runner: runnerName)))
        }
        let scheduleRow = KeyValueRow(key: L("Schedule"), value: routine.scheduleSummary)
        scheduleRow.toolTip = routine.schedule.hasPrefix("every ") ? routine.schedule : "\(routine.schedule) · \(routine.timezone)"
        let skips = routine.missedRunPolicy == "skip"
        let missedRow = KeyValueRow(key: L("Missed runs"), value: skips ? L("Skip") : L("Run once"))
        missedRow.toolTip = skips
            ? L("When %@ was off at a scheduled time, the routine waits for the next one.", runnerName)
            : L("When %@ was off at a scheduled time, the routine runs once when it’s back.", runnerName)
        // "Tomorrow 9:00 AM" on a line of its own, as the last check and run read.
        let next = routine.nextRunAt.map { Format.upcoming($0) }.map { $0.prefix(1).uppercased() + $0.dropFirst() }
        rows += [
            scheduleRow,
            KeyValueRow(key: routine.check == nil ? L("Next run") : L("Next check"), value: next ?? "—"),
            missedRow,
        ]
        if let lastCheck = routine.lastCheckSummary {
            rows.append(KeyValueRow(key: L("Last check"), value: lastCheck))
            // Only a failing check has a success to tell apart from it.
            if ["failed", "blocked"].contains(routine.health.status) {
                rows.append(KeyValueRow(key: L("Last successful check"), value: routine.health.lastSuccessAt.map { Format.daySeparator($0) } ?? L("Never")))
            }
        }
        rows.append(KeyValueRow(key: L("Last run"), value: routine.lastRunSummary))
        schedule.setRows(rows)
        if prompt.string != routine.prompt { prompt.string = routine.prompt }
        checkSection.isHidden = routine.check == nil
        if check.string != (routine.check ?? "") { check.string = routine.check ?? "" }
        pauseButton.title = routine.isEnabled ? L("Pause") : L("Resume")
        // A run needs its Runner online, and a routine paused by failed sign-ins needs Resume.
        runButton.isEnabled = !routine.isRunning && routine.state != "waiting_for_runner" && routine.pausedReason != "authentication"
        runButton.toolTip = runner.map { L("Runs on %@ now", $0.name) } ?? L("Runs on the bot's Runner now")
        fitSheetToContent()
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
