import AppKit

/// Adds a bot from a template, a link someone shared or a file: its name, the Runner it runs on,
/// the provider, and for each plugin it uses one of that Runner's own connections. The new bot
/// shares nothing with the one it came from, and its routines start paused.
final class TemplateImportViewController: SheetViewController {
    enum Source: Equatable {
        case link(String)
        case file(URL)
    }

    private let store = AppStore.shared
    private var source: Source?
    private let onCreate: (Chat.ID) -> Void
    private let reply: TemplateReply
    /// Where the template comes from: a pasted link, or the file Choose File… picked.
    private let fromField = NSTextField()
    private lazy var chooseButton: NSButton = {
        let button = NSButton(title: L("Choose File…"), target: self, action: #selector(chooseFile))
        button.bezelStyle = .rounded
        return button
    }()
    /// What the template sets up, shown once one opens.
    private var formViews: [NSView] = []
    /// Whether the user typed a name, which a template opened later leaves alone.
    private var isNameEdited = false
    private let nameField = NSTextField()
    private let runnerPopup = NSPopUpButton()
    private let providerPopup = NSPopUpButton()
    private let pluginRows = Build.stack([], spacing: 12)
    private let list = TemplateItemList(maxHeight: 260)
    private let note = Build.label("", font: .systemFont(ofSize: 11.5), color: .tertiaryLabelColor, lines: 0)
    private lazy var runners = store.runners
    private lazy var providerKinds = store.providerKinds
    /// The connection each plugin uses, by plugin; the CLI's pick until the user makes one.
    private var mappings: [String: String] = [:]
    private var preview: TemplateImportPreview?
    private var generation = 0
    private var isImporting = false
    /// The file's contents are listed once; another Runner or connection doesn't change them.
    private var listsContents = false
    /// The picked Runner's plugins as last previewed, so a change to them previews again.
    private var previewedPlugins: [InstalledPlugin] = []

    init(source: Source? = nil, reply: TemplateReply? = nil, onCreate: @escaping (Chat.ID) -> Void) {
        self.source = source
        self.onCreate = onCreate
        self.reply = reply ?? { method, params in try await AppStore.shared.templateReply(method, params) }
        super.init(title: L("New Bot from Template"), subtitle: "", width: 480)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()
        fromField.placeholderString = L("Paste a link to a shared bot")
        fromField.delegate = self
        switch source {
        case let .link(link): fromField.stringValue = link
        case let .file(url): fromField.stringValue = (url.path as NSString).abbreviatingWithTildeInPath
        case nil: break
        }
        let from = Build.stack([fromField, chooseButton], orientation: .horizontal, spacing: 8)
        fromField.setContentHuggingPriority(.defaultLow, for: .horizontal)
        chooseButton.setContentCompressionResistancePriority(.required, for: .horizontal)
        nameField.placeholderString = L("Name")
        nameField.delegate = self
        for runner in runners {
            runnerPopup.addItem(withTitle: runner.isThisDevice ? L("%@ (this computer)", runner.name) : runner.name)
        }
        runnerPopup.selectItem(at: runners.firstIndex(where: \.isThisDevice) ?? 0)
        runnerPopup.isEnabled = !runners.isEmpty
        runnerPopup.target = self
        runnerPopup.action = #selector(runnerChanged)
        for kind in providerKinds { providerPopup.addItem(withTitle: "\(kind.name) (\(kind.subtitle))") }
        providerPopup.selectItem(at: providerKinds.firstIndex(of: store.preferredProvider) ?? 0)

        let fromRow = formRow(L("From"), from)
        from.trailingAnchor.constraint(equalTo: fromRow.trailingAnchor).isActive = true
        formViews = [formRow(L("Name"), nameField), formRow(L("Runner"), runnerPopup), formRow(L("Provider"), providerPopup), pluginRows, list]
        for view in [fromRow] + formViews + [note] {
            contentStack.addArrangedSubview(view)
            view.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
        }
        contentStack.setCustomSpacing(16, after: pluginRows)
        formViews.forEach { $0.isHidden = true }
        setButtons(confirm: L("Create Bot"))
        confirmButton.isEnabled = false
        if source != nil {
            showNote(L("Loading…"))
            refresh()
        } else {
            showNote("")
        }
    }

    /// A file instead of a link.
    @objc private func chooseFile() {
        guard let window = view.window, !isImporting else { return }
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.lorcaTemplate]
        panel.beginSheetModal(for: window) { [weak self] response in
            guard let self, response == .OK, let url = panel.url else { return }
            self.fromField.stringValue = (url.path as NSString).abbreviatingWithTildeInPath
            self.open(.file(url))
        }
    }

    /// Another template: what it sets up shows again from the start.
    private func open(_ source: Source) {
        guard source != self.source else { return }
        self.source = source
        mappings.removeAll()
        listsContents = false
        preview = nil
        formViews.forEach { $0.isHidden = true }
        showNote(L("Loading…"))
        refresh()
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        // A plugin added or signed in on the Runner meanwhile changes what the import needs.
        store.observe(self) { [weak self] event in
            guard let self, case .rosterChanged = event, !self.isImporting,
                (self.runner.flatMap { self.store.device($0.id)?.plugins } ?? []) != self.previewedPlugins
            else { return }
            self.refresh()
        }
    }

    private var runner: Device? {
        runners.indices.contains(runnerPopup.indexOfSelectedItem) ? runners[runnerPopup.indexOfSelectedItem] : nil
    }

    private var trimmedName: String { nameField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines) }

    private var params: [String: Any] {
        var params: [String: Any] = ["mappings": mappings]
        switch source {
        case let .link(link): params["link"] = link
        case let .file(url): params["path"] = url.path
        case nil: break
        }
        if let runner { params["runner_id"] = runner.id }
        return params
    }

    @objc private func runnerChanged() {
        mappings.removeAll()
        refresh()
    }

    @objc private func connectionChanged(_ popup: NSPopUpButton) {
        guard let plugin = popup.identifier?.rawValue, let id = popup.selectedItem?.representedObject as? String else { return }
        mappings[plugin] = id
        refresh()
    }

    private func refresh() {
        guard source != nil else { return }
        generation += 1
        let current = generation
        let request = params
        previewedPlugins = runner.flatMap { store.device($0.id)?.plugins } ?? []
        confirmButton.isEnabled = false
        Task { [weak self] in
            guard let self else { return }
            do {
                let json = try await self.reply("templates.import.preview", request)
                guard current == self.generation else { return }
                self.show(TemplateImportPreview(json: json))
            } catch {
                guard current == self.generation else { return }
                self.preview = nil
                self.formViews.forEach { $0.isHidden = true }
                self.showNote(error.localizedDescription, color: .systemRed)
            }
        }
    }

    private func show(_ preview: TemplateImportPreview) {
        self.preview = preview
        // A file or link that holds no template only says why.
        guard preview.hasTemplate else {
            formViews.forEach { $0.isHidden = true }
            showNote(preview.issues.joined(separator: "\n"), color: .systemRed)
            updateButton()
            return
        }
        formViews.forEach { $0.isHidden = false }
        if !listsContents, !isNameEdited { nameField.stringValue = preview.profile?.title ?? "" }
        for plugin in preview.plugins { mappings[plugin.id] = plugin.selected }
        showPlugins(preview.plugins)
        if !listsContents {
            listsContents = true
            list.setSections([
                (L("Profile"), preview.profile.map { [TemplateItemRow.profile($0, isSelectable: false)] } ?? []),
                (L("Skills"), preview.skills.map { TemplateItemRow(item: $0, isSelectable: false) }),
                (L("Routines"), preview.routines.map { TemplateItemRow(item: $0, isSelectable: false) }),
                (L("Memories"), preview.memories.map { TemplateItemRow(item: $0, isSelectable: false, titleLines: 3) }),
            ])
        }
        let runnerName = runner?.name ?? ""
        if runners.isEmpty {
            showNote(L("Pair a Runner first: a bot runs on a computer."), color: .systemOrange)
        } else if !preview.issues.isEmpty {
            showNote(preview.issues.joined(separator: "\n"), color: .systemOrange)
        } else if let plugin = preview.plugins.first(where: { !$0.isReady }) {
            let text = plugin.connections.isEmpty ? L("Add %@ to %@ from the Marketplace first.", plugin.name, runnerName)
                : plugin.selected == nil ? L("Choose the %@ connection this bot uses.", plugin.name)
                : L("Finish setting up %@ on %@ first.", plugin.name, runnerName)
            showNote(text, color: .systemOrange)
        } else {
            showNote(preview.routines.isEmpty ? "" : L("Routines start paused."))
        }
        updateButton()
    }

    /// A line per plugin: a pop-up of the Runner's connections for it, or that it has none.
    private func showPlugins(_ plugins: [TemplateImportPreview.Plugin]) {
        pluginRows.arrangedSubviews.forEach { $0.removeFromSuperview() }
        for plugin in plugins {
            let control: NSView
            if plugin.connections.isEmpty {
                control = Build.label(L("Not on %@", runner?.name ?? ""), font: .systemFont(ofSize: 13), color: .systemOrange)
            } else {
                let popup = NSPopUpButton()
                popup.identifier = NSUserInterfaceItemIdentifier(plugin.id)
                if plugin.selected == nil {
                    popup.addItem(withTitle: L("Choose…"))
                    popup.lastItem?.isEnabled = false
                }
                for connection in plugin.connections {
                    popup.addItem(withTitle: connection.isReady ? connection.name : L("%@ (needs setup)", connection.name))
                    popup.lastItem?.representedObject = connection.id
                    if connection.id == plugin.selected { popup.select(popup.lastItem) }
                }
                popup.autoenablesItems = false
                popup.target = self
                popup.action = #selector(connectionChanged(_:))
                popup.setAccessibilityLabel(plugin.name)
                control = popup
            }
            let row = formRow(plugin.name, control)
            pluginRows.addArrangedSubview(row)
            row.widthAnchor.constraint(equalTo: pluginRows.widthAnchor).isActive = true
        }
        pluginRows.isHidden = plugins.isEmpty
    }

    private func showNote(_ text: String, color: NSColor = .tertiaryLabelColor) {
        note.stringValue = text
        note.textColor = color
        note.isHidden = text.isEmpty
        fitSheetToContent()
    }

    private func updateButton() {
        confirmButton.isEnabled = preview?.canImport == true && !trimmedName.isEmpty && runner != nil && !isImporting
    }

    override func confirmTapped() {
        guard let preview, preview.canImport, let runner, !trimmedName.isEmpty, !isImporting else { return }
        setImporting(true)
        var request = params
        request["runner_id"] = runner.id
        request["name"] = trimmedName
        request["expected_digest"] = preview.digest
        request["reviewed"] = true
        if providerKinds.indices.contains(providerPopup.indexOfSelectedItem) {
            request["provider"] = providerKinds[providerPopup.indexOfSelectedItem].wireValue
        }
        Task { [weak self] in
            guard let self else { return }
            do {
                let chatID = try await self.store.importTemplate(request)
                self.dismiss(nil)
                self.onCreate(chatID)
            } catch {
                self.setImporting(false)
                self.showNote(error.localizedDescription, color: .systemRed)
            }
        }
    }

    private func setImporting(_ importing: Bool) {
        isImporting = importing
        for control in [nameField, runnerPopup, providerPopup] as [NSControl] { control.isEnabled = !importing }
        for case let popup as NSPopUpButton in pluginRows.arrangedSubviews.flatMap(\.subviews) { popup.isEnabled = !importing }
        runnerPopup.isEnabled = !importing && !runners.isEmpty
        updateButton()
    }

    override func cancelOperation(_ sender: Any?) { if !isImporting { super.cancelOperation(sender) } }
    override func dismissSheet() { if !isImporting { super.dismissSheet() } }
}

extension TemplateImportViewController: NSTextFieldDelegate {
    func controlTextDidChange(_ obj: Notification) {
        guard (obj.object as? NSTextField) === fromField else {
            isNameEdited = true
            updateButton()
            return
        }
        // A link reads once it is whole: its page and the key after #.
        let text = fromField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        if text.contains("/t/"), text.contains("#") {
            open(.link(text))
        }
    }
}
