import AppKit

/// Adds or edits one of a Runner's MCP servers: its name, the command that starts it or its URL,
/// the environment or headers, and what it is for, in a form or as the JSON of its mcp.json
/// entry. JSON pasted into a field of the form, as READMEs and other apps' configs write it, opens
/// the JSON view. Once saved, the sheet follows the server: how it stands, Reconnect or Sign in, a
/// switch that turns it off, and the tools it offered.
final class McpServerViewController: SheetViewController {
    private enum Mode {
        case form
        case json
    }

    /// What pasted JSON came to.
    private enum Reading {
        case empty
        case reading
        case read([ParsedServer])
        case failed(String)
    }

    private let store = AppStore.shared
    private let runner: Device
    /// The server as the Runner last answered for it; nil while it is being added.
    private var saved: McpServer?

    private let statusSection = SectionView(title: L("Status"))
    private let toolsSection = SectionView(title: L("Tools"))
    private lazy var modeControl = NSSegmentedControl(
        labels: [L("Form"), L("JSON")], trackingMode: .selectOne, target: self, action: #selector(modeChanged))
    private lazy var typeControl = NSSegmentedControl(
        labels: [L("Command"), L("URL")], trackingMode: .selectOne, target: self, action: #selector(typeChanged))
    private let nameField = NSTextField()
    private let nameNote = Build.label("", font: Theme.Font.caption, color: .tertiaryLabelColor)
    private let commandField = NSTextField()
    private let commandNote = Build.label("", font: Theme.Font.caption, color: .tertiaryLabelColor, lines: 0)
    private let envEditor = PairsEditor(namePlaceholder: "API_KEY", valuePlaceholder: L("value"), addTitle: L("Add Variable"), label: L("Environment"))
    private let urlField = NSTextField()
    private let urlNote = Build.label(
        L("Streamable HTTP. When the server asks for a sign-in, Lorca signs in with OAuth; or send a token in a header."),
        font: Theme.Font.caption, color: .tertiaryLabelColor, lines: 0)
    private let headersEditor = PairsEditor(namePlaceholder: "Authorization", valuePlaceholder: "Bearer …", addTitle: L("Add Header"), label: L("Headers"))
    private let aboutField = NSTextField()
    private let jsonScroll = NSTextView.scrollableTextView()
    private var jsonView: NSTextView { jsonScroll.documentView as! NSTextView }
    private let jsonNote = Build.label("", font: Theme.Font.caption, color: .tertiaryLabelColor, lines: 0)
    private let pasteNote = Build.label(
        L("Paste the JSON a server's README gives, or a whole mcpServers block from Claude Desktop or Cursor, into any field."),
        font: Theme.Font.caption, color: .tertiaryLabelColor, lines: 0)
    private var nameGrid: NSGridView!
    private var formGrid: NSGridView!
    private var jsonGrid: NSGridView!
    private let removeButton = NSButton()
    private let spinner = NSProgressIndicator()
    private let status = Build.label("", font: Theme.Font.caption, color: .secondaryLabelColor, lines: 0)
    private lazy var statusRow = Build.stack([spinner, status], orientation: .horizontal, spacing: 8)

    private var mode = Mode.form
    private var reading = Reading.empty
    private var readGeneration = 0
    private var readTask: Task<Void, Never>?
    private var task: Task<Void, Never>?
    private var isBusy = false
    private var isConnecting = false
    private var isClosed = false
    /// Only the newest load renders, so an older answer arriving late (a sealed request to
    /// another Runner) never covers a newer one.
    private var loads = 0

    /// Opens a server of `runner`'s mcp.json (fetched first, with its tools, so the sheet opens
    /// filled in), or an empty sheet to add one.
    static func present(runner: Device, name: String? = nil, from presenter: NSViewController) {
        Task { [weak presenter] in
            var server: McpServer?
            if let name {
                do {
                    server = try await AppStore.shared.mcpServer(name, on: runner.id)
                } catch {
                    guard let window = presenter?.view.window else { return }
                    let alert = NSAlert()
                    alert.messageText = L("Couldn't open %@", name)
                    alert.informativeText = error.localizedDescription
                    await alert.beginSheetModal(for: window)
                    return
                }
            }
            guard let presenter, presenter.view.window != nil, presenter.presentedViewControllers?.isEmpty != false else { return }
            presenter.presentAsSheet(McpServerViewController(runner: runner, server: server))
        }
    }

    private init(runner: Device, server: McpServer?) {
        self.runner = runner
        saved = server
        super.init(
            title: server?.name ?? L("Add MCP Server"),
            subtitle: L(
                "A command %@ runs, or a remote server's URL, speaking the Model Context Protocol. Every bot on %@ can use its tools.", runner.name,
                runner.name),
            width: 560)
        nameField.stringValue = server?.name ?? ""
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    private var name: String { nameField.stringValue.trimmingCharacters(in: .whitespaces) }
    private var base: McpEntry { saved?.entry ?? McpEntry() }

    private var form: McpForm {
        var form = McpForm(entry: McpEntry())
        form.isRemote = typeControl.selectedSegment == 1
        form.command = commandField.stringValue
        form.env = envEditor.pairs
        form.url = urlField.stringValue
        form.headers = headersEditor.pairs
        form.about = aboutField.stringValue
        return form
    }

    private func fill(_ form: McpForm) {
        typeControl.selectedSegment = form.isRemote ? 1 : 0
        commandField.stringValue = form.command
        envEditor.pairs = form.env
        urlField.stringValue = form.url
        headersEditor.pairs = form.headers
        aboutField.stringValue = form.about
    }

    /// The entry the form, or the one server of the JSON, describes.
    private var entry: McpEntry? {
        switch mode {
        case .form:
            return form.entry(over: base)
        case .json:
            if case .read(let servers) = reading, servers.count == 1 { return servers[0].entry }
            return nil
        }
    }

    /// The JSON's servers when it holds more than one, which an Add adds together.
    private var several: [ParsedServer]? {
        guard mode == .json, saved == nil, case .read(let servers) = reading, servers.count > 1 else { return nil }
        return servers
    }

    /// What stops a save, in words, or nil.
    private var problem: String? {
        switch mode {
        case .form:
            return form.problem(name: name)
        case .json:
            switch reading {
            case .empty:
                return L("Paste a server's JSON.")
            case .reading:
                return L("Reading…")
            case .failed(let message):
                return message
            case .read(let servers):
                // Several are added together, the ones that can run; the note names the others.
                if let several { return several.allSatisfy({ $0.problem != nil }) ? several.first?.problem : nil }
                if servers.count > 1 { return L("That JSON has %d servers. Add them from an empty sheet, or keep one.", servers.count) }
                guard let server = servers.first else { return L("Paste a server's JSON.") }
                if let problem = server.problem { return problem }
                return name.isEmpty ? L("Give the server a name.") : nil
            }
        }
    }

    /// Whether what the sheet shows differs from what the Runner has.
    private var isDirty: Bool {
        guard let saved else { return true }
        if name != saved.name { return true }
        guard let entry else { return false }
        return !entry.saysTheSame(as: saved.entry)
    }

    /// What the server's tools go by: its name as an identifier, as the CLI makes it.
    private var toolPrefix: String {
        var out = ""
        for scalar in name.lowercased().unicodeScalars {
            if (scalar >= "a" && scalar <= "z") || (scalar >= "0" && scalar <= "9") {
                out.unicodeScalars.append(scalar)
            } else if !out.hasSuffix("_") {
                out.append("_")
            }
        }
        let trimmed = out.trimmingCharacters(in: CharacterSet(charactersIn: "_"))
        return trimmed.isEmpty ? "name" : trimmed
    }

    // MARK: - Layout

    override func loadView() {
        super.loadView()
        spinner.style = .spinning
        spinner.controlSize = .small
        spinner.isDisplayedWhenStopped = false
        spinner.translatesAutoresizingMaskIntoConstraints = false
        statusRow.alignment = .centerY
        statusRow.isHidden = true

        modeControl.selectedSegment = 0
        modeControl.setAccessibilityLabel(L("Edit as"))
        typeControl.setAccessibilityLabel(L("Type"))
        nameField.placeholderString = "github"
        nameField.delegate = self
        nameField.setAccessibilityLabel(L("Name"))
        for field in [commandField, urlField] {
            field.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
            field.delegate = self
        }
        commandField.placeholderString = "npx -y @modelcontextprotocol/server-filesystem ~/Documents"
        commandField.setAccessibilityLabel(L("Command"))
        urlField.placeholderString = "https://mcp.example.com/mcp"
        urlField.setAccessibilityLabel(L("URL"))
        aboutField.placeholderString = L("What it is for, which bots read (optional)")
        aboutField.delegate = self
        aboutField.setAccessibilityLabel(L("About"))
        commandNote.stringValue =
            runner.isThisDevice
            ? L("Starts on this computer from your login shell, so npx, uvx, and docker resolve as they do in a terminal.")
            : L("Starts on %@ from its login shell, so npx, uvx, and docker resolve as they do in a terminal there.", runner.name)
        for editor in [envEditor, headersEditor] {
            editor.onChange = { [weak self] in self?.updateControls() }
            editor.onResize = { [weak self] in self?.refit() }
        }

        jsonView.font = .monospacedSystemFont(ofSize: 11.5, weight: .regular)
        jsonView.isRichText = false
        jsonView.allowsUndo = true
        // JSON wants its straight quotes and dashes as typed.
        jsonView.isAutomaticQuoteSubstitutionEnabled = false
        jsonView.isAutomaticDashSubstitutionEnabled = false
        jsonView.isAutomaticTextReplacementEnabled = false
        jsonView.isAutomaticSpellingCorrectionEnabled = false
        jsonView.isContinuousSpellCheckingEnabled = false
        jsonView.textContainerInset = NSSize(width: 4, height: 6)
        jsonView.delegate = self
        jsonView.setAccessibilityLabel(L("The server's JSON"))
        jsonScroll.borderType = .bezelBorder
        jsonScroll.hasVerticalScroller = true
        jsonScroll.translatesAutoresizingMaskIntoConstraints = false
        jsonScroll.heightAnchor.constraint(equalToConstant: 200).isActive = true

        let texts = [L("Name"), L("Type"), L("Command"), L("Environment"), L("URL"), L("Headers"), L("About"), L("JSON")]
        let labelWidth = ceil(texts.map { formLabel($0).intrinsicContentSize.width }.max() ?? 0)
        let envLabel = pairsLabel(texts[3])
        let headersLabel = pairsLabel(texts[5])
        nameGrid = NSGridView(views: [[formLabel(texts[0]), nameField], [NSGridCell.emptyContentView, nameNote]])
        formGrid = NSGridView(views: [
            [formLabel(texts[1]), typeControl],
            [formLabel(texts[2]), commandField],
            [NSGridCell.emptyContentView, commandNote],
            [envLabel.box, envEditor],
            [formLabel(texts[4]), urlField],
            [NSGridCell.emptyContentView, urlNote],
            [headersLabel.box, headersEditor],
            [formLabel(texts[6]), aboutField],
        ])
        // In one grid now, so a constraint can tie each label to its editor's first line.
        envEditor.alignedLabel = envLabel.label
        headersEditor.alignedLabel = headersLabel.label
        jsonGrid = NSGridView(views: [[topLabel(texts[7]), jsonScroll], [NSGridCell.emptyContentView, jsonNote]])
        for grid in [nameGrid!, formGrid!, jsonGrid!] {
            grid.translatesAutoresizingMaskIntoConstraints = false
            grid.columnSpacing = 10
            grid.rowSpacing = 8
            grid.column(at: 0).width = labelWidth
            grid.column(at: 1).xPlacement = .fill
            for row in 0..<grid.numberOfRows { grid.row(at: row).yPlacement = .center }
        }
        // A label beside rows of fields sits on their first row.
        formGrid.row(at: 3).yPlacement = .top
        formGrid.row(at: 6).yPlacement = .top
        jsonGrid.row(at: 0).yPlacement = .top
        for (grid, row) in [(nameGrid!, 1), (formGrid!, 2), (formGrid!, 5), (jsonGrid!, 1)] { grid.row(at: row).topPadding = -4 }
        formGrid.cell(for: typeControl)?.xPlacement = .leading

        let spacer = NSView()
        spacer.setContentHuggingPriority(.defaultLow, for: .horizontal)
        let modeRow = Build.stack([spacer, modeControl], orientation: .horizontal, spacing: 0)
        for view in [statusSection, toolsSection, modeRow, nameGrid!, formGrid!, jsonGrid!, pasteNote, statusRow] as [NSView] {
            contentStack.addArrangedSubview(view)
            view.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
        }

        removeButton.bezelStyle = .rounded
        removeButton.attributedTitle = NSAttributedString(
            string: L("Remove…"), attributes: [.foregroundColor: NSColor.systemRed, .font: NSFont.systemFont(ofSize: NSFont.systemFontSize)])
        removeButton.target = self
        removeButton.action = #selector(confirmRemove)
        setButtons(confirm: saved == nil ? L("Add") : L("Done"), leading: removeButton)

        fill(McpForm(entry: base))
        render()
        applyMode()
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        store.observe(self) { [weak self] event in
            switch event {
            // The Runner's state moved: a sign-in finished, a connection came up or failed.
            case .rosterChanged, .snapshotReplaced: self?.load()
            default: break
            }
        }
    }

    override func viewDidAppear() {
        super.viewDidAppear()
        if saved == nil { view.window?.makeFirstResponder(nameField) }
    }

    private func formLabel(_ text: String) -> NSTextField {
        Build.label(text, font: .systemFont(ofSize: 12), color: .secondaryLabelColor)
    }

    /// A label beside rows of fields, in a box the grid tops with them. The editor keeps the label
    /// itself on the text of its first line (`PairsEditor.alignedLabel`): the first row's name, or
    /// the add button while there are no rows.
    private func pairsLabel(_ text: String) -> (box: NSView, label: NSTextField) {
        let label = formLabel(text)
        let box = NSView()
        box.translatesAutoresizingMaskIntoConstraints = false
        box.addSubview(label)
        // Where the label sits until the editor ties it to its first line.
        let resting = label.topAnchor.constraint(equalTo: box.topAnchor)
        resting.priority = .defaultLow
        NSLayoutConstraint.activate([
            label.leadingAnchor.constraint(equalTo: box.leadingAnchor),
            label.trailingAnchor.constraint(equalTo: box.trailingAnchor),
            label.topAnchor.constraint(greaterThanOrEqualTo: box.topAnchor),
            label.bottomAnchor.constraint(equalTo: box.bottomAnchor),
            resting,
        ])
        return (box, label)
    }

    /// A label set down to the first row of the fields beside it.
    private func topLabel(_ text: String) -> NSView {
        let label = formLabel(text)
        let box = NSView()
        box.translatesAutoresizingMaskIntoConstraints = false
        box.addSubview(label)
        NSLayoutConstraint.activate([
            label.leadingAnchor.constraint(equalTo: box.leadingAnchor),
            label.trailingAnchor.constraint(equalTo: box.trailingAnchor),
            label.topAnchor.constraint(equalTo: box.topAnchor, constant: 4),
            label.bottomAnchor.constraint(equalTo: box.bottomAnchor),
        ])
        return box
    }

    private func refit() {
        if view.window != nil { fitSheetToContent() }
    }

    /// Form or JSON, and in the form, the command's rows or the URL's.
    private func applyMode() {
        let json = mode == .json
        formGrid.isHidden = json
        jsonGrid.isHidden = !json
        nameGrid.isHidden = several != nil
        pasteNote.isHidden = json || saved != nil
        let remote = typeControl.selectedSegment == 1
        for row in [1, 2, 3] { formGrid.row(at: row).isHidden = remote }
        for row in [4, 5, 6] { formGrid.row(at: row).isHidden = !remote }
        updateJSONNote()
        updateControls()
        refit()
    }

    private func updateControls() {
        if let several {
            confirmButton.title = L("Add %d Servers", several.count)
        } else if saved == nil {
            confirmButton.title = L("Add")
        } else {
            confirmButton.title = isDirty ? L("Save") : L("Done")
        }
        confirmButton.isEnabled = !isBusy && (!isDirty || problem == nil)
        removeButton.isHidden = saved == nil
        removeButton.isEnabled = !isBusy
        for control in [nameField, commandField, urlField, aboutField, typeControl, modeControl] as [NSControl] { control.isEnabled = !isBusy }
        jsonView.isEditable = !isBusy
        envEditor.isEnabled = !isBusy
        headersEditor.isEnabled = !isBusy
        nameNote.stringValue = L("Bots call its tools as %@__tool.", toolPrefix)
    }

    // MARK: - The server

    /// How the saved server stands, with what to do about it, whether it is on, and its tools.
    private func render() {
        guard let server = saved else {
            statusSection.isHidden = true
            toolsSection.isHidden = true
            return
        }
        statusSection.isHidden = false
        let state = isConnecting ? (text: L("Connecting…"), color: NSColor.controlAccentColor) : server.state
        let runs = server.isEnabled && server.problem == nil
        let signsIn = server.needsSignIn && !isConnecting
        let stateRow = ActionRow(key: L("State"), value: state.text, tint: state.color, actionTitle: runs ? (signsIn ? L("Sign in") : L("Reconnect")) : nil)
        stateRow.onAction = { [weak self] in
            guard let self, !self.isConnecting, !self.isBusy else { return }
            if signsIn {
                self.signIn()
            } else {
                Task { await self.connect(fresh: true) }
            }
        }
        var rows: [NSView] = [stateRow]
        if server.signsIn, server.isSignedIn, !server.needsSignIn {
            let account = ActionRow(key: L("Account"), value: L("Signed in"), tint: .systemGreen, actionTitle: L("Sign Out"), secondActionTitle: L("Sign in again"))
            account.onAction = { [weak self] in self?.signOut() }
            account.onSecondAction = { [weak self] in self?.signIn() }
            rows.append(account)
        }
        if server.problem == nil {
            let toggle = NSSwitch()
            toggle.controlSize = .small
            toggle.state = server.isEnabled ? .on : .off
            toggle.target = self
            toggle.action = #selector(enabledChanged(_:))
            toggle.setAccessibilityLabel(L("On"))
            let row = AccessoryRow(key: L("On"), accessory: toggle)
            row.toolTip = L("Off, no bot on %@ sees it and it never starts.", runner.name)
            rows.append(row)
        }
        statusSection.setRows(rows)
        let tools = server.tools ?? []
        toolsSection.isHidden = tools.isEmpty
        if !tools.isEmpty {
            let list = ToolsList(tools: tools)
            list.onShow = { [weak self] tool, shown in self?.showTool(tool, shown: shown) }
            toolsSection.setRows([list])
        }
        refit()
    }

    /// The server as the Runner has it now: its state and its tools.
    private func load() {
        guard let current = saved, !isConnecting else { return }
        loads += 1
        let load = loads
        Task { [weak self] in
            guard let self else { return }
            guard let fresh = try? await self.store.mcpServer(current.name, on: self.runner.id), load == self.loads, !self.isClosed else { return }
            self.saved = fresh
            self.render()
            self.updateControls()
        }
    }

    /// Connects the server and waits for how it went: from scratch for Reconnect, or taking the
    /// connection a save started.
    private func connect(fresh: Bool, server: McpServer? = nil) async {
        guard let current = server ?? saved else { return }
        isConnecting = true
        loads += 1
        render()
        do {
            let answered = try await store.reconnectMcpServer(current.name, fresh: fresh, on: runner.id)
            guard !isClosed else { return }
            saved = answered
        } catch {
            guard !isClosed else { return }
            show(error.localizedDescription, color: .systemRed)
        }
        isConnecting = false
        render()
        updateControls()
    }

    private func signIn() {
        guard let current = saved else { return }
        Task { [weak self] in
            guard let self else { return }
            do {
                // The Runner notes the sign-in on the plugin, so the state reads it.
                try await self.store.connectPlugin(current.id, on: self.runner.id)
                self.load()
            } catch {
                self.alert(L("Couldn't start the sign-in"), error.localizedDescription)
            }
        }
    }

    /// Offers a tool to bots or keeps it from them, in the Runner's mcp.json; a failure puts the
    /// switch back.
    private func showTool(_ tool: String, shown: Bool) {
        guard let current = saved else { return }
        Task { [weak self] in
            guard let self else { return }
            do {
                let answered = try await self.store.setMcpTool(tool, shown: shown, server: current.name, on: self.runner.id)
                guard !self.isClosed else { return }
                self.saved = answered
            } catch {
                self.alert(shown ? L("Couldn't offer %@ to bots", tool) : L("Couldn't hide %@", tool), error.localizedDescription)
            }
            guard !self.isClosed else { return }
            self.render()
        }
    }

    /// Forgets the sign-in on the Runner. Nothing is revoked at the server; its next use asks again.
    private func signOut() {
        guard let current = saved else { return }
        Task { [weak self] in
            guard let self else { return }
            do {
                let answered = try await self.store.signOutMcpServer(current.name, on: self.runner.id)
                guard !self.isClosed else { return }
                self.saved = answered
                self.render()
                self.updateControls()
            } catch {
                self.alert(L("Couldn't sign out of %@", current.name), error.localizedDescription)
            }
        }
    }

    @objc private func enabledChanged(_ sender: NSSwitch) {
        guard let current = saved else { return }
        let on = sender.state == .on
        Task { [weak self] in
            guard let self else { return }
            do {
                let answered = try await self.store.setMcpServer(current.name, enabled: on, on: self.runner.id)
                guard !self.isClosed else { return }
                self.saved = answered
                self.render()
                if on { await self.connect(fresh: false, server: answered) }
            } catch {
                sender.state = on ? .off : .on
                self.alert(on ? L("Couldn't turn %@ on", current.name) : L("Couldn't turn %@ off", current.name), error.localizedDescription)
            }
        }
    }

    // MARK: - The JSON

    @objc private func modeChanged() {
        if modeControl.selectedSegment == 1 {
            showJSON(form.entry(over: base).json)
        } else {
            Task { await showForm() }
        }
    }

    @objc private func typeChanged() {
        applyMode()
    }

    private func showJSON(_ text: String) {
        jsonView.string = text
        mode = .json
        modeControl.selectedSegment = 1
        applyMode()
        read(text, after: 0)
    }

    /// Back to the form: the JSON's one server fills it.
    private func showForm() async {
        let text = jsonView.string
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            mode = .form
            applyMode()
            return
        }
        do {
            let servers = try await store.parseMcpJSON(text)
            guard servers.count == 1, let server = servers.first else {
                show(L("The form holds one server, and that JSON has %d.", servers.count), color: .systemOrange)
                modeControl.selectedSegment = 1
                return
            }
            guard let entry = server.entry else {
                show(server.problem ?? L("That JSON is not a server."), color: .systemRed)
                modeControl.selectedSegment = 1
                return
            }
            fill(McpForm(entry: entry))
            if let parsed = server.name, name.isEmpty { nameField.stringValue = parsed }
            hideStatus()
            mode = .form
            applyMode()
        } catch {
            show(error.localizedDescription, color: .systemRed)
            modeControl.selectedSegment = 1
        }
    }

    /// Reads the JSON once it stops changing, through the CLI on this Mac, which knows every app's
    /// spelling.
    private func read(_ text: String, after delay: Double) {
        readTask?.cancel()
        readGeneration += 1
        let generation = readGeneration
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            reading = .empty
            applyMode()
            return
        }
        reading = .reading
        updateJSONNote()
        updateControls()
        readTask = Task { [weak self] in
            if delay > 0 { try? await Task.sleep(nanoseconds: UInt64(delay * 1_000_000_000)) }
            guard !Task.isCancelled else { return }
            let result: Reading
            do {
                result = .read(try await AppStore.shared.parseMcpJSON(text))
            } catch {
                result = .failed(error.localizedDescription)
            }
            guard let self, generation == self.readGeneration else { return }
            self.reading = result
            // One named server: its name fills an empty Name.
            if case .read(let servers) = result, servers.count == 1, let parsed = servers[0].name, self.name.isEmpty {
                self.nameField.stringValue = parsed
            }
            self.applyMode()
        }
    }

    /// Under the JSON: what it holds, or why it cannot be read.
    private func updateJSONNote() {
        var color = NSColor.tertiaryLabelColor
        var text = ""
        switch reading {
        case .empty:
            text = L("One server's JSON, or the mcpServers block of Claude Desktop, Cursor, or a README.")
        case .reading:
            text = L("Reading…")
        case .failed(let message):
            text = message
            color = .systemRed
        case .read(let servers):
            if let several {
                text = L("%d servers: %@", several.count, several.map { $0.name ?? "?" }.joined(separator: ", "))
            } else if let server = servers.first {
                if let problem = server.problem {
                    text = problem
                    color = .systemRed
                } else if let entry = server.entry {
                    text = entry.isRemote ? L("A remote server at %@", entry.address) : L("A command: %@", entry.address)
                }
            }
        }
        jsonNote.stringValue = text
        jsonNote.textColor = color
    }

    /// Text that reads as JSON rather than a command, a URL, or a name: what a README's snippet is.
    private static func looksLikeJSON(_ text: String) -> Bool {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.count > 2 else { return false }
        return trimmed.hasPrefix("{") || trimmed.range(of: #"^"[^"]+"\s*:\s*\{"#, options: .regularExpression) != nil
    }

    // MARK: - Saving

    override func confirmTapped() {
        guard confirmButton.isEnabled else { return }
        guard isDirty else {
            dismiss(nil)
            return
        }
        if let several {
            addAll(several)
            return
        }
        guard let entry else { return }
        let newName = name
        let previous = saved?.name
        begin(previous == nil ? L("Adding %@…", newName) : L("Saving %@…", newName))
        task = Task { [weak self] in
            guard let self else { return }
            do {
                let server = try await self.store.saveMcpServer(newName, entry: entry, previousName: previous, on: self.runner.id)
                guard !self.isClosed else { return }
                self.saved = server
                self.setSheetTitle(server.name)
                self.nameField.stringValue = server.name
                self.fill(McpForm(entry: server.entry))
                self.mode = .form
                self.modeControl.selectedSegment = 0
                self.isBusy = false
                self.hideStatus()
                self.render()
                self.applyMode()
                // The Runner starts it once to list its tools; the sheet waits on that connection.
                if server.isEnabled { await self.connect(fresh: false, server: server) }
            } catch {
                guard !self.isClosed else { return }
                self.fail(error)
            }
        }
    }

    /// Adds the servers of pasted JSON that can run, and says which could not be added.
    private func addAll(_ servers: [ParsedServer]) {
        let usable = servers.filter { $0.name != nil && $0.entry != nil && $0.problem == nil }
        let skipped = servers.filter { $0.name == nil || $0.entry == nil || $0.problem != nil }.map {
            "\($0.name ?? "?"): \($0.problem ?? L("That JSON is not a server."))"
        }
        begin(L("Adding %d servers…", usable.count))
        task = Task { [weak self] in
            guard let self else { return }
            var failed = skipped
            for server in usable {
                guard let name = server.name, let entry = server.entry else { continue }
                do {
                    _ = try await self.store.saveMcpServer(name, entry: entry, on: self.runner.id)
                } catch {
                    failed.append("\(name): \(error.localizedDescription)")
                }
                if self.isClosed { return }
            }
            self.isBusy = false
            if failed.isEmpty {
                self.dismiss(nil)
            } else {
                self.show(failed.joined(separator: "\n"), color: .systemRed)
                self.updateControls()
            }
        }
    }

    @objc private func confirmRemove() {
        guard let window = view.window, let current = saved else { return }
        let alert = NSAlert()
        alert.messageText = L("Remove %@ from %@?", current.name, runner.name)
        alert.informativeText = L("Its entry leaves mcp.json on %@, every bot there loses its tools, and its sign-in is forgotten.", runner.name)
        alert.addButton(withTitle: L("Remove"))
        alert.addButton(withTitle: L("Cancel"))
        alert.alertStyle = .warning
        Task { [weak self] in
            guard await alert.beginSheetModal(for: window) == .alertFirstButtonReturn, let self else { return }
            self.begin(L("Removing…"))
            do {
                try await self.store.removeMcpServer(current.name, on: self.runner.id)
                self.dismiss(nil)
            } catch {
                self.fail(error)
            }
        }
    }

    private func begin(_ message: String) {
        isBusy = true
        updateControls()
        spinner.startAnimation(nil)
        status.textColor = .secondaryLabelColor
        status.stringValue = message
        statusRow.isHidden = false
        refit()
    }

    private func fail(_ error: Error) {
        isBusy = false
        show(error.localizedDescription, color: .systemRed)
        updateControls()
    }

    private func show(_ message: String, color: NSColor) {
        spinner.stopAnimation(nil)
        status.textColor = color
        status.stringValue = message
        statusRow.isHidden = false
        refit()
    }

    private func hideStatus() {
        spinner.stopAnimation(nil)
        statusRow.isHidden = true
    }

    private func alert(_ title: String, _ text: String) {
        let alert = NSAlert()
        alert.messageText = title
        alert.informativeText = text
        if let window = view.window { alert.beginSheetModal(for: window) } else { alert.runModal() }
    }

    override func dismissSheet() {
        isClosed = true
        task?.cancel()
        readTask?.cancel()
        super.dismissSheet()
    }

    override func viewDidDisappear() {
        super.viewDidDisappear()
        isClosed = true
        task?.cancel()
        readTask?.cancel()
    }
}

extension McpServerViewController: NSTextFieldDelegate, NSTextViewDelegate {
    func controlTextDidChange(_ obj: Notification) {
        // JSON pasted into a field of the form opens the JSON view with it.
        if let field = obj.object as? NSTextField, [nameField, commandField, urlField].contains(where: { $0 === field }),
            Self.looksLikeJSON(field.stringValue)
        {
            let text = field.stringValue
            field.stringValue = ""
            showJSON(text)
            return
        }
        updateControls()
    }

    func textDidChange(_ notification: Notification) {
        read(jsonView.string, after: 0.3)
    }
}

extension PluginViewController {
    /// A plugin's sheet, or for one of the Runner's mcp.json servers, the server's own.
    static func present(pluginID: String, runner: Device, bot: Bot?, chatID: Chat.ID? = nil, from presenter: NSViewController) {
        if let plugin = runner.plugins.first(where: { $0.id == pluginID }), plugin.isMcpServer {
            McpServerViewController.present(runner: runner, name: plugin.name, from: presenter)
        } else {
            presenter.presentAsSheet(PluginViewController(pluginID: pluginID, runner: runner, bot: bot, chatID: chatID))
        }
    }
}

/// Names and values as rows of fields, a button that adds a row, and one that removes each: a
/// command's environment, or a remote server's headers. A value whose name says it is a key is
/// hidden until its eye shows it.
private final class PairsEditor: NSView {
    private let stack = Build.stack([], spacing: 6)
    private let namePlaceholder: String
    private let valuePlaceholder: String
    private let addTitle: String
    private var rows: [PairRow] = []
    private lazy var addButton: NSButton = {
        let button = NSButton(title: addTitle, target: self, action: #selector(addRow))
        button.isBordered = false
        button.image = NSImage(systemSymbolName: "plus", accessibilityDescription: nil)
        button.imagePosition = .imageLeading
        button.contentTintColor = .controlAccentColor
        button.attributedTitle = NSAttributedString(
            string: addTitle, attributes: [.foregroundColor: NSColor.controlAccentColor, .font: NSFont.systemFont(ofSize: 12, weight: .medium)])
        return button
    }()

    /// Called on every edit.
    var onChange: (() -> Void)?
    /// A row came or went, so the sheet's height changes.
    var onResize: (() -> Void)?

    /// The label beside the editor, kept on the baseline of its first line: the first row's name
    /// field, or the add button while there are no rows. Set once both are in one view tree.
    weak var alignedLabel: NSView? {
        didSet { alignLabel() }
    }
    private var labelBaseline: NSLayoutConstraint?

    private func alignLabel() {
        labelBaseline?.isActive = false
        guard let alignedLabel else { return }
        let first: NSView = rows.first?.nameField ?? addButton
        let baseline = alignedLabel.firstBaselineAnchor.constraint(equalTo: first.firstBaselineAnchor)
        baseline.priority = NSLayoutConstraint.Priority(999)
        baseline.isActive = true
        labelBaseline = baseline
    }

    var isEnabled = true {
        didSet {
            for row in rows { row.isEnabled = isEnabled }
            addButton.isEnabled = isEnabled
        }
    }

    init(namePlaceholder: String, valuePlaceholder: String, addTitle: String, label: String) {
        self.namePlaceholder = namePlaceholder
        self.valuePlaceholder = valuePlaceholder
        self.addTitle = addTitle
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        setAccessibilityRole(.group)
        setAccessibilityLabel(label)
        addSubview(stack)
        stack.pin(to: self)
        stack.addArrangedSubview(addButton)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    var pairs: [(String, String)] {
        get { rows.map { ($0.name, $0.value) } }
        set {
            for row in rows {
                stack.removeArrangedSubview(row)
                row.removeFromSuperview()
            }
            rows = []
            for (name, value) in newValue { insert(name: name, value: value) }
            alignLabel()
        }
    }

    @discardableResult
    private func insert(name: String, value: String) -> PairRow {
        let row = PairRow(name: name, value: value, namePlaceholder: namePlaceholder, valuePlaceholder: valuePlaceholder)
        row.isEnabled = isEnabled
        row.onChange = { [weak self] in self?.onChange?() }
        row.onRemove = { [weak self, weak row] in
            guard let self, let row else { return }
            self.stack.removeArrangedSubview(row)
            row.removeFromSuperview()
            self.rows.removeAll { $0 === row }
            self.alignLabel()
            self.onChange?()
            self.onResize?()
        }
        stack.insertArrangedSubview(row, at: rows.count)
        row.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true
        rows.append(row)
        if rows.count == 1 { alignLabel() }
        return row
    }

    @objc private func addRow() {
        let row = insert(name: "", value: "")
        onChange?()
        onResize?()
        window?.makeFirstResponder(row.nameField)
    }
}

/// One name and value, the value in a field that hides a key, and a button that removes the row.
private final class PairRow: NSView, NSTextFieldDelegate {
    let nameField = NSTextField()
    private let valueField = APIKeyField()
    private lazy var removeButton = Build.imageButton(symbol: "minus.circle", tooltip: L("Remove"), target: self, action: #selector(removeTapped))
    var onChange: (() -> Void)?
    var onRemove: (() -> Void)?

    var name: String { nameField.stringValue }
    var value: String { valueField.stringValue }

    var isEnabled = true {
        didSet {
            nameField.isEnabled = isEnabled
            valueField.isEnabled = isEnabled
            removeButton.isEnabled = isEnabled
        }
    }

    init(name: String, value: String, namePlaceholder: String, valuePlaceholder: String) {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        nameField.stringValue = name
        nameField.placeholderString = namePlaceholder
        nameField.font = .monospacedSystemFont(ofSize: 11.5, weight: .regular)
        nameField.delegate = self
        nameField.setAccessibilityLabel(L("Name"))
        nameField.translatesAutoresizingMaskIntoConstraints = false
        valueField.stringValue = value
        valueField.placeholderString = valuePlaceholder
        valueField.setFieldLabel(L("Value"))
        valueField.setRevealed(!looksSecret(name))
        valueField.onChange = { [weak self] in self?.onChange?() }
        for view in [nameField, valueField, removeButton] as [NSView] { addSubview(view) }
        NSLayoutConstraint.activate([
            nameField.leadingAnchor.constraint(equalTo: leadingAnchor),
            nameField.centerYAnchor.constraint(equalTo: centerYAnchor),
            nameField.widthAnchor.constraint(equalTo: widthAnchor, multiplier: 0.4, constant: -14),
            valueField.leadingAnchor.constraint(equalTo: nameField.trailingAnchor, constant: 6),
            valueField.trailingAnchor.constraint(equalTo: removeButton.leadingAnchor, constant: -4),
            valueField.topAnchor.constraint(equalTo: topAnchor),
            valueField.bottomAnchor.constraint(equalTo: bottomAnchor),
            removeButton.trailingAnchor.constraint(equalTo: trailingAnchor),
            removeButton.centerYAnchor.constraint(equalTo: centerYAnchor),
            removeButton.widthAnchor.constraint(equalToConstant: 20),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func controlTextDidChange(_ obj: Notification) {
        // A key's value hides as its name comes to say it is one.
        valueField.setRevealed(!looksSecret(nameField.stringValue))
        onChange?()
    }

    @objc private func removeTapped() {
        onRemove?()
    }
}

/// The tools a server offered: each one's name, the first line of what it does, whether it only
/// reads, and a switch that offers it to bots or keeps it from them, in a list that scrolls past a
/// few.
private final class ToolsList: NSView {
    private let tools: [McpTool]
    /// A tool's switch moved: its name, and whether bots see it now.
    var onShow: ((String, Bool) -> Void)?

    init(tools: [McpTool]) {
        self.tools = tools
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        let rows = tools.enumerated().map { index, tool in row(tool, index: index) }
        let stack = Build.stack(rows, spacing: 0)
        let document = FlippedView()
        document.translatesAutoresizingMaskIntoConstraints = false
        document.addSubview(stack)
        let scroll = NSScrollView()
        scroll.documentView = document
        scroll.drawsBackground = false
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.translatesAutoresizingMaskIntoConstraints = false
        addSubview(scroll)
        scroll.pin(to: self)
        NSLayoutConstraint.activate([
            scroll.heightAnchor.constraint(equalToConstant: min(CGFloat(tools.count) * 26 + 6, 140)),
            document.widthAnchor.constraint(equalTo: scroll.widthAnchor),
            stack.topAnchor.constraint(equalTo: document.topAnchor, constant: 3),
            stack.leadingAnchor.constraint(equalTo: document.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: document.trailingAnchor),
            stack.bottomAnchor.constraint(equalTo: document.bottomAnchor, constant: -3),
        ])
        for row in stack.arrangedSubviews { row.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    private func row(_ tool: McpTool, index: Int) -> NSView {
        let name = Build.label(tool.name, font: .monospacedSystemFont(ofSize: 11, weight: .semibold), color: tool.isHidden ? .tertiaryLabelColor : .labelColor)
        name.setContentCompressionResistancePriority(.defaultHigh, for: .horizontal)
        let about = Build.label(tool.about, font: Theme.Font.caption, color: tool.isHidden ? .tertiaryLabelColor : .secondaryLabelColor)
        about.lineBreakMode = .byTruncatingTail
        about.setContentHuggingPriority(.defaultLow - 1, for: .horizontal)
        var views: [NSView] = [name, about]
        if tool.isReadOnly {
            let tag = Build.pill(L("Reads only"), color: .secondaryLabelColor)
            tag.toolTip = L("It only reads, so it runs without Auto-review.")
            views.append(tag)
        }
        let toggle = NSSwitch()
        toggle.controlSize = .mini
        toggle.state = tool.isHidden ? .off : .on
        toggle.tag = index
        toggle.target = self
        toggle.action = #selector(toggled(_:))
        toggle.toolTip = tool.isHidden ? L("Hidden from bots") : L("Offered to bots")
        toggle.setAccessibilityLabel(L("Offer %@ to bots", tool.name))
        views.append(toggle)
        let row = Build.stack(views, orientation: .horizontal, spacing: 8)
        row.edgeInsets = NSEdgeInsets(top: 4, left: 12, bottom: 4, right: 12)
        row.toolTip = tool.about
        return row
    }

    @objc private func toggled(_ sender: NSSwitch) {
        guard tools.indices.contains(sender.tag) else { return }
        onShow?(tools[sender.tag].name, sender.state == .on)
    }
}
