import AppKit

/// A skill of a bot's or a group's: its name and when to use it, the instructions, an example,
/// the reference and script files it carries, and how it changed. A draft a bot proposed or a
/// capture wrote opens here too, and is used only once it is saved. A save carries the revision
/// it was opened at, so it is refused when the skill changed on another Device meanwhile.
final class PlaybookViewController: SheetViewController {
    private let store = AppStore.shared
    private let scope: PlaybookScope
    private var record: PlaybookRecord?

    private let nameField = NSTextField()
    private let nameNote = Build.label("", font: Theme.Font.caption, color: .systemRed)
    private let descriptionField = NSTextField()
    private let instructions = NSTextView()
    private let examples = NSTextView()
    private let references = PlaybookFilesPane(folder: "references")
    private let scripts = PlaybookFilesPane(folder: "scripts")
    private let history = PlaybookHistoryPane()
    private let tabs = NSTabView()
    private let deleteButton = NSButton()

    var onClose: (() -> Void)?

    init(scope: PlaybookScope, record: PlaybookRecord? = nil) {
        self.scope = scope
        self.record = record
        super.init(title: record?.content?.name ?? L("New Skill"), subtitle: Self.subtitle(scope: scope, isDraft: record?.isDraft == true), width: 600)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    /// Fetches the skill's body and history, then opens it over `presenter`.
    static func open(_ skill: PlaybookSummary, from presenter: NSViewController) {
        Task { [weak presenter] in
            do {
                let record = try await AppStore.shared.playbook(skill.id, in: skill.scope)
                guard let presenter, presenter.view.window != nil, presenter.presentedViewControllers?.isEmpty != false else { return }
                presenter.presentAsSheet(PlaybookViewController(scope: skill.scope, record: record))
            } catch {
                guard let window = presenter?.view.window else { return }
                let alert = NSAlert()
                alert.messageText = L("Couldn't open %@", skill.name)
                alert.informativeText = error.localizedDescription
                await alert.beginSheetModal(for: window)
            }
        }
    }

    /// Who uses the skill, and for a draft, that it waits for a save.
    private static func subtitle(scope: PlaybookScope, isDraft: Bool) -> String {
        let store = AppStore.shared
        if scope.isGroup {
            let group = store.chat(scope.id).map { store.title(for: $0) } ?? L("Group")
            return isDraft ? L("A draft. Once you save it, the bots in %@ can use it there.", group) : L("The bots in %@ can use it there.", group)
        }
        let bot = store.bot(scope.id)?.name ?? L("The bot")
        return isDraft ? L("A draft. Once you save it, %@ can use it in every chat.", bot) : L("%@ can use it in every chat.", bot)
    }

    override func loadView() {
        super.loadView()
        nameField.placeholderString = "weekly-report"
        nameField.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
        nameField.setAccessibilityLabel(L("Name"))
        descriptionField.placeholderString = L("When a bot should use it")
        descriptionField.setAccessibilityLabel(L("Description"))
        for field in [nameField, descriptionField] {
            field.translatesAutoresizingMaskIntoConstraints = false
            field.delegate = self
        }

        let labelWidth = ceil([L("Name"), L("Description")].map { formLabel($0).intrinsicContentSize.width }.max() ?? 0)
        let grid = NSGridView(views: [
            [formLabel(L("Name")), nameField],
            [NSGridCell.emptyContentView, nameNote],
            [formLabel(L("Description")), descriptionField],
        ])
        grid.translatesAutoresizingMaskIntoConstraints = false
        grid.columnSpacing = 10
        grid.rowSpacing = 8
        grid.column(at: 0).width = labelWidth
        grid.column(at: 1).xPlacement = .fill
        for row in 0..<grid.numberOfRows { grid.row(at: row).yPlacement = .center }
        grid.row(at: 1).topPadding = -4
        grid.row(at: 1).isHidden = true

        tabs.translatesAutoresizingMaskIntoConstraints = false
        let pages: [(String, NSView)] = [
            (L("Instructions"), Self.editor(instructions, placeholder: L("What to do, step by step"))),
            (L("Examples"), Self.editor(examples, placeholder: L("A good result to aim for (optional)"))),
            (L("References"), references),
            (L("Scripts"), scripts),
        ] + (record == nil ? [] : [(L("History"), history as NSView)])
        for (title, page) in pages {
            let item = NSTabViewItem(identifier: title)
            item.label = title
            item.view = page
            tabs.addTabViewItem(item)
        }
        instructions.setAccessibilityLabel(L("Instructions"))
        examples.setAccessibilityLabel(L("Examples"))
        instructions.delegate = self

        contentStack.addArrangedSubview(grid)
        contentStack.addArrangedSubview(tabs)
        contentStack.setCustomSpacing(16, after: grid)
        NSLayoutConstraint.activate([
            grid.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            tabs.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            tabs.heightAnchor.constraint(equalToConstant: 300),
        ])

        deleteButton.bezelStyle = .rounded
        deleteButton.attributedTitle = NSAttributedString(
            string: L("Delete…"), attributes: [.foregroundColor: NSColor.systemRed, .font: NSFont.systemFont(ofSize: NSFont.systemFontSize)])
        deleteButton.target = self
        deleteButton.action = #selector(confirmDelete)
        setButtons(confirm: L("Save"), leading: record == nil ? nil : deleteButton)

        fill(record?.content ?? PlaybookContent())
        updateControls()
    }

    override func viewDidAppear() {
        super.viewDidAppear()
        view.window?.makeFirstResponder(record == nil ? nameField : instructions)
    }

    override func dismiss(_ sender: Any?) {
        super.dismiss(sender)
        onClose?()
    }

    private func formLabel(_ text: String) -> NSTextField {
        Build.label(text, font: .systemFont(ofSize: 12), color: .secondaryLabelColor)
    }

    /// A scrolling text view in a bezel, for the instructions and the example.
    private static func editor(_ text: NSTextView, placeholder: String) -> NSScrollView {
        text.isRichText = false
        text.font = .systemFont(ofSize: 13)
        text.textColor = .labelColor
        text.allowsUndo = true
        text.isAutomaticQuoteSubstitutionEnabled = false
        text.isAutomaticDashSubstitutionEnabled = false
        text.textContainerInset = NSSize(width: 6, height: 8)
        text.isVerticallyResizable = true
        text.isHorizontallyResizable = false
        text.autoresizingMask = [.width]
        text.textContainer?.widthTracksTextView = true
        let attributes: [NSAttributedString.Key: Any] = [.foregroundColor: NSColor.placeholderTextColor, .font: NSFont.systemFont(ofSize: 13)]
        text.setValue(NSAttributedString(string: placeholder, attributes: attributes), forKey: "placeholderAttributedString")
        let scroll = NSScrollView()
        scroll.documentView = text
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.borderType = .bezelBorder
        scroll.translatesAutoresizingMaskIntoConstraints = false
        return scroll
    }

    private func fill(_ content: PlaybookContent) {
        nameField.stringValue = content.name
        descriptionField.stringValue = content.description
        instructions.string = content.instructions
        examples.string = content.examples
        references.files = content.references
        scripts.files = content.scripts
        if let record { history.show(record) }
    }

    /// What the sheet holds, with the name as the CLI takes it: lowercase, hyphens for spaces.
    private var content: PlaybookContent {
        let name = nameField.stringValue.trimmingCharacters(in: .whitespaces).lowercased()
            .replacingOccurrences(of: " ", with: "-").replacingOccurrences(of: "_", with: "-")
        return PlaybookContent(
            name: name, description: descriptionField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines),
            instructions: instructions.string, examples: examples.string, references: references.files, scripts: scripts.files)
    }

    private var nameProblem: String? {
        let name = content.name
        guard !name.isEmpty else { return nil }
        let allowed = name.unicodeScalars.allSatisfy { ("a"..."z").contains($0) || ("0"..."9").contains($0) || $0 == "-" }
        return allowed && !name.hasPrefix("-") && !name.hasSuffix("-") && !name.contains("--") && name.utf8.count <= 64
            ? nil : L("Use lowercase letters, numbers, and hyphens.")
    }

    private func showNameNote(_ text: String?) {
        nameNote.stringValue = text ?? ""
        guard let grid = nameNote.superview as? NSGridView, grid.row(at: 1).isHidden != (text == nil) else { return }
        grid.row(at: 1).isHidden = text == nil
        if view.window != nil { fitSheetToContent() }
    }

    private func updateControls() {
        let content = content
        showNameNote(nameProblem)
        confirmButton.isEnabled = !content.name.isEmpty && nameProblem == nil && !content.description.isEmpty
            && !content.instructions.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    override func confirmTapped() {
        save(over: record)
    }

    private func save(over base: PlaybookRecord?) {
        let content = content
        confirmButton.isEnabled = false
        deleteButton.isEnabled = false
        Task { [weak self] in
            guard let self else { return }
            do {
                _ = try await store.savePlaybook(content, in: scope, over: base)
                dismiss(nil)
            } catch {
                confirmButton.isEnabled = true
                deleteButton.isEnabled = true
                let message = error.localizedDescription
                if message.contains("changed since") {
                    resolveConflict()
                } else if message.contains("already exists") {
                    showNameNote(L("Another skill here has this name."))
                } else {
                    showAlert(
                        message.contains("was removed") ? L("This skill was deleted on another Device.") : L("Couldn't save the skill"),
                        message.contains("was removed") ? "" : message)
                }
            }
        }
    }

    /// The skill changed on another Device while the sheet was open: show that version, or save
    /// these edits over it.
    private func resolveConflict() {
        guard let window = view.window, let record else { return }
        let alert = NSAlert()
        alert.messageText = L("This skill changed on another Device")
        alert.informativeText = L("Reload shows the latest version and discards your edits. Overwrite saves yours over it.")
        alert.addButton(withTitle: L("Reload"))
        alert.addButton(withTitle: L("Overwrite with mine"))
        alert.addButton(withTitle: L("Cancel"))
        alert.beginSheetModal(for: window) { [weak self] response in
            guard let self, response != .alertThirdButtonReturn else { return }
            Task { [weak self] in
                guard let self else { return }
                do {
                    let fresh = try await store.playbook(record.id, in: scope)
                    if response == .alertFirstButtonReturn {
                        self.record = fresh
                        fill(fresh.content ?? PlaybookContent())
                        updateControls()
                    } else {
                        save(over: fresh)
                    }
                } catch {
                    showAlert(L("This skill was deleted on another Device."), "")
                }
            }
        }
    }

    @objc private func confirmDelete() {
        guard let window = view.window, let record else { return }
        let alert = NSAlert()
        alert.messageText = L("Delete “%@”?", record.content?.name ?? "")
        alert.informativeText = record.isDraft ? L("The draft is deleted.") : L("Your bots stop using this skill. This can't be undone.")
        alert.addButton(withTitle: L("Delete"))
        alert.addButton(withTitle: L("Cancel"))
        alert.alertStyle = .warning
        alert.beginSheetModal(for: window) { [weak self] response in
            guard let self, response == .alertFirstButtonReturn else { return }
            Task { [weak self] in
                guard let self else { return }
                do {
                    try await store.removePlaybook(record)
                    dismiss(nil)
                } catch {
                    showAlert(L("Couldn't delete the skill"), error.localizedDescription)
                }
            }
        }
    }

    private func showAlert(_ message: String, _ detail: String) {
        guard let window = view.window else { return }
        let alert = NSAlert()
        alert.messageText = message
        alert.informativeText = detail
        alert.beginSheetModal(for: window, completionHandler: nil)
    }
}

extension PlaybookViewController: NSTextFieldDelegate {
    func controlTextDidChange(_ obj: Notification) {
        updateControls()
    }
}

extension PlaybookViewController: NSTextViewDelegate {
    func textDidChange(_ notification: Notification) {
        updateControls()
    }
}

/// A list beside a text view, in one bezel: a skill's files, or its history.
class PlaybookListPane: NSView, NSTableViewDataSource, NSTableViewDelegate {
    let table = NSTableView()
    let text = NSTextView()
    let empty = Build.label("", font: .systemFont(ofSize: 12), color: .tertiaryLabelColor, alignment: .center)
    private let listScroll = NSScrollView()
    let textScroll = NSScrollView()

    init(listWidth: CGFloat, footer: NSView?) {
        super.init(frame: .zero)
        // A tab view sizes its pages by their frames.
        autoresizingMask = [.width, .height]
        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("item"))
        table.addTableColumn(column)
        table.headerView = nil
        table.style = .plain
        table.dataSource = self
        table.delegate = self
        table.focusRingType = .none
        listScroll.documentView = table
        listScroll.hasVerticalScroller = true
        listScroll.autohidesScrollers = true
        listScroll.borderType = .bezelBorder
        listScroll.translatesAutoresizingMaskIntoConstraints = false

        text.isRichText = false
        text.textColor = .labelColor
        text.allowsUndo = true
        text.textContainerInset = NSSize(width: 6, height: 8)
        text.isVerticallyResizable = true
        text.isHorizontallyResizable = false
        text.autoresizingMask = [.width]
        text.textContainer?.widthTracksTextView = true
        textScroll.documentView = text
        textScroll.hasVerticalScroller = true
        textScroll.autohidesScrollers = true
        textScroll.borderType = .bezelBorder
        textScroll.translatesAutoresizingMaskIntoConstraints = false

        addSubview(listScroll)
        addSubview(textScroll)
        addSubview(empty)
        var constraints = [
            listScroll.topAnchor.constraint(equalTo: topAnchor),
            listScroll.leadingAnchor.constraint(equalTo: leadingAnchor),
            listScroll.widthAnchor.constraint(equalToConstant: listWidth),
            textScroll.topAnchor.constraint(equalTo: listScroll.topAnchor),
            textScroll.leadingAnchor.constraint(equalTo: listScroll.trailingAnchor, constant: 8),
            textScroll.trailingAnchor.constraint(equalTo: trailingAnchor),
            textScroll.bottomAnchor.constraint(equalTo: bottomAnchor),
            empty.centerXAnchor.constraint(equalTo: textScroll.centerXAnchor),
            empty.centerYAnchor.constraint(equalTo: textScroll.centerYAnchor),
            empty.widthAnchor.constraint(lessThanOrEqualTo: textScroll.widthAnchor, constant: -24),
        ]
        if let footer {
            footer.translatesAutoresizingMaskIntoConstraints = false
            addSubview(footer)
            constraints += [
                footer.topAnchor.constraint(equalTo: listScroll.bottomAnchor, constant: -1),
                footer.leadingAnchor.constraint(equalTo: listScroll.leadingAnchor),
                footer.bottomAnchor.constraint(equalTo: textScroll.bottomAnchor),
            ]
        } else {
            constraints.append(listScroll.bottomAnchor.constraint(equalTo: textScroll.bottomAnchor))
        }
        NSLayoutConstraint.activate(constraints)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func numberOfRows(in tableView: NSTableView) -> Int { 0 }
    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? { nil }
    func tableViewSelectionDidChange(_ notification: Notification) {}
}

/// The references or the scripts a skill carries: a file list with + and −, a name edited in
/// place, and the selected file's text. Files are text the skill holds; nothing here reads or
/// writes a file on disk, and a script runs only when a bot runs it, with the usual Auto-review.
final class PlaybookFilesPane: PlaybookListPane, NSTextViewDelegate, NSTextFieldDelegate {
    private let folder: String
    private let buttons = NSSegmentedControl()
    private var items: [PlaybookFile] = []
    private var shown: Int?

    init(folder: String) {
        self.folder = folder
        buttons.segmentCount = 2
        buttons.trackingMode = .momentary
        buttons.segmentStyle = .smallSquare
        for (index, (symbol, label)) in [("plus", folder == "scripts" ? L("Add Script") : L("Add Reference")), ("minus", L("Remove"))].enumerated() {
            buttons.setImage(NSImage(systemSymbolName: symbol, accessibilityDescription: label), forSegment: index)
            buttons.setWidth(28, forSegment: index)
            buttons.setToolTip(label, forSegment: index)
        }
        super.init(listWidth: 168, footer: buttons)
        buttons.target = self
        buttons.action = #selector(buttonPressed)
        table.rowHeight = 24
        text.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
        text.isAutomaticQuoteSubstitutionEnabled = false
        text.isAutomaticDashSubstitutionEnabled = false
        text.isAutomaticTextReplacementEnabled = false
        text.isAutomaticSpellingCorrectionEnabled = false
        text.isContinuousSpellCheckingEnabled = false
        text.delegate = self
        empty.stringValue = folder == "scripts" ? L("No scripts") : L("No references")
    }

    var files: [PlaybookFile] {
        get {
            commit()
            return items
        }
        set {
            items = newValue
            shown = nil
            table.reloadData()
            select(items.isEmpty ? nil : 0)
        }
    }

    private func commit() {
        guard let shown, items.indices.contains(shown) else { return }
        items[shown].text = text.string
    }

    private func select(_ row: Int?) {
        commit()
        shown = row
        if let row {
            table.selectRowIndexes([row], byExtendingSelection: false)
            text.string = items[row].text
        } else {
            table.deselectAll(nil)
            text.string = ""
        }
        text.isEditable = row != nil
        textScroll.isHidden = row == nil
        empty.isHidden = row != nil
        buttons.setEnabled(row != nil, forSegment: 1)
        buttons.setEnabled(items.count < 16, forSegment: 0)
    }

    private func name(of file: PlaybookFile) -> String {
        String(file.path.dropFirst(folder.count + 1))
    }

    @objc private func buttonPressed() {
        if buttons.selectedSegment == 0 {
            commit()
            let stem = folder == "scripts" ? "script" : "notes"
            let ext = folder == "scripts" ? "sh" : "md"
            var name = "\(stem).\(ext)"
            var number = 2
            while items.contains(where: { $0.path == "\(folder)/\(name)" }) {
                name = "\(stem)-\(number).\(ext)"
                number += 1
            }
            items.append(PlaybookFile(path: "\(folder)/\(name)", text: ""))
            table.reloadData()
            select(items.count - 1)
            table.editColumn(0, row: items.count - 1, with: nil, select: true)
        } else if let shown {
            items.remove(at: shown)
            self.shown = nil
            table.reloadData()
            select(items.isEmpty ? nil : min(shown, items.count - 1))
        }
    }

    override func numberOfRows(in tableView: NSTableView) -> Int { items.count }

    override func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let field = NSTextField(string: name(of: items[row]))
        field.isBordered = false
        field.drawsBackground = false
        field.isEditable = true
        field.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
        field.lineBreakMode = .byTruncatingMiddle
        field.cell?.usesSingleLineMode = true
        field.delegate = self
        field.tag = row
        let cell = NSTableCellView()
        cell.addSubview(field)
        cell.textField = field
        field.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            field.leadingAnchor.constraint(equalTo: cell.leadingAnchor, constant: 4),
            field.trailingAnchor.constraint(equalTo: cell.trailingAnchor, constant: -4),
            field.centerYAnchor.constraint(equalTo: cell.centerYAnchor),
        ])
        return cell
    }

    override func tableViewSelectionDidChange(_ notification: Notification) {
        let row = table.selectedRow
        guard row != shown else { return }
        select(row >= 0 ? row : nil)
    }

    /// A rename keeps the folder; a name the CLI would refuse goes back to what it was.
    func controlTextDidEndEditing(_ obj: Notification) {
        guard let field = obj.object as? NSTextField, items.indices.contains(field.tag) else { return }
        let name = field.stringValue.trimmingCharacters(in: .whitespaces)
        let safe = !name.isEmpty && name.split(separator: "/", omittingEmptySubsequences: false).allSatisfy { part in
            !part.isEmpty && !part.hasPrefix(".") && part.unicodeScalars.allSatisfy { $0.isASCII && (CharacterSet.alphanumerics.contains($0) || "-_.".unicodeScalars.contains($0)) }
        }
        let path = "\(folder)/\(name)"
        if safe, !items.enumerated().contains(where: { $0.offset != field.tag && $0.element.path == path }) {
            items[field.tag].path = path
        } else {
            NSSound.beep()
            field.stringValue = self.name(of: items[field.tag])
        }
    }

    func textDidChange(_ notification: Notification) {
        commit()
    }
}

/// How a skill changed, newest first; picking a step shows its instructions then.
final class PlaybookHistoryPane: PlaybookListPane {
    private var steps: [PlaybookRevision] = []

    init() {
        super.init(listWidth: 196, footer: nil)
        table.rowHeight = 38
        text.isEditable = false
        text.font = .systemFont(ofSize: 13)
        empty.stringValue = L("Deleted")
    }

    func show(_ record: PlaybookRecord) {
        steps = record.revisions.sorted { ($0.revision, $0.createdAt) > ($1.revision, $1.createdAt) }
        table.reloadData()
        guard !steps.isEmpty else { return }
        table.selectRowIndexes([0], byExtendingSelection: false)
        showStep(0)
    }

    private func showStep(_ row: Int) {
        let content = steps[row].content
        text.string = content?.instructions ?? ""
        textScroll.isHidden = content == nil
        empty.isHidden = content != nil
    }

    /// What happened at a step, in a word or three.
    private func what(_ step: PlaybookRevision) -> String {
        switch (step.status, step.provenance.kind) {
        case ("deleted", _): return L("Deleted")
        case ("draft", "corrections"): return L("Drafted from corrections")
        case ("draft", _): return L("Drafted from a chat")
        case (_, "edit") where step.revision > 1:
            return steps.first { $0.revision == step.revision - 1 }?.status == "draft" ? L("Saved") : L("Edited")
        default: return L("Created")
        }
    }

    override func numberOfRows(in tableView: NSTableView) -> Int { steps.count }

    override func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let step = steps[row]
        let date = Format.daySeparator(Date(timeIntervalSince1970: step.createdAt))
        let device = AppStore.shared.device(step.deviceId)?.name
        let title = Build.label(what(step), font: .systemFont(ofSize: 12.5, weight: .medium))
        let detail = Build.label(device.map { "\(date) · \($0)" } ?? date, font: Theme.Font.caption, color: .secondaryLabelColor)
        let stack = Build.stack([title, detail], spacing: 1)
        let cell = NSTableCellView()
        cell.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: cell.leadingAnchor, constant: 6),
            stack.trailingAnchor.constraint(lessThanOrEqualTo: cell.trailingAnchor, constant: -6),
            stack.centerYAnchor.constraint(equalTo: cell.centerYAnchor),
        ])
        return cell
    }

    override func tableViewSelectionDidChange(_ notification: Notification) {
        guard table.selectedRow >= 0 else { return }
        showStep(table.selectedRow)
    }
}
