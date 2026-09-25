import AppKit

/// A chat's running tasks, in a popover under the toolbar's Running tasks button: the commands
/// its bots have running in their terminals (`AppStore.runningCommands`). The working row only
/// says what a bot is doing, and a command's card shows only once the bot hands the command over;
/// here each one shows what it does in the bot's words, who runs it and for how long, the command
/// (a click shows all of it), its last lines, and Stop. One that ends while the popover is open
/// stays, saying how it ended, until the popover closes.
final class RunningTasksViewController: NSViewController {
    static let width: CGFloat = 420
    private static let maxHeight: CGFloat = 480

    let chatID: Chat.ID
    private let store = AppStore.shared
    private let scrollView = NSScrollView()
    private let document = FlippedView()
    private(set) var tasks: [RunningTask] = []
    private var rows: [Message.ID: RunningTaskView] = [:]
    private var separators: [HairlineView] = []
    /// Every command listed since the popover opened, so one that ends stays.
    private var listed: Set<Message.ID> = []
    /// Why a Stop did not go through, by row.
    private var errors: [Message.ID: String] = [:]
    private var clock: Timer?

    /// A command's block was clicked: show the whole command under the title.
    var onShowCommand: ((_ title: String, _ command: String) -> Void)?
    /// Nothing is left to show: the commands' rows are gone.
    var onEmpty: (() -> Void)?

    init(chatID: Chat.ID) {
        self.chatID = chatID
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        scrollView.drawsBackground = false
        scrollView.hasVerticalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.documentView = document
        view = scrollView
        store.observe(self) { [weak self] event in self?.handle(event) }
        reload()
    }

    override func viewWillAppear() {
        super.viewWillAppear()
        // The running time ticks while the popover shows.
        clock?.invalidate()
        clock = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.tick() }
        }
    }

    override func viewDidDisappear() {
        super.viewDidDisappear()
        clock?.invalidate()
        clock = nil
    }

    private func handle(_ event: StoreEvent) {
        switch event {
        case let .messageAdded(id, _) where id == chatID, let .messageChanged(id, _) where id == chatID,
            let .messageRemoved(id, _) where id == chatID, let .runningTasksChanged(id) where id == chatID:
            reload()
        case .snapshotReplaced, .rosterChanged, .chatsChanged:
            reload()
        default:
            break
        }
    }

    /// The running tasks, and those listed before that have ended since, in the order they
    /// started.
    private func reload() {
        var tasks: [RunningTask] = []
        if let chat = store.chat(chatID) {
            let running = Set(store.runningCommands(in: chatID).map(\.id))
            for message in chat.messages {
                guard case let .tool(tool) = message.body, let run = tool.run,
                    running.contains(message.id) || (listed.contains(message.id) && run.hasEnded)
                else { continue }
                listed.insert(message.id)
                tasks.append(RunningTask(
                    id: message.id,
                    title: tool.description ?? run.firstLine,
                    botName: message.author.botID.flatMap(store.bot)?.name ?? L("The bot"),
                    showsBotName: chat.isGroup,
                    command: run.command,
                    firstLine: run.firstLine,
                    output: run.output ?? "",
                    state: run.state,
                    startedAt: message.createdAt))
            }
        }
        // What went wrong with a Stop matters while the command runs.
        errors = errors.filter { id, _ in tasks.contains { $0.id == id && $0.isLive } }
        guard tasks != self.tasks else { return }
        let hadTasks = !self.tasks.isEmpty
        self.tasks = tasks
        apply()
        if tasks.isEmpty, hadTasks { onEmpty?() }
    }

    /// Configures a row for each task, drops the others, and lays them out.
    private func apply() {
        let ids = Set(tasks.map(\.id))
        for (id, row) in rows where !ids.contains(id) {
            row.removeFromSuperview()
            rows[id] = nil
        }
        let now = Date()
        for task in tasks {
            let row = rows[task.id] ?? makeRow()
            rows[task.id] = row
            row.configure(task, error: errors[task.id], now: now)
        }
        layoutRows()
    }

    private func makeRow() -> RunningTaskView {
        let row = RunningTaskView()
        row.onStop = { [weak self] id in
            guard let self else { return }
            do {
                try await store.stopCommand(chatID: chatID, messageID: id)
                errors[id] = nil
            } catch {
                errors[id] = error.localizedDescription
            }
            apply()
        }
        row.onShowCommand = { [weak self] task in
            self?.onShowCommand?(L("%@'s command", task.botName), task.command)
        }
        document.addSubview(row)
        return row
    }

    /// Stacks the rows with a hairline between two, and sizes the popover to them, up to
    /// `maxHeight`, past which they scroll.
    private func layoutRows() {
        let width = Self.width
        var y: CGFloat = 0
        var used = 0
        for (index, task) in tasks.enumerated() {
            guard let row = rows[task.id] else { continue }
            if index > 0 {
                if separators.count == used {
                    let line = HairlineView()
                    line.translatesAutoresizingMaskIntoConstraints = true
                    document.addSubview(line)
                    separators.append(line)
                }
                separators[used].frame = NSRect(x: RunningTaskView.inset, y: y, width: width - RunningTaskView.inset * 2, height: 1)
                separators[used].isHidden = false
                used += 1
                y += 1
            }
            let height = RunningTaskView.height(of: task, error: errors[task.id], width: width)
            row.frame = NSRect(x: 0, y: y, width: width, height: height)
            y += height
        }
        for line in separators[used...] { line.isHidden = true }
        document.frame = NSRect(x: 0, y: 0, width: width, height: y)
        preferredContentSize = NSSize(width: width, height: min(y, Self.maxHeight))
    }

    private func tick() {
        let now = Date()
        for row in rows.values { row.tick(now) }
    }
}

/// What a row of Running tasks shows.
struct RunningTask: Equatable {
    var id: Message.ID
    /// What the command does, in the bot's words: "Install dependencies".
    var title: String
    var botName: String
    /// In a group: whose command it is.
    var showsBotName: Bool
    var command: String
    var firstLine: String
    var output: String
    var state: CommandRun.State
    var startedAt: Date

    var isLive: Bool { state == .running || state == .waiting }

    /// "Scout · Running · 1:05", or how it ended: "Finished".
    func status(at now: Date) -> String {
        var parts = showsBotName ? [botName] : []
        switch state {
        case .running: parts += [L("Running"), Self.elapsed(since: startedAt, now: now)]
        case .waiting: parts += [L("Waiting for input"), Self.elapsed(since: startedAt, now: now)]
        case .exited: parts.append(L("Finished"))
        case .failed: parts.append(L("Failed"))
        default: parts.append(L("Stopped"))
        }
        return parts.joined(separator: " · ")
    }

    /// "0:42", "12:03", "1:02:03".
    static func elapsed(since start: Date, now: Date) -> String {
        let seconds = max(0, Int(now.timeIntervalSince(start)))
        let (hours, minutes, rest) = (seconds / 3600, seconds % 3600 / 60, seconds % 60)
        return hours > 0 ? String(format: "%d:%02d:%02d", hours, minutes, rest) : String(format: "%d:%02d", minutes, rest)
    }
}

/// One running command: the terminal symbol and what it does, with Stop at the end of that line;
/// who runs it and for how long; the command on one line, which shows all of it on click; and its
/// last lines, which grow to `outputLines` and then scroll.
final class RunningTaskView: NSView {
    static let inset: CGFloat = 14
    private static let textX: CGFloat = 38
    private static let top: CGFloat = 12
    private static let outputLines = 10
    private static let titleFont = NSFont.systemFont(ofSize: 12.5, weight: .semibold)
    private static let statusFont = NSFont.systemFont(ofSize: 11)
    private static let commandFont = NSFont.monospacedSystemFont(ofSize: 11, weight: .regular)
    private static let commandPadding = NSSize(width: 8, height: 5)
    private static let controlHeight: CGFloat = 22

    /// The row's height for `task` at `width`, with what went wrong with a Stop under it.
    static func height(of task: RunningTask, error: String?, width: CGFloat) -> CGFloat {
        Layout(task: task, error: error, width: width).height
    }

    /// Where each part sits.
    @MainActor private struct Layout {
        var title: NSRect
        var status: NSRect
        var command: NSRect
        var commandText: String
        var output: NSRect?
        var error: NSRect?
        var height: CGFloat

        init(task: RunningTask, error: String?, width: CGFloat) {
            let textWidth = width - RunningTaskView.textX - RunningTaskView.inset
            let columnWidth = textWidth - RunningTaskView.commandPadding.width * 2
            let font = RunningTaskView.commandFont
            title = NSRect(x: RunningTaskView.textX, y: RunningTaskView.top, width: textWidth, height: 17)
            status = NSRect(x: RunningTaskView.textX, y: title.maxY + 1, width: textWidth, height: 15)
            let line = TextMeasure.labelSize(of: NSAttributedString(string: "X", attributes: [.font: font]), width: columnWidth).height
            command = NSRect(x: RunningTaskView.textX, y: status.maxY + 7, width: textWidth, height: line + RunningTaskView.commandPadding.height * 2)
            commandText = CommandCellView.commandLine(task.firstLine, fitting: columnWidth, font: font)
            var bottom = command.maxY
            if !task.output.isEmpty {
                let height = CommandOutputView.height(of: task.output, width: textWidth, lines: RunningTaskView.outputLines)
                output = NSRect(x: RunningTaskView.textX, y: bottom + 6, width: textWidth, height: height)
                bottom += 6 + height
            }
            if let error {
                let text = NSAttributedString(string: error, attributes: [.font: RunningTaskView.statusFont])
                let height = min(TextMeasure.labelSize(of: text, width: textWidth).height, 30)
                self.error = NSRect(x: RunningTaskView.textX, y: bottom + 6, width: textWidth, height: height)
                bottom += 6 + height
            }
            height = bottom + RunningTaskView.top
        }
    }

    private let icon = NSImageView()
    private let title = Build.label("", font: RunningTaskView.titleFont)
    private let status = Build.label("", font: RunningTaskView.statusFont, color: .secondaryLabelColor)
    private let command = CommandBlockView(font: RunningTaskView.commandFont, lines: 1, padding: RunningTaskView.commandPadding)
    private let output = CommandOutputView()
    private let stopButton = NSButton()
    /// Why a Stop did not go through.
    private let errorLabel = Build.label("", font: RunningTaskView.statusFont, color: .systemRed, lines: 2)
    private var task: RunningTask?
    private var error: String?
    private var busy = false { didSet { stopButton.isEnabled = !busy } }

    /// Stops the command in row `id`.
    var onStop: ((_ id: Message.ID) async -> Void)?
    var onShowCommand: ((RunningTask) -> Void)?

    init() {
        super.init(frame: .zero)
        icon.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 14, weight: .medium)
        icon.image = NSImage(systemSymbolName: "terminal", accessibilityDescription: nil)
        icon.contentTintColor = .controlAccentColor
        command.onClick = { [weak self] in
            guard let self, let task else { return }
            onShowCommand?(task)
        }
        stopButton.title = L("Stop")
        stopButton.bezelStyle = .rounded
        stopButton.controlSize = .small
        stopButton.font = .systemFont(ofSize: 11)
        stopButton.target = self
        stopButton.action = #selector(stop(_:))
        for view: NSView in [icon, title, status, command, output, stopButton, errorLabel] {
            addSubview(view.framePositioned())
        }
        setAccessibilityElement(true)
        setAccessibilityRole(.group)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { true }

    func configure(_ task: RunningTask, error: String?, now: Date) {
        let previous = self.task
        if previous?.id != task.id { busy = false }
        if previous?.id != task.id || previous?.output != task.output {
            output.text = task.output
        }
        self.task = task
        self.error = error
        title.stringValue = task.title
        title.toolTip = task.title
        stopButton.isHidden = !task.isLive
        errorLabel.stringValue = error ?? ""
        setAccessibilityLabel(task.title)
        tick(now)
        needsLayout = true
    }

    /// The status line, whose running time moves on.
    func tick(_ now: Date) {
        guard let task else { return }
        let text = task.status(at: now)
        if status.stringValue != text { status.stringValue = text }
    }

    @objc private func stop(_ sender: Any?) {
        guard let onStop, let id = task?.id, !busy else { return }
        busy = true
        Task { @MainActor in
            await onStop(id)
            busy = false
        }
    }

    override func layout() {
        super.layout()
        guard let task else { return }
        let layout = Layout(task: task, error: error, width: bounds.width)
        icon.frame = NSRect(x: Self.inset, y: Self.top - 1, width: 18, height: 18)
        title.frame = layout.title
        if !stopButton.isHidden {
            // Stop ends the title's line, as on the command's card.
            let size = stopButton.intrinsicContentSize
            stopButton.frame = NSRect(x: bounds.width - Self.inset - size.width - 4, y: Self.top - 3, width: size.width + 4, height: Self.controlHeight)
            title.frame.size.width = stopButton.frame.minX - 8 - title.frame.minX
        }
        status.frame = layout.status
        command.frame = layout.command
        command.text = layout.commandText
        output.isHidden = layout.output == nil
        output.frame = layout.output ?? .zero
        errorLabel.isHidden = layout.error == nil
        errorLabel.frame = layout.error ?? .zero
    }
}
