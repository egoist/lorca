import AppKit

/// Settings → Auto-review, after Grok Bot's: the switch and the rules, shared by every Device
/// through the roster. Add and Edit use sheets; a card's Always allow adds a structured rule here.
final class AutoReviewSettingsViewController: SettingsPaneViewController {
    private let store = AppStore.shared
    private let check = SectionView(title: L("Auto-review"))
    private let rules = SectionView(title: SettingsEntry.autoReviewRules.row)
    private let toggle = NSSwitch()

    override func viewDidLoad() {
        super.viewDidLoad()
        title = L("Auto-review")
        toggle.controlSize = .small
        toggle.target = self
        toggle.action = #selector(toggled)
        let add = HoverButton(
            symbol: "plus", pointSize: 11, tooltip: L("Add rule"), target: self,
            action: #selector(showAddRule))
        rules.setHeaderAccessory(add)
        addSection(check)
        addSection(rules)
        addFootnote(L("Auto-review checks effectful plugin actions and every shell command before they run. The parser identifies concrete risks; the bot's model applies your rules and latest request, so safe commands normally run automatically and risky commands ask. Off, every such action asks. Write one short, natural-language rule for each action; \"Ask first\" takes priority if rules conflict. Built-in safety checks always apply."))
        store.observe(self) { [weak self] event in
            switch event {
            case .rosterChanged, .snapshotReplaced: self?.render()
            default: break
            }
        }
        render()
    }

    private func render() {
        let review = store.autoReview
        toggle.state = review.isEnabled ? .on : .off
        let description = NoteRow(text: L("Lorca checks each action before it runs and asks you first when needed. Add rules to customize what bots can do automatically."))
        let switchRow = AccessoryRow(key: SettingsEntry.autoReviewSwitch.row, accessory: toggle)
        check.setRows([switchRow, description])

        var ruleRows: [NSView] = review.rules.map { rule in
            let content: NSView
            let scope: String?
            if rule.tool?.hasPrefix("computer/bash/") == true {
                let runner = rule.runnerID.flatMap { store.device($0) }?.name ?? L("Runner")
                scope = [runner, rule.workdir].compactMap { $0 }.joined(separator: " · ")
                content = ExactShellCommandView(command: rule.command ?? rule.text)
            } else {
                let label = Build.label(rule.text, font: .systemFont(ofSize: 12), lines: 0)
                label.isSelectable = true
                content = label
                scope = nil
            }
            let patternCount = rule.patterns.isEmpty
                ? nil
                : (rule.patterns.count == 1 ? L("1 pattern") : L("%d patterns", rule.patterns.count))
            let detail = [scope, patternCount, rule.behavior.title].compactMap { $0 }.joined(separator: " · ")
            let row = AutoReviewRuleRow(content: content, detail: detail)
            row.onEdit = { [weak self] in self?.showEditRule(rule) }
            row.onDelete = { [weak self] in self?.delete(rule.id) }
            return row
        }
        if ruleRows.isEmpty {
            ruleRows.append(NoteRow(text: L("No rules yet. Always allow on a card adds one, or write one below.")))
        }
        rules.setRows(ruleRows)
    }

    @objc private func toggled() {
        var review = store.autoReview
        review.isEnabled = toggle.state == .on
        store.setAutoReview(review)
    }

    @objc private func showAddRule() {
        presentRuleEditor(nil)
    }

    private func showEditRule(_ rule: AutoReviewRule) {
        presentRuleEditor(rule)
    }

    private func presentRuleEditor(_ rule: AutoReviewRule?) {
        let editor = AutoReviewRuleEditorViewController(rule: rule)
        editor.onSave = { [weak self] text, behavior in
            guard let self else { return }
            var review = self.store.autoReview
            if let rule, let index = review.rules.firstIndex(where: { $0.id == rule.id }) {
                var updated = review.rules[index]
                if updated.tool == nil { updated.text = text }
                updated.behavior = behavior
                review.rules[index] = updated
            } else {
                review.rules.append(AutoReviewRule(id: "", text: text, behavior: behavior))
            }
            self.store.setAutoReview(review)
        }
        presentAsSheet(editor)
    }

    private func delete(_ id: String) {
        var review = store.autoReview
        review.rules.removeAll { $0.id == id }
        store.setAutoReview(review)
    }
}

/// Add and Edit share one sheet. User-written rule text is editable; an exact tool or shell
/// identity stays fixed, while its Allow/Ask behavior can still change.
final class AutoReviewRuleEditorViewController: SheetViewController, NSTextFieldDelegate {
    private let rule: AutoReviewRule?
    private let text = NSTextField()
    private let behavior = SettingsPopUpButton()
    private let canEditText: Bool
    private let originalText: String

    var onSave: ((String, AutoReviewRule.Behavior) -> Void)?

    init(rule: AutoReviewRule?) {
        self.rule = rule
        canEditText = rule?.tool == nil
        originalText = rule?.text ?? ""
        let exact = rule?.tool != nil
        super.init(
            title: rule == nil ? L("Add rule") : L("Edit rule"),
            subtitle: exact
                ? L("Exact actions cannot be changed. Delete this rule and allow a different action instead.")
                : L("Describe when a bot should be allowed automatically or asked first."),
            width: 500
        )
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()

        let value = rule?.command ?? rule?.text ?? ""
        let ruleContent: NSView
        if let command = rule?.command {
            let ruleSection = SectionView(title: L("Command"))
            ruleSection.setRows([ExactShellCommandView(command: command, maxLines: 0)])
            ruleContent = ruleSection
        } else {
            text.stringValue = value
            text.isEditable = canEditText
            text.isSelectable = true
            text.font = .systemFont(ofSize: 12)
            text.textColor = .labelColor
            text.isBezeled = true
            text.bezelStyle = .roundedBezel
            text.drawsBackground = true
            text.backgroundColor = .textBackgroundColor
            text.maximumNumberOfLines = 0
            text.lineBreakMode = .byWordWrapping
            text.cell?.wraps = true
            text.cell?.usesSingleLineMode = false
            text.cell?.isScrollable = false
            text.delegate = self
            text.translatesAutoresizingMaskIntoConstraints = false
            let caption = Build.label(
                L("Rule").uppercased(), font: .systemFont(ofSize: 10, weight: .semibold),
                color: .tertiaryLabelColor)
            let field = Build.stack([caption, text], spacing: 6)
            text.widthAnchor.constraint(equalTo: field.widthAnchor).isActive = true
            text.heightAnchor.constraint(equalToConstant: 90).isActive = true
            ruleContent = field
        }

        for choice in [AutoReviewRule.Behavior.allow, .ask] {
            behavior.addItem(withTitle: choice.title)
            behavior.lastItem?.representedObject = choice.rawValue
        }
        behavior.selectItem(at: rule?.behavior == .ask ? 1 : 0)
        let behaviorSection = SectionView(title: L("Behavior"))
        behaviorSection.setRows([AccessoryRow(key: L("Auto-review"), accessory: behavior)])

        contentStack.addArrangedSubview(ruleContent)
        var fullWidth: [NSView] = [ruleContent]
        if let patterns = rule?.patterns, !patterns.isEmpty {
            let patternSection = SectionView(title: L("Allowed patterns"))
            patternSection.setRows([ExactShellCommandView(command: patterns.joined(separator: "\n"), maxLines: 0)])
            contentStack.addArrangedSubview(patternSection)
            fullWidth.append(patternSection)
        }
        contentStack.addArrangedSubview(behaviorSection)
        fullWidth.append(behaviorSection)
        NSLayoutConstraint.activate(fullWidth.map { $0.widthAnchor.constraint(equalTo: contentStack.widthAnchor) })
        setButtons(confirm: rule == nil ? L("Add rule") : L("Save"))
    }

    override func viewDidAppear() {
        super.viewDidAppear()
        if canEditText { view.window?.makeFirstResponder(text) }
    }

    override func confirmTapped() {
        let value = (canEditText ? text.stringValue : originalText).trimmingCharacters(in: .whitespacesAndNewlines)
        guard !value.isEmpty else {
            NSSound.beep()
            return
        }
        let selected = behavior.selectedItem?.representedObject as? String ?? "allow"
        onSave?(value, AutoReviewRule.Behavior(rawValue: selected) ?? .allow)
        dismissSheet()
    }

    func control(_ control: NSControl, textView: NSTextView, doCommandBy commandSelector: Selector) -> Bool {
        guard control === text, commandSelector == #selector(NSResponder.insertNewline(_:)) else { return false }
        textView.insertNewlineIgnoringFieldEditor(nil)
        return true
    }
}

/// A complete command in a bounded wrapping code block.
final class ExactShellCommandView: NSView {
    private let command: NSTextField
    private let maxLines: Int
    private var renderedWidth: CGFloat = 0
    private static let commandFont = NSFont.monospacedSystemFont(ofSize: 11, weight: .regular)

    init(command commandText: String, maxLines: Int = 2) {
        self.maxLines = maxLines
        command = NSTextField(wrappingLabelWithString: commandText)
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        command.translatesAutoresizingMaskIntoConstraints = false
        command.font = Self.commandFont
        command.textColor = .labelColor
        command.maximumNumberOfLines = maxLines
        command.lineBreakMode = maxLines == 0 ? .byCharWrapping : .byTruncatingTail
        command.cell?.wraps = true
        command.cell?.usesSingleLineMode = false
        command.cell?.isScrollable = false
        command.cell?.truncatesLastVisibleLine = maxLines != 0
        command.isSelectable = true
        command.toolTip = commandText
        command.preferredMaxLayoutWidth = 640
        command.setContentHuggingPriority(.init(1), for: .horizontal)
        command.setContentCompressionResistancePriority(.init(1), for: .horizontal)
        let box = BackgroundView()
        box.fillColor = Theme.codeBackground
        box.cornerRadius = 6
        addSubview(box)
        box.addSubview(command)
        NSLayoutConstraint.activate([
            box.topAnchor.constraint(equalTo: topAnchor),
            box.leadingAnchor.constraint(equalTo: leadingAnchor),
            box.trailingAnchor.constraint(equalTo: trailingAnchor),
            box.bottomAnchor.constraint(equalTo: bottomAnchor),
            command.topAnchor.constraint(equalTo: box.topAnchor, constant: 7),
            command.leadingAnchor.constraint(equalTo: box.leadingAnchor, constant: 8),
            command.trailingAnchor.constraint(equalTo: box.trailingAnchor, constant: -8),
            command.bottomAnchor.constraint(equalTo: box.bottomAnchor, constant: -7),
        ])
    }

    override func layout() {
        super.layout()
        let available = bounds.width - 16
        if available > 0, renderedWidth != available {
            renderedWidth = available
            command.preferredMaxLayoutWidth = available
            invalidateIntrinsicContentSize()
            needsLayout = true
        }
    }

    override var intrinsicContentSize: NSSize {
        let lineHeight = ceil(Self.commandFont.boundingRectForFont.height)
        return NSSize(
            width: NSView.noIntrinsicMetric,
            height: maxLines == 0 ? command.intrinsicContentSize.height + 14 : lineHeight * CGFloat(maxLines) + 14)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }
}

/// Key on the left, any control on the right, inside a section card.
final class AccessoryRow: NSView {
    init(key keyText: String, accessory: NSView) {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        let key = Build.label(keyText, font: .systemFont(ofSize: 12.5))
        accessory.translatesAutoresizingMaskIntoConstraints = false
        addSubview(key)
        addSubview(accessory)
        NSLayoutConstraint.activate([
            heightAnchor.constraint(greaterThanOrEqualToConstant: 36),
            key.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            key.centerYAnchor.constraint(equalTo: centerYAnchor),
            accessory.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
            accessory.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }
}

/// One rule in two levels: content across the full width, then metadata and actions below.
final class AutoReviewRuleRow: NSView {
    var onEdit: (() -> Void)?
    var onDelete: (() -> Void)?

    init(content: NSView, detail: String) {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        content.translatesAutoresizingMaskIntoConstraints = false
        content.setContentHuggingPriority(.init(1), for: .horizontal)
        content.setContentCompressionResistancePriority(.init(1), for: .horizontal)

        let detailLabel = Build.label(
            detail, font: .systemFont(ofSize: 10.5), color: .secondaryLabelColor)
        detailLabel.toolTip = detail
        detailLabel.setContentHuggingPriority(.defaultLow, for: .horizontal)
        let edit = HoverButton(
            symbol: "square.and.pencil", pointSize: 11, tooltip: L("Edit rule"), target: self,
            action: #selector(edit))
        let delete = HoverButton(
            symbol: "trash", pointSize: 11, tooltip: L("Delete rule"), target: self,
            action: #selector(delete))
        edit.translatesAutoresizingMaskIntoConstraints = false
        delete.translatesAutoresizingMaskIntoConstraints = false

        addSubview(content)
        addSubview(detailLabel)
        addSubview(edit)
        addSubview(delete)
        // Past 900 points the content stops short of the row's end. Filling the row sits below the
        // split view's holding priority, so a wide page never pulls the sidebar divider, or the
        // window, in to keep the rows within the cap.
        let fillWidth = content.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12)
        fillWidth.priority = .defaultLow - 1
        NSLayoutConstraint.activate([
            content.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            content.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor, constant: -12),
            content.widthAnchor.constraint(lessThanOrEqualToConstant: 900),
            fillWidth,
            content.topAnchor.constraint(equalTo: topAnchor, constant: 10),

            detailLabel.topAnchor.constraint(equalTo: content.bottomAnchor, constant: 7),
            detailLabel.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            detailLabel.trailingAnchor.constraint(lessThanOrEqualTo: edit.leadingAnchor, constant: -8),
            detailLabel.centerYAnchor.constraint(equalTo: edit.centerYAnchor),
            detailLabel.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -8),

            edit.trailingAnchor.constraint(equalTo: delete.leadingAnchor, constant: -4),
            edit.widthAnchor.constraint(equalToConstant: 28),
            edit.heightAnchor.constraint(equalToConstant: 24),
            delete.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -8),
            delete.widthAnchor.constraint(equalToConstant: 28),
            delete.heightAnchor.constraint(equalToConstant: 24),
            delete.centerYAnchor.constraint(equalTo: edit.centerYAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    @objc private func edit() { onEdit?() }
    @objc private func delete() { onDelete?() }
}
