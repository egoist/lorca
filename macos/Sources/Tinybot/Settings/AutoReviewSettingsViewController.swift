import AppKit

/// Settings → Auto-review, after Grok Bot's: the switch and the rules, shared by every Device
/// through the roster. A rule is "When a bot wants to: …" with Allow automatically or Ask
/// first; a card's Always allow adds one here too.
final class AutoReviewSettingsViewController: SettingsPaneViewController {
    private let store = AppStore.shared
    private let check = SectionView(title: "Auto-review")
    private let rules = SectionView(title: "Auto-review Rules")
    private let toggle = NSSwitch()
    private let draft = NSTextField()
    private let draftBehavior = NSPopUpButton()
    private var rows: [(rule: AutoReviewRule, field: NSTextField, popup: NSPopUpButton)] = []

    override func viewDidLoad() {
        super.viewDidLoad()
        title = "Auto-review"
        toggle.controlSize = .small
        toggle.target = self
        toggle.action = #selector(toggled)
        draft.placeholderString = "When a bot wants to…"
        draft.controlSize = .small
        draft.font = .systemFont(ofSize: 12)
        draft.delegate = self
        for behavior in [AutoReviewRule.Behavior.allow, .ask] {
            draftBehavior.addItem(withTitle: behavior.title)
            draftBehavior.lastItem?.representedObject = behavior.rawValue
        }
        draftBehavior.controlSize = .small
        addSection(check)
        addSection(rules)
        addFootnote("Auto-review checks a plugin action that changes something before it runs, using the bot's own model, and asks you in the chat when the action needs a look. Off, every such action asks. Write one short, natural-language rule for each action; \"Ask first\" takes priority if rules conflict. Built-in safety checks always apply.")
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
        let description = NoteRow(text: "Tinybot checks each action before it runs and asks you first when needed. Add rules to customize what bots can do automatically.")
        let switchRow = AccessoryRow(key: "Check actions before they run", accessory: toggle)
        check.setRows([switchRow, description])

        rows = []
        var ruleRows: [NSView] = review.rules.map { rule in
            let field = NSTextField()
            field.stringValue = rule.text
            field.controlSize = .small
            field.font = .systemFont(ofSize: 12)
            field.isEditable = rule.tool == nil
            field.delegate = self
            field.toolTip = rule.tool.map { "Made from Always allow on a card, for \($0)." }
            let popup = NSPopUpButton()
            for behavior in [AutoReviewRule.Behavior.allow, .ask] {
                popup.addItem(withTitle: behavior.title)
                popup.lastItem?.representedObject = behavior.rawValue
            }
            popup.selectItem(at: rule.behavior == .allow ? 0 : 1)
            popup.controlSize = .small
            popup.target = self
            popup.action = #selector(behaviorChanged(_:))
            rows.append((rule, field, popup))
            let row = RuleRow(field: field, popup: popup)
            row.onDelete = { [weak self] in self?.delete(rule.id) }
            return row
        }
        if ruleRows.isEmpty {
            ruleRows.append(NoteRow(text: "No rules yet. Always allow on a card adds one, or write one below."))
        }
        let add = RuleRow(field: draft, popup: draftBehavior, addTitle: "Add rule")
        add.onAdd = { [weak self] in self?.addRule() }
        ruleRows.append(add)
        rules.setRows(ruleRows)
    }

    @objc private func toggled() {
        var review = store.autoReview
        review.isEnabled = toggle.state == .on
        store.setAutoReview(review)
    }

    @objc private func behaviorChanged(_ sender: NSPopUpButton) {
        save()
    }

    private func addRule() {
        let text = draft.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        var review = store.autoReview
        let behavior = AutoReviewRule.Behavior(rawValue: draftBehavior.selectedItem?.representedObject as? String ?? "allow") ?? .allow
        review.rules.append(AutoReviewRule(id: "", text: text, behavior: behavior))
        draft.stringValue = ""
        draftBehavior.selectItem(at: 0)
        store.setAutoReview(review)
    }

    private func delete(_ id: String) {
        var review = store.autoReview
        review.rules.removeAll { $0.id == id }
        store.setAutoReview(review)
    }

    /// Writes every row back: the texts as edited and the behaviors as picked.
    private func save() {
        var review = store.autoReview
        review.rules = rows.compactMap { row in
            let text = row.field.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !text.isEmpty else { return nil }
            var rule = row.rule
            rule.text = text
            rule.behavior = AutoReviewRule.Behavior(rawValue: row.popup.selectedItem?.representedObject as? String ?? "allow") ?? .allow
            return rule
        }
        if review != store.autoReview { store.setAutoReview(review) }
    }
}

extension AutoReviewSettingsViewController: NSTextFieldDelegate {
    func controlTextDidEndEditing(_ obj: Notification) {
        guard let field = obj.object as? NSTextField else { return }
        if field === draft {
            if (obj.userInfo?["NSTextMovement"] as? Int) == NSReturnTextMovement { addRule() }
        } else {
            save()
        }
    }
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

/// One rule: "When a bot wants to:" field, "It should:" popup, and a delete or Add button.
final class RuleRow: NSView {
    var onDelete: (() -> Void)?
    var onAdd: (() -> Void)?

    init(field: NSTextField, popup: NSPopUpButton, addTitle: String? = nil) {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        field.translatesAutoresizingMaskIntoConstraints = false
        popup.translatesAutoresizingMaskIntoConstraints = false
        let button = NSButton()
        button.bezelStyle = .rounded
        button.controlSize = .small
        button.font = .systemFont(ofSize: 11)
        if let addTitle {
            button.title = addTitle
            button.action = #selector(add)
        } else {
            button.image = NSImage(systemSymbolName: "xmark", accessibilityDescription: "Delete rule")
            button.bezelStyle = .accessoryBarAction
            button.isBordered = false
            button.action = #selector(delete)
        }
        button.target = self
        button.translatesAutoresizingMaskIntoConstraints = false
        addSubview(field)
        addSubview(popup)
        addSubview(button)
        NSLayoutConstraint.activate([
            heightAnchor.constraint(greaterThanOrEqualToConstant: 36),
            field.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            field.centerYAnchor.constraint(equalTo: centerYAnchor),
            popup.leadingAnchor.constraint(equalTo: field.trailingAnchor, constant: 8),
            popup.widthAnchor.constraint(equalToConstant: 150),
            popup.centerYAnchor.constraint(equalTo: centerYAnchor),
            button.leadingAnchor.constraint(equalTo: popup.trailingAnchor, constant: 8),
            button.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -10),
            button.centerYAnchor.constraint(equalTo: centerYAnchor),
            button.widthAnchor.constraint(greaterThanOrEqualToConstant: 24),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    @objc private func add() { onAdd?() }
    @objc private func delete() { onDelete?() }
}
