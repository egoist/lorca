import AppKit

/// One routine's details, after Grok Bot's routine page: the schedule and its next and last
/// runs, the task, and the actions on it. Run Now starts it on the bot's Runner; Pause and
/// Resume flip the switch the inspector shows; Edit in Chat hands the bot the change to make,
/// since the bot owns its routines; Delete asks first.
final class RoutineViewController: SheetViewController {
    private let store = AppStore.shared
    private let routineID: Routine.ID
    private let bot: Bot

    private let schedule = SectionView(title: "Schedule")
    private let task = SectionView(title: "Task")
    private let prompt = NSTextView()
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
            title: routine?.name ?? "Routine",
            subtitle: "A task \(bot.name) runs on its own in this chat. \(bot.name) set it up and can change it: ask in chat.",
            width: 520
        )
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()

        prompt.isEditable = false
        prompt.isRichText = false
        prompt.font = .systemFont(ofSize: 12)
        prompt.textColor = .labelColor
        prompt.drawsBackground = false
        prompt.textContainerInset = NSSize(width: 8, height: 8)
        prompt.isVerticallyResizable = true
        prompt.isHorizontallyResizable = false
        prompt.autoresizingMask = [.width]
        prompt.textContainer?.widthTracksTextView = true
        let scroll = NSScrollView()
        scroll.documentView = prompt
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.drawsBackground = false
        scroll.translatesAutoresizingMaskIntoConstraints = false
        task.setRows([scroll])

        for (button, title, action) in [
            (runButton, "Run Now", #selector(runNow)),
            (pauseButton, "Pause", #selector(togglePaused)),
            (editButton, "Edit in Chat…", #selector(editInChat)),
            (deleteButton, "Delete…", #selector(confirmDelete)),
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
        contentStack.addArrangedSubview(actions)
        NSLayoutConstraint.activate([
            schedule.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            task.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            actions.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            scroll.heightAnchor.constraint(equalToConstant: 96),
        ])

        setButtons(confirm: "Done", cancel: nil)
        refresh()
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
        let state: (String, NSColor) =
            routine.isRunning
            ? ("Running…", .controlAccentColor)
            : routine.isEnabled ? ("On", .systemGreen) : (routine.pausedReason == "away" ? "Paused while you were away" : "Paused", .secondaryLabelColor)
        let scheduleRow = KeyValueRow(key: "Schedule", value: routine.scheduleText)
        scheduleRow.toolTip = routine.schedule
        schedule.setRows([
            KeyValueRow(key: "State", value: state.0, tint: state.1),
            scheduleRow,
            KeyValueRow(key: "Next run", value: routine.nextRunAt.map { Format.upcoming($0) } ?? "—"),
            KeyValueRow(key: "Last run", value: routine.lastRunSummary),
        ])
        if prompt.string != routine.prompt { prompt.string = routine.prompt }
        pauseButton.title = routine.isEnabled ? "Pause" : "Resume"
        runButton.isEnabled = !routine.isRunning
        let runner = store.device(bot.runnerID)
        runButton.toolTip = runner.map { "Runs on \($0.name) now" } ?? "Runs on the bot's Runner now"
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
        onEditInChat?("Edit my routine \"\(routine.name)\": ")
        dismiss(nil)
    }

    @objc private func confirmDelete() {
        guard let routine = store.routine(routineID), let window = view.window else { return }
        let alert = NSAlert()
        alert.messageText = "Delete “\(routine.name)”?"
        alert.informativeText = "This deletes the routine and stops its future runs. This can't be undone."
        alert.addButton(withTitle: "Delete routine")
        alert.addButton(withTitle: "Cancel")
        alert.alertStyle = .warning
        alert.beginSheetModal(for: window) { [weak self] response in
            guard let self, response == .alertFirstButtonReturn else { return }
            self.store.deleteRoutine(self.routineID)
            self.dismiss(nil)
        }
    }
}
