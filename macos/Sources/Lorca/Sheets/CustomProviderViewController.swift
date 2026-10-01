import AppKit

/// Adds or edits a custom provider: any server that speaks OpenAI's Chat Completions or
/// Responses API, or Anthropic's Messages API. The sheet loads the models the server lists as
/// the user fills it in, and the user picks the ones bots can use, adding any the server does
/// not list. The CLI checks the server again before saving, and the provider reaches every
/// paired Device encrypted with the account key.
final class CustomProviderViewController: SheetViewController {
    private enum Listing: Equatable {
        case needsURL
        case loading
        case listed
        case unlisted
        case failed(String)
    }

    private enum Entry {
        case add(String)
        case model(CustomModel)
    }

    private let store = AppStore.shared
    /// The provider being edited; nil adds one.
    private let kind: ProviderCredential.Kind?
    /// Runs with the provider's kind once it is saved.
    private let onSave: (ProviderCredential.Kind) -> Void

    private let nameField = NSTextField()
    private let apiPopup = NSPopUpButton()
    private let baseURLField = NSTextField()
    private let endpointNote = Build.label("", font: Theme.Font.caption, color: .tertiaryLabelColor, lines: 0)
    private let keyField = APIKeyField()

    private let searchField = NSSearchField()
    private let table = NSTableView()
    private let tableScroll = NSScrollView()
    private let overlaySpinner = NSProgressIndicator()
    private let overlayLabel = Build.label("", font: Theme.Font.caption, color: .secondaryLabelColor, lines: 0, alignment: .center)
    private lazy var overlay = Build.stack([overlaySpinner, overlayLabel], spacing: 8, alignment: .centerX)
    private let listingSpinner = NSProgressIndicator()
    private let listingNote = Build.label("", font: Theme.Font.caption, color: .secondaryLabelColor)
    private let countLabel = Build.label("", font: Theme.Font.caption, color: .secondaryLabelColor)
    private let defaultLabel = Build.label(L("Default"), font: Theme.Font.caption, color: .secondaryLabelColor)
    private let defaultPopup = NSPopUpButton()

    private let deleteButton = NSButton()
    private let status = Build.label("", font: Theme.Font.caption, color: .secondaryLabelColor, lines: 0)
    private let spinner = NSProgressIndicator()
    private lazy var statusRow = Build.stack([spinner, status], orientation: .horizontal, spacing: 8)

    /// Every model the sheet knows, in list order: saved, added by hand, or listed by the server.
    private var models: [CustomModel]
    /// The ones bots can pick.
    private var selected: Set<String>
    /// Ids the user typed in, which stay when another listing replaces the server's.
    private var added: Set<String> = []
    private var defaultID: String?
    private var entries: [Entry] = []
    private var listing = Listing.needsURL
    private var fetch: Task<Void, Never>?
    private var fetchGeneration = 0
    private var task: Task<Void, Never>?
    private var isBusy = false

    /// Opens an existing provider (its key fetched first, so the sheet opens populated), a
    /// preset, or an empty sheet. A kind the account no longer has opens the sheet to add one.
    static func present(
        kind: ProviderCredential.Kind?, preset: CustomProviderPreset? = nil, from presenter: NSViewController,
        onSave: @escaping (ProviderCredential.Kind) -> Void = { _ in }
    ) {
        Task { [weak presenter] in
            do {
                let store = AppStore.shared
                let existing = kind.flatMap { store.credential(for: $0) }
                var apiKey = ""
                if let existing, !store.isMock { apiKey = try await store.providerAPIKey(existing.kind).apiKey ?? "" }
                guard let presenter, presenter.view.window != nil,
                    presenter.presentedViewControllers?.isEmpty != false else { return }
                presenter.presentAsSheet(
                    CustomProviderViewController(existing: existing, preset: existing == nil ? preset : nil, apiKey: apiKey, onSave: onSave))
            } catch {
                guard let window = presenter?.view.window else { return }
                await NSAlert(error: error).beginSheetModal(for: window)
            }
        }
    }

    private init(existing: ProviderCredential?, preset: CustomProviderPreset?, apiKey: String, onSave: @escaping (ProviderCredential.Kind) -> Void) {
        kind = existing?.kind
        self.onSave = onSave
        models = existing?.models ?? []
        selected = Set(models.map(\.id))
        defaultID = models.first?.id
        super.init(
            title: existing?.name ?? preset.map { L("Add %@", $0.name) } ?? L("Add Custom Provider"),
            subtitle: L("Any server that speaks OpenAI’s or Anthropic’s API, such as a gateway or a model server on your network. Encrypted and shared with your paired Devices."),
            width: 520)
        nameField.stringValue = existing?.name ?? preset?.name ?? ""
        baseURLField.stringValue = existing?.baseURL ?? preset?.baseURL ?? ""
        let api = existing?.api ?? preset?.api ?? .chatCompletions
        apiPopup.addItems(withTitles: CustomAPI.allCases.map(\.title))
        apiPopup.selectItem(at: CustomAPI.allCases.firstIndex(of: api) ?? 0)
        keyField.stringValue = apiKey
        keyField.placeholderString = preset?.keyPlaceholder ?? L("Optional for a server on your network")
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    private var api: CustomAPI {
        CustomAPI.allCases[max(0, apiPopup.indexOfSelectedItem)]
    }

    private var baseURL: String {
        baseURLField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// The name to save: the one typed, else the known server's name or the host.
    private var name: String {
        let typed = nameField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        return typed.isEmpty ? suggestedName ?? "" : typed
    }

    private var suggestedName: String? {
        CustomProviderPreset.matching(baseURL)?.name ?? URL(string: baseURL)?.host
    }

    // MARK: - Layout

    override func loadView() {
        super.loadView()

        for indicator in [spinner, overlaySpinner, listingSpinner] {
            indicator.style = .spinning
            indicator.controlSize = .small
            indicator.isDisplayedWhenStopped = false
            indicator.translatesAutoresizingMaskIntoConstraints = false
        }
        statusRow.alignment = .centerY
        statusRow.isHidden = true

        nameField.delegate = self
        apiPopup.target = self
        apiPopup.action = #selector(apiChanged)
        baseURLField.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
        baseURLField.delegate = self
        keyField.onChange = { [weak self] in
            self?.updateControls()
            self?.loadModels(after: 0.5)
        }
        let labels = [L("Name"), L("API"), L("API base URL"), L("API key")].map(formLabel)
        let form = NSGridView(views: [
            [labels[0], nameField],
            [labels[1], apiPopup],
            [labels[2], baseURLField],
            [NSGridCell.emptyContentView, endpointNote],
            [labels[3], keyField],
        ])
        form.translatesAutoresizingMaskIntoConstraints = false
        form.columnSpacing = 10
        form.rowSpacing = 8
        form.row(at: 3).topPadding = -4
        for row in 0..<form.numberOfRows { form.row(at: row).yPlacement = .center }
        // The labels' column is as wide as the longest label; the controls take the rest.
        form.column(at: 0).width = ceil(labels.map(\.intrinsicContentSize.width).max() ?? 0)
        form.column(at: 1).xPlacement = .fill
        contentStack.addArrangedSubview(form)
        form.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true

        let models = modelsSection()
        contentStack.addArrangedSubview(models)
        models.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true

        contentStack.addArrangedSubview(statusRow)
        statusRow.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true

        deleteButton.bezelStyle = .rounded
        deleteButton.hasDestructiveAction = true
        deleteButton.attributedTitle = NSAttributedString(
            string: L("Delete"), attributes: [.foregroundColor: NSColor.systemRed, .font: NSFont.systemFont(ofSize: NSFont.systemFontSize)])
        deleteButton.target = self
        deleteButton.action = #selector(deleteTapped)
        setButtons(confirm: kind == nil ? L("Add") : L("Save"), leading: kind == nil ? nil : deleteButton)

        updateEndpoint()
        reloadEntries()
        updateControls()
        loadModels(after: 0)
    }

    override func viewDidAppear() {
        super.viewDidAppear()
        // A preset needs its key next; an empty sheet starts at the name.
        guard kind == nil else { return }
        if nameField.stringValue.isEmpty {
            view.window?.makeFirstResponder(nameField)
        } else {
            keyField.focus()
        }
    }

    private func formLabel(_ text: String) -> NSTextField {
        Build.label(text, font: .systemFont(ofSize: 12), color: .secondaryLabelColor)
    }

    /// The search-or-add field, the checklist with its messages, and the count and default.
    private func modelsSection() -> NSView {
        let title = Build.label(L("Models"), font: .systemFont(ofSize: 12, weight: .semibold))
        listingNote.lineBreakMode = .byTruncatingTail
        listingNote.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        let header = Build.stack([title, NSView(), listingNote, listingSpinner], orientation: .horizontal, spacing: 6)
        header.alignment = .centerY

        searchField.placeholderString = L("Search or add a model ID")
        searchField.sendsSearchStringImmediately = true
        searchField.delegate = self
        searchField.translatesAutoresizingMaskIntoConstraints = false

        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("model"))
        column.resizingMask = .autoresizingMask
        table.addTableColumn(column)
        table.headerView = nil
        table.style = .plain
        table.rowHeight = 34
        table.intercellSpacing = NSSize(width: 0, height: 0)
        table.selectionHighlightStyle = .none
        table.columnAutoresizingStyle = .uniformColumnAutoresizingStyle
        table.dataSource = self
        table.delegate = self
        table.target = self
        table.action = #selector(rowClicked)
        table.setAccessibilityLabel(L("Models"))
        tableScroll.documentView = table
        tableScroll.hasVerticalScroller = true
        tableScroll.autohidesScrollers = true
        tableScroll.borderType = .bezelBorder
        tableScroll.translatesAutoresizingMaskIntoConstraints = false

        overlay.translatesAutoresizingMaskIntoConstraints = false
        let listArea = NSView()
        listArea.translatesAutoresizingMaskIntoConstraints = false
        listArea.addSubview(tableScroll)
        listArea.addSubview(overlay)
        tableScroll.pin(to: listArea)
        NSLayoutConstraint.activate([
            listArea.heightAnchor.constraint(equalToConstant: 196),
            overlay.centerXAnchor.constraint(equalTo: listArea.centerXAnchor),
            overlay.centerYAnchor.constraint(equalTo: listArea.centerYAnchor),
            overlay.widthAnchor.constraint(lessThanOrEqualTo: listArea.widthAnchor, constant: -48),
        ])

        defaultPopup.controlSize = .small
        defaultPopup.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
        defaultPopup.target = self
        defaultPopup.action = #selector(defaultChanged)
        defaultPopup.setAccessibilityLabel(L("Default model"))
        defaultPopup.translatesAutoresizingMaskIntoConstraints = false
        defaultPopup.widthAnchor.constraint(lessThanOrEqualToConstant: 240).isActive = true
        let footer = Build.stack([countLabel, NSView(), defaultLabel, defaultPopup], orientation: .horizontal, spacing: 6)
        footer.alignment = .centerY
        footer.heightAnchor.constraint(equalToConstant: 22).isActive = true

        let section = Build.stack([header, searchField, listArea, footer], spacing: 8)
        section.setCustomSpacing(6, after: header)
        section.translatesAutoresizingMaskIntoConstraints = false
        for view in [header, searchField, listArea, footer] {
            view.widthAnchor.constraint(equalTo: section.widthAnchor).isActive = true
        }
        return section
    }

    // MARK: - Models

    /// Asks the server for its models once the fields stop changing; a newer request replaces
    /// an older one's answer.
    private func loadModels(after delay: Double) {
        fetch?.cancel()
        fetchGeneration += 1
        let generation = fetchGeneration
        let root = baseURL
        guard root.hasPrefix("http://") || root.hasPrefix("https://"), URL(string: root)?.host?.isEmpty == false else {
            show(.needsURL)
            return
        }
        show(.loading)
        let (name, api, key) = (name, api, keyField.stringValue)
        fetch = Task { [weak self] in
            try? await Task.sleep(nanoseconds: UInt64(delay * 1_000_000_000))
            guard !Task.isCancelled else { return }
            do {
                let listed = try await AppStore.shared.listCustomModels(name: name, api: api, baseURL: root, apiKey: key)
                guard let self, generation == self.fetchGeneration else { return }
                self.take(listed)
            } catch {
                guard let self, generation == self.fetchGeneration, !Task.isCancelled else { return }
                self.show(.failed(error.localizedDescription))
            }
        }
    }

    /// A listing replaces the last one, and a server with none leaves no other server's models
    /// behind; picked and typed models stay. A short list, as a model server on the user's
    /// network has, starts picked.
    private func take(_ listed: [CustomModel]?) {
        let (picked, typed) = (selected, added)
        models = ModelChecklist.merge(models, keeping: { picked.contains($0) || typed.contains($0) }, with: listed ?? [])
        guard let listed, !listed.isEmpty else {
            show(.unlisted)
            reloadEntries()
            return
        }
        if selected.isEmpty && listed.count <= 8 {
            selected = Set(listed.map(\.id))
            defaultID = listed.first?.id
        }
        show(.listed)
        reloadEntries()
        updateControls()
    }

    private func show(_ state: Listing) {
        listing = state
        if state == .loading { listingSpinner.startAnimation(nil) } else { listingSpinner.stopAnimation(nil) }
        updateOverlay()
    }

    private func reloadEntries() {
        let query = searchField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        var entries: [Entry] = ModelChecklist.addCandidate(query, in: models).map { [.add($0)] } ?? []
        entries += ModelChecklist.filter(models, query).map(Entry.model)
        self.entries = entries
        table.reloadData()
        updateOverlay()
        updateFooter()
    }

    /// A message in place of the list when it has no rows: what the sheet needs, that it is
    /// loading, or what the server answered.
    private func updateOverlay() {
        let message: String? =
            switch listing {
            case .needsURL: L("Enter the base URL to load the server’s models.")
            case .loading: L("Loading models…")
            case .listed: nil
            case .unlisted: L("This server doesn’t list its models. Add model IDs above.")
            case .failed(let error): "\(error) \(L("You can still add model IDs above."))"
            }
        let empty = entries.isEmpty
        overlay.isHidden = !empty || message == nil
        overlayLabel.stringValue = message ?? ""
        if empty && listing == .loading { overlaySpinner.startAnimation(nil) } else { overlaySpinner.stopAnimation(nil) }
        // With rows on screen, a failure reads in the header instead.
        if case .failed(let error) = listing, !empty {
            listingNote.stringValue = error
            listingNote.toolTip = error
        } else {
            listingNote.stringValue = ""
            listingNote.toolTip = nil
        }
    }

    /// How many models are picked, and which one bots run without a model of their own. The
    /// row keeps its height while nothing is picked.
    private func updateFooter() {
        let picked = models.filter { selected.contains($0.id) }
        countLabel.stringValue = picked.isEmpty ? "" : L("%d selected", picked.count)
        defaultPopup.removeAllItems()
        for model in picked { defaultPopup.addItem(withTitle: model.displayName) }
        if let index = picked.firstIndex(where: { $0.id == defaultID }) { defaultPopup.selectItem(at: index) }
        defaultPopup.isHidden = picked.isEmpty
        defaultLabel.isHidden = picked.isEmpty
        defaultPopup.isEnabled = !isBusy
    }

    private func toggle(_ id: String) {
        if selected.remove(id) == nil {
            selected.insert(id)
            if defaultID == nil { defaultID = id }
        } else if defaultID == id {
            defaultID = models.first { selected.contains($0.id) }?.id
        }
        if let row = entries.firstIndex(where: { if case .model(let model) = $0 { model.id == id } else { false } }) {
            table.reloadData(forRowIndexes: [row], columnIndexes: [0])
        }
        updateFooter()
        updateControls()
    }

    /// A model the server does not list, picked, at the top of the list.
    private func add(_ id: String) {
        if !models.contains(where: { $0.id == id }) { models.insert(CustomModel(id: id), at: 0) }
        added.insert(id)
        selected.insert(id)
        if defaultID == nil { defaultID = id }
        searchField.stringValue = ""
        reloadEntries()
        updateControls()
        table.scrollRowToVisible(0)
    }

    @objc private func rowClicked() {
        let row = table.clickedRow
        guard entries.indices.contains(row), !isBusy else { return }
        switch entries[row] {
        case .add(let id): add(id)
        case .model(let model): toggle(model.id)
        }
    }

    @objc private func defaultChanged() {
        let picked = models.filter { selected.contains($0.id) }
        let index = defaultPopup.indexOfSelectedItem
        if picked.indices.contains(index) { defaultID = picked[index].id }
    }

    // MARK: - Fields

    @objc private func apiChanged() {
        updateEndpoint()
        loadModels(after: 0)
    }

    /// Names the URL the CLI calls, so a base URL with a missing or doubled path shows here,
    /// and offers the server's host as the name.
    private func updateEndpoint() {
        baseURLField.placeholderString = api.baseURLPlaceholder
        endpointNote.stringValue = baseURL.isEmpty ? L("Lorca adds %@ to it.", api.path) : L("Requests go to %@.", api.endpoint(for: baseURL))
        nameField.placeholderString = suggestedName ?? "OpenRouter"
        if kind == nil, keyField.stringValue.isEmpty {
            keyField.placeholderString = CustomProviderPreset.matching(baseURL)?.keyPlaceholder ?? L("Optional for a server on your network")
        }
        if view.window != nil { fitSheetToContent() }
    }

    // MARK: - Saving

    override func confirmTapped() {
        guard confirmButton.isEnabled else { return }
        let name = name
        beginOperation(L("Checking %@…", name))
        let ids = ModelChecklist.orderedIDs(models, selected: selected, defaultID: defaultID)
        let (api, baseURL, apiKey) = (api, baseURL, keyField.stringValue)
        task = Task { [weak self] in
            guard let self else { return }
            do {
                let saved = try await self.store.saveCustomProvider(
                    kind: self.kind, name: name, api: api, baseURL: baseURL, apiKey: apiKey, models: ids)
                try Task.checkCancellation()
                self.spinner.stopAnimation(nil)
                self.status.textColor = .systemGreen
                self.status.stringValue = L("%@ connected.", name)
                try await Task.sleep(nanoseconds: 600_000_000)
                self.dismiss(nil)
                self.onSave(saved)
            } catch {
                guard !Task.isCancelled else { return }
                self.showError(error)
            }
        }
    }

    override func dismissSheet() {
        task?.cancel()
        fetch?.cancel()
        super.dismissSheet()
    }

    override func viewDidDisappear() {
        super.viewDidDisappear()
        task?.cancel()
        fetch?.cancel()
        keyField.clear()
    }

    @objc private func deleteTapped() {
        guard let kind else { return }
        beginOperation(L("Deleting…"))
        task = Task { [weak self] in
            guard let self else { return }
            do {
                try await self.store.disconnectProvider(kind)
                try Task.checkCancellation()
                self.dismiss(nil)
            } catch {
                guard !Task.isCancelled else { return }
                self.showError(error)
            }
        }
    }

    private func beginOperation(_ message: String) {
        isBusy = true
        updateControls()
        spinner.startAnimation(nil)
        status.textColor = .secondaryLabelColor
        status.stringValue = message
        statusRow.isHidden = false
        if view.window != nil { fitSheetToContent() }
    }

    private func showError(_ error: Error) {
        isBusy = false
        spinner.stopAnimation(nil)
        status.textColor = .systemRed
        status.stringValue = error.localizedDescription
        statusRow.isHidden = false
        updateControls()
        fitSheetToContent()
    }

    private func updateControls() {
        for control in [nameField, apiPopup, baseURLField, searchField, deleteButton] as [NSControl] { control.isEnabled = !isBusy }
        keyField.isEnabled = !isBusy
        table.isEnabled = !isBusy
        defaultPopup.isEnabled = !isBusy
        confirmButton.isEnabled = !isBusy && !name.isEmpty && !baseURL.isEmpty && !selected.isEmpty
    }
}

extension CustomProviderViewController: NSTextFieldDelegate, NSSearchFieldDelegate {
    func controlTextDidChange(_ obj: Notification) {
        if obj.object as? NSSearchField === searchField {
            reloadEntries()
        } else if obj.object as? NSTextField === baseURLField {
            updateEndpoint()
            loadModels(after: 0.5)
        }
        updateControls()
    }

    /// Return in the search field adds the id it holds, or picks the one model it names; with
    /// nothing typed it falls through to the sheet's default button.
    func control(_ control: NSControl, textView: NSTextView, doCommandBy commandSelector: Selector) -> Bool {
        guard control === searchField, commandSelector == #selector(NSResponder.insertNewline(_:)) else { return false }
        let query = searchField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !query.isEmpty else { return false }
        if let id = ModelChecklist.addCandidate(query, in: models) {
            add(id)
        } else {
            if !selected.contains(query) { toggle(query) }
            searchField.stringValue = ""
            reloadEntries()
        }
        return true
    }
}

extension CustomProviderViewController: NSTableViewDataSource, NSTableViewDelegate {
    func numberOfRows(in tableView: NSTableView) -> Int {
        entries.count
    }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let cell = tableView.makeView(withIdentifier: ModelCellView.identifier, owner: nil) as? ModelCellView ?? ModelCellView()
        switch entries[row] {
        case .add(let id):
            cell.configure(adding: id)
        case .model(let model):
            cell.configure(model: model, isPicked: selected.contains(model.id)) { [weak self] in self?.toggle(model.id) }
        }
        return cell
    }

    func tableView(_ tableView: NSTableView, shouldSelectRow row: Int) -> Bool {
        false
    }
}

/// One row of the model checklist: a checkbox with the model's name, its id under the name when
/// they differ, and its context window and whether it sees images. The Add row has a plus in
/// the checkbox's place.
private final class ModelCellView: NSTableCellView {
    static let identifier = NSUserInterfaceItemIdentifier("customModel")

    private let checkbox = NSButton(checkboxWithTitle: "", target: nil, action: nil)
    private let plus = NSImageView()
    private let title = Build.label("", font: .systemFont(ofSize: 13))
    private let subtitle = Build.label("", font: .systemFont(ofSize: 11), color: .secondaryLabelColor)
    private let contextLabel = Build.label("", font: .monospacedDigitSystemFont(ofSize: 11, weight: .regular), color: .secondaryLabelColor)
    private let eye = NSImageView()
    private var onToggle: (() -> Void)?

    init() {
        super.init(frame: .zero)
        identifier = Self.identifier
        checkbox.target = self
        checkbox.action = #selector(toggled)
        plus.image = NSImage(systemSymbolName: "plus.circle.fill", accessibilityDescription: nil)
        plus.contentTintColor = .controlAccentColor
        eye.image = NSImage(systemSymbolName: "eye", accessibilityDescription: L("Sees images"))
        eye.symbolConfiguration = .init(pointSize: 11, weight: .regular)
        eye.contentTintColor = .secondaryLabelColor
        eye.toolTip = L("Sees images")
        title.lineBreakMode = .byTruncatingTail
        subtitle.lineBreakMode = .byTruncatingMiddle
        for label in [title, subtitle] { label.setContentCompressionResistancePriority(.defaultLow, for: .horizontal) }
        contextLabel.setContentCompressionResistancePriority(.required, for: .horizontal)
        let text = Build.stack([title, subtitle], spacing: 0)
        let trailing = Build.stack([contextLabel, eye], orientation: .horizontal, spacing: 6)
        trailing.alignment = .centerY
        for view in [checkbox, plus, text, trailing] as [NSView] {
            view.translatesAutoresizingMaskIntoConstraints = false
            addSubview(view)
        }
        NSLayoutConstraint.activate([
            checkbox.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 10),
            checkbox.centerYAnchor.constraint(equalTo: centerYAnchor),
            plus.centerXAnchor.constraint(equalTo: checkbox.centerXAnchor),
            plus.centerYAnchor.constraint(equalTo: centerYAnchor),
            text.leadingAnchor.constraint(equalTo: checkbox.trailingAnchor, constant: 6),
            text.centerYAnchor.constraint(equalTo: centerYAnchor),
            trailing.leadingAnchor.constraint(greaterThanOrEqualTo: text.trailingAnchor, constant: 8),
            trailing.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -10),
            trailing.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func configure(model: CustomModel, isPicked: Bool, onToggle: @escaping () -> Void) {
        self.onToggle = onToggle
        checkbox.isHidden = false
        plus.isHidden = true
        checkbox.state = isPicked ? .on : .off
        checkbox.setAccessibilityLabel(model.displayName)
        title.stringValue = model.displayName
        title.textColor = .labelColor
        subtitle.stringValue = model.name == nil ? "" : model.id
        subtitle.isHidden = model.name == nil
        contextLabel.stringValue = model.contextWindow.map { Format.tokens($0) } ?? ""
        contextLabel.toolTip = model.contextWindow.map { L("Context window: %@ tokens", Format.tokens($0)) }
        eye.isHidden = model.images != true
    }

    func configure(adding id: String) {
        onToggle = nil
        checkbox.isHidden = true
        plus.isHidden = false
        title.stringValue = L("Add “%@”", id)
        title.textColor = .controlAccentColor
        subtitle.stringValue = ""
        subtitle.isHidden = true
        contextLabel.stringValue = ""
        contextLabel.toolTip = nil
        eye.isHidden = true
    }

    @objc private func toggled() {
        onToggle?()
    }
}
