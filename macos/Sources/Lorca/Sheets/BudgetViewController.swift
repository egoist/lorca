import AppKit

/// What a DM's turns, a task's runs, or a routine's runs may use on the bot's Runner: spending,
/// tokens, run time, retries, and plugin calls. Work that stopped at a limit shows why on a card
/// with Resume; raising the limit it reached and resuming goes on where it stopped.
final class BudgetViewController: SheetViewController {
    private enum Field: CaseIterable {
        case usd, tokens, runtime, retries, connectorCalls

        var title: String {
            switch self {
            case .usd: L("Spending")
            case .tokens: L("Tokens")
            case .runtime: L("Run time")
            case .retries: L("Retries")
            case .connectorCalls: L("Plugin calls")
            }
        }

        var unit: String {
            switch self {
            case .usd: "USD"
            case .runtime: L("minutes")
            default: ""
            }
        }

        var reached: String {
            switch self {
            case .usd: "usd"
            case .tokens: "tokens"
            case .runtime: "runtime"
            case .retries: "retries"
            case .connectorCalls: "connector_calls"
            }
        }
    }

    private let store = AppStore.shared
    private let bot: Bot
    private let chatID: Chat.ID
    /// What the limits are for: `chat` for each new turn in the DM, `task`, or `routine`.
    private let scope: (kind: String, id: String)
    private var fields: [Field: NSTextField] = [:]
    private var used: [Field: NSTextField] = [:]
    private let card = BackgroundView()
    private let cardTitle = Build.label("", font: .systemFont(ofSize: 13, weight: .semibold))
    private let cardDetail = Build.label("", font: .systemFont(ofSize: 12), color: .secondaryLabelColor, lines: 0)
    private let resumeButton = NSButton(title: L("Resume"), target: nil, action: nil)
    private let errorLabel = Build.label("", font: .systemFont(ofSize: 12), color: .systemRed, lines: 0)
    private static let width: CGFloat = 460

    /// A DM's limits for each new turn, and its newest turn when that one stopped.
    init(bot: Bot, chatID: Chat.ID) {
        self.bot = bot
        self.chatID = chatID
        scope = ("chat", chatID)
        super.init(title: L("Limits"), subtitle: L("Each turn with %@ stops when it reaches one of these.", bot.name), width: Self.width)
    }

    /// A routine's limits, which all of its runs count toward.
    init(bot: Bot, chatID: Chat.ID, routine: Routine) {
        self.bot = bot
        self.chatID = chatID
        scope = ("routine", routine.id)
        super.init(title: L("Limits"), subtitle: L("All runs of %@ count toward these. When it reaches one, it waits until you resume it.", routine.name), width: Self.width)
    }

    /// A task's limits, which all of its runs count toward; `bot` is its owner.
    init(bot: Bot, task: DurableTask) {
        self.bot = bot
        chatID = task.chatIds.first ?? ""
        scope = ("task", task.id)
        super.init(title: L("Limits"), subtitle: L("All runs of this task count toward these. When it reaches one, the task waits until you resume it."), width: Self.width)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    /// The allowance the form edits: the task's or routine's, or the DM's for new turns.
    private var configured: BudgetState? { store.budget(scope.kind, scope.id, runnerID: bot.runnerID) }

    /// What stopped and waits for Resume: the task or routine, or the DM's newest turn.
    private var stopped: BudgetState? {
        if scope.kind != "chat" { return configured?.isStopped == true ? configured : nil }
        return store.stoppedTurn(in: chatID, runnerID: bot.runnerID)
    }

    /// Whose use the form shows beside the limits.
    private var usage: BudgetState.Usage? { scope.kind != "chat" ? configured?.usage : stopped?.usage }

    override func loadView() {
        super.loadView()
        buildCard()
        let grid = NSGridView(numberOfColumns: 3, rows: 0)
        grid.rowSpacing = 8
        grid.columnSpacing = 8
        grid.rowAlignment = .firstBaseline
        for field in Field.allCases {
            let label = Build.label(field.title, font: .systemFont(ofSize: 12), color: .secondaryLabelColor)
            let input = NSTextField()
            input.placeholderString = L("No limit")
            input.alignment = .right
            input.font = .systemFont(ofSize: 12)
            input.setAccessibilityLabel(field.title)
            input.translatesAutoresizingMaskIntoConstraints = false
            input.widthAnchor.constraint(equalToConstant: 100).isActive = true
            let unit = Build.label(field.unit, font: .systemFont(ofSize: 12), color: .secondaryLabelColor)
            let use = Build.label("", font: .systemFont(ofSize: 12), color: .secondaryLabelColor, alignment: .right)
            use.setContentHuggingPriority(.defaultLow, for: .horizontal)
            fields[field] = input
            used[field] = use
            // The field and its unit keep their width; what it used takes the rest of the line.
            let entry = Build.stack([input, unit], orientation: .horizontal, spacing: 6)
            entry.setHuggingPriority(.required, for: .horizontal)
            use.setContentCompressionResistancePriority(.required, for: .horizontal)
            grid.addRow(with: [label, entry, use])
        }
        grid.column(at: 0).width = 76
        // The form is as tall as its rows: a sheet with room to spare keeps it below the form,
        // never between two rows.
        grid.heightAnchor.constraint(equalToConstant: grid.fittingSize.height).isActive = true
        grid.column(at: 2).xPlacement = .fill
        grid.translatesAutoresizingMaskIntoConstraints = false
        contentStack.addArrangedSubview(card)
        contentStack.addArrangedSubview(grid)
        contentStack.addArrangedSubview(errorLabel)
        for view in [card, grid, errorLabel] {
            view.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
        }
        errorLabel.isHidden = true
        setButtons(confirm: L("Save"))
        fill(configured?.limits ?? BudgetLimits())
        refresh()
        store.observe(self) { [weak self] event in
            if case .budgetsChanged = event { self?.refresh() }
        }
    }

    private func buildCard() {
        card.fillColor = Theme.botBubble
        card.borderColor = Theme.botBubbleBorder
        card.cornerRadius = 9
        card.translatesAutoresizingMaskIntoConstraints = false
        let icon = NSImageView(image: NSImage(systemSymbolName: "exclamationmark.circle.fill", accessibilityDescription: nil) ?? NSImage())
        icon.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 15, weight: .regular)
        icon.contentTintColor = .systemOrange
        icon.translatesAutoresizingMaskIntoConstraints = false
        icon.setContentHuggingPriority(.required, for: .horizontal)
        resumeButton.bezelStyle = .rounded
        resumeButton.setContentHuggingPriority(.required, for: .horizontal)
        resumeButton.target = self
        resumeButton.action = #selector(resume)
        resumeButton.translatesAutoresizingMaskIntoConstraints = false
        resumeButton.setContentCompressionResistancePriority(.required, for: .horizontal)
        // The detail wraps at the width the card leaves it, known before the sheet is laid out,
        // so the sheet is sized for the lines it takes.
        cardDetail.preferredMaxLayoutWidth = Self.width - 40 - 12 - icon.fittingSize.width - 10 - 12 - resumeButton.fittingSize.width - 12
        let text = Build.stack([cardTitle, cardDetail], spacing: 2)
        text.translatesAutoresizingMaskIntoConstraints = false
        for view in [icon, text, resumeButton] { card.addSubview(view) }
        NSLayoutConstraint.activate([
            icon.leadingAnchor.constraint(equalTo: card.leadingAnchor, constant: 12),
            icon.centerYAnchor.constraint(equalTo: card.centerYAnchor),
            text.leadingAnchor.constraint(equalTo: icon.trailingAnchor, constant: 10),
            text.topAnchor.constraint(equalTo: card.topAnchor, constant: 10),
            text.bottomAnchor.constraint(equalTo: card.bottomAnchor, constant: -10),
            // The text takes the card's width up to Resume, so it wraps where the card does.
            resumeButton.leadingAnchor.constraint(equalTo: text.trailingAnchor, constant: 12),
            resumeButton.trailingAnchor.constraint(equalTo: card.trailingAnchor, constant: -12),
            resumeButton.centerYAnchor.constraint(equalTo: card.centerYAnchor),
        ])
    }

    /// The card and the use beside each limit follow the Runner; what the user typed stays.
    private func refresh() {
        guard isViewLoaded else { return }
        if let stopped {
            cardTitle.stringValue = stopped.stoppedTitle
            cardDetail.stringValue = stopped.stoppedDetail
            card.isHidden = false
        } else {
            card.isHidden = true
        }
        for field in Field.allCases {
            let text = usage.map { used(field, $0) } ?? ""
            used[field]?.stringValue = text
            used[field]?.textColor = stopped?.reached == field.reached ? .systemOrange : .secondaryLabelColor
        }
        fitSheetToContent()
    }

    private func used(_ field: Field, _ usage: BudgetState.Usage) -> String {
        switch field {
        case .usd:
            guard usage.apiCostUsd > 0 || usage.subscriptionEstimateUsd > 0 else { return usage.unknownPriceCalls > 0 ? L("Price unknown") : "" }
            return L("%@ used", Format.spend(api: usage.apiCostUsd, estimate: usage.subscriptionEstimateUsd))
        case .tokens: return usage.tokens > 0 ? L("%@ used", Format.count(usage.tokens)) : ""
        case .runtime: return usage.runtimeSecs >= 1 ? L("%@ used", Format.minutes(Int(usage.runtimeSecs))) : ""
        case .retries: return usage.retries > 0 ? L("%@ used", Format.count(usage.retries)) : ""
        case .connectorCalls: return usage.connectorCalls > 0 ? L("%@ used", Format.count(usage.connectorCalls)) : ""
        }
    }

    // MARK: - Fields

    private static func number(fractionDigits: Int) -> NumberFormatter {
        let formatter = NumberFormatter()
        formatter.numberStyle = .decimal
        formatter.maximumFractionDigits = fractionDigits
        return formatter
    }

    private func fill(_ limits: BudgetLimits) {
        let money = Self.number(fractionDigits: 2)
        money.minimumFractionDigits = 2
        let whole = Self.number(fractionDigits: 0)
        let minutes = Self.number(fractionDigits: 1)
        fields[.usd]?.stringValue = limits.maxUsd.flatMap { money.string(from: $0 as NSNumber) } ?? ""
        fields[.tokens]?.stringValue = limits.maxTokens.flatMap { whole.string(from: $0 as NSNumber) } ?? ""
        fields[.runtime]?.stringValue = limits.maxRuntimeSecs.flatMap { minutes.string(from: Double($0) / 60 as NSNumber) } ?? ""
        fields[.retries]?.stringValue = limits.maxRetries.flatMap { whole.string(from: $0 as NSNumber) } ?? ""
        fields[.connectorCalls]?.stringValue = limits.maxConnectorCalls.flatMap { whole.string(from: $0 as NSNumber) } ?? ""
    }

    /// The limits as typed; nil after pointing at the field that isn't a number.
    private func limits() -> BudgetLimits? {
        let parser = Self.number(fractionDigits: 6)
        parser.isLenient = true
        var limits = BudgetLimits()
        for field in Field.allCases {
            guard let input = fields[field] else { continue }
            let text = input.stringValue.trimmingCharacters(in: .whitespaces).replacingOccurrences(of: "$", with: "")
            guard !text.isEmpty else { continue }
            guard let value = parser.number(from: text)?.doubleValue, value.isFinite, value >= 0,
                field == .usd || field == .runtime || value == value.rounded()
            else {
                showError(L("Enter a number, or leave it empty for no limit."), focus: input)
                return nil
            }
            switch field {
            case .usd: limits.maxUsd = value
            case .tokens: limits.maxTokens = Int(value)
            case .runtime:
                guard value <= 525_600 else {
                    showError(L("Run time can be at most a year, or leave it empty for no limit."), focus: input)
                    return nil
                }
                limits.maxRuntimeSecs = Int((value * 60).rounded())
            case .retries: limits.maxRetries = Int(value)
            case .connectorCalls: limits.maxConnectorCalls = Int(value)
            }
        }
        return limits
    }

    private func showError(_ text: String, focus: NSView? = nil) {
        errorLabel.stringValue = text
        errorLabel.isHidden = false
        if let focus { view.window?.makeFirstResponder(focus) }
        fitSheetToContent()
    }

    // MARK: - Actions

    override func confirmTapped() {
        guard let limits = limits() else { return }
        run { [self] in try await save(limits) }
    }

    /// Resumes where it stopped. When the limit it reached didn't go up, resuming means using
    /// the limits in full again, so that asks first.
    @objc private func resume() {
        guard let stopped, let limits = limits() else { return }
        guard stopped.fits(limits) else {
            guard let window = view.window else { return }
            let alert = NSAlert()
            alert.messageText = switch scope.kind {
            case "routine": L("Resume this routine with fresh limits?")
            case "task": L("Resume this task with fresh limits?")
            default: L("Resume this turn with fresh limits?")
            }
            alert.informativeText = L("It already used its limits. Resuming lets it use them again in full.")
            alert.addButton(withTitle: L("Resume"))
            alert.addButton(withTitle: L("Cancel"))
            alert.beginSheetModal(for: window) { [weak self] response in
                guard response == .alertFirstButtonReturn, let self else { return }
                self.run { [self] in try await self.resume(stopped, limits: limits, fresh: true) }
            }
            return
        }
        run { [self] in try await resume(stopped, limits: limits, fresh: false) }
    }

    private func save(_ limits: BudgetLimits) async throws {
        try await store.setBudget(scope.kind, scope.id, limits: limits, bot: bot, chatID: chatID)
    }

    /// The stopped turn takes the limits in the form too, so raising one lets it go on.
    private func resume(_ stopped: BudgetState, limits: BudgetLimits, fresh: Bool) async throws {
        try await save(limits)
        if stopped.kind == "job" {
            try await store.setBudget("job", stopped.id, limits: limits, bot: bot, chatID: chatID)
        }
        try await store.resumeBudget(stopped.kind, stopped.id, bot: bot, fresh: fresh)
    }

    /// Runs a change on the Runner with the buttons off, and closes the sheet once it's done.
    private func run(_ change: @escaping () async throws -> Void) {
        errorLabel.isHidden = true
        confirmButton.isEnabled = false
        resumeButton.isEnabled = false
        Task { [weak self] in
            do {
                try await change()
                self?.dismiss(nil)
            } catch {
                guard let self else { return }
                self.confirmButton.isEnabled = true
                self.resumeButton.isEnabled = true
                self.showError(error.localizedDescription)
            }
        }
    }
}
