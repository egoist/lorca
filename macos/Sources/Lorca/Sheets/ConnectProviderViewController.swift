import AppKit

/// Connects a provider for the account. API-key providers take a pasted key; ChatGPT and Grok
/// sign in through the browser, driven by the CLI. The credential reaches every paired Device
/// encrypted with the account key.
final class ConnectProviderViewController: SheetViewController {
    private let store = AppStore.shared
    private let kind: ProviderCredential.Kind
    private let secureKeyField = APIKeySecureTextField()
    private let revealedKeyField = APIKeyTextField()
    private var isKeyRevealed = false
    private var keyField: NSTextField { isKeyRevealed ? revealedKeyField : secureKeyField }
    private lazy var revealButton = Build.imageButton(
        symbol: "eye", tooltip: L("Show API key"), target: self, action: #selector(toggleKeyVisibility))
    private let disconnectButton = NSButton()
    private let baseURLField = NSTextField()
    private let initialBaseURL: String?
    private let isEditing: Bool
    private let status = Build.label("", font: Theme.Font.caption, color: .secondaryLabelColor, lines: 0)
    private let spinner = NSProgressIndicator()
    private lazy var statusRow = Build.stack([spinner, status], orientation: .horizontal, spacing: 8)
    private var task: Task<Void, Never>?
    private var isBusy = false

    private let onDone: () -> Void

    /// Fetch the saved credential before presenting, so the first visible frame is populated
    /// and masked, with no loading row changing the sheet's size.
    static func present(
        kind: ProviderCredential.Kind, from presenter: NSViewController,
        baseURL: String? = nil, onDone: @escaping () -> Void = {}
    ) {
        Task { [weak presenter] in
            do {
                let store = AppStore.shared
                let credential = kind.usesAPIKey && store.credential(for: kind)?.isConnected == true
                    ? try await store.providerAPIKey(kind) : nil
                guard let presenter, presenter.view.window != nil,
                    presenter.presentedViewControllers?.isEmpty != false else { return }
                presenter.presentAsSheet(ConnectProviderViewController(
                    kind: kind, credential: credential, baseURL: baseURL, onDone: onDone))
            } catch {
                guard let window = presenter?.view.window else { return }
                await NSAlert(error: error).beginSheetModal(for: window)
            }
        }
    }

    private init(
        kind: ProviderCredential.Kind, credential: Wire.ProviderAPIKey?, baseURL: String?,
        onDone: @escaping () -> Void
    ) {
        self.kind = kind
        self.initialBaseURL = credential == nil ? baseURL : credential?.baseUrl
        self.isEditing = credential?.apiKey != nil
        self.onDone = onDone
        let subtitle =
            if kind.usesAPIKey {
                L("Encrypted and shared with your paired Devices.")
            } else {
                L("Your browser opens a %@ sign-in. The tokens are shared with your paired Devices, encrypted with your account key; the relay cannot read them.", kind.rawValue)
            }
        super.init(title: isEditing ? kind.rawValue : L("Connect %@", kind.rawValue), subtitle: subtitle, width: 420)
        secureKeyField.stringValue = credential?.apiKey ?? ""
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()

        spinner.style = .spinning
        spinner.controlSize = .small
        spinner.isDisplayedWhenStopped = false
        spinner.translatesAutoresizingMaskIntoConstraints = false

        statusRow.alignment = .centerY
        statusRow.isHidden = true

        if kind.usesAPIKey {
            let keyContainer = NSView()
            keyContainer.translatesAutoresizingMaskIntoConstraints = false
            revealedKeyField.isHidden = true
            for field in [secureKeyField, revealedKeyField] {
                field.placeholderString = kind.keyPlaceholder
                field.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
                field.delegate = self
                field.setAccessibilityLabel(L("API key"))
                field.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
                keyContainer.addSubview(field)
                field.pin(to: keyContainer)
            }
            revealButton.setAccessibilityLabel(L("Show API key"))
            keyContainer.addSubview(revealButton)
            NSLayoutConstraint.activate([
                revealButton.trailingAnchor.constraint(equalTo: keyContainer.trailingAnchor, constant: -4),
                revealButton.centerYAnchor.constraint(equalTo: keyContainer.centerYAnchor),
                revealButton.widthAnchor.constraint(equalToConstant: 24),
                revealButton.heightAnchor.constraint(equalToConstant: 20),
            ])
            addField(keyContainer, label: L("API key"))

            baseURLField.placeholderString = kind.defaultBaseURL
            baseURLField.stringValue = initialBaseURL ?? ""
            baseURLField.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
            baseURLField.translatesAutoresizingMaskIntoConstraints = false
            let baseURLNote = Build.label(
                L("Leave empty to use %@’s API.", kind.rawValue),
                font: Theme.Font.caption, color: .tertiaryLabelColor, lines: 0)
            let baseURLRow = addField(baseURLField, label: L("API base URL"))
            contentStack.addArrangedSubview(baseURLNote)
            contentStack.setCustomSpacing(4, after: baseURLRow)
            baseURLNote.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
            disconnectButton.title = L("Disconnect")
            disconnectButton.bezelStyle = .rounded
            disconnectButton.hasDestructiveAction = true
            disconnectButton.attributedTitle = NSAttributedString(
                string: L("Disconnect"), attributes: [.foregroundColor: NSColor.systemRed, .font: NSFont.systemFont(ofSize: NSFont.systemFontSize)])
            disconnectButton.target = self
            disconnectButton.action = #selector(disconnectTapped)
            setButtons(confirm: isEditing ? L("Save") : L("Connect"), leading: isEditing ? disconnectButton : nil)
            updateControls()
        } else {
            let flow =
                kind == .chatgpt
                ? L("Sign-in uses the same OAuth flow as the Codex CLI.")
                : L("Sign-in uses the same OAuth flow as xAI's Grok CLI.")
            let note = Build.label(
                "\(flow) \(kind.signInRequirement)",
                font: Theme.Font.caption, color: .tertiaryLabelColor, lines: 0)
            contentStack.addArrangedSubview(note)
            note.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
            setButtons(confirm: L("Sign in with %@…", kind.rawValue))
        }

        contentStack.addArrangedSubview(statusRow)
        statusRow.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
    }

    override func confirmTapped() {
        guard confirmButton.isEnabled else { return }
        beginOperation(kind.usesAPIKey ? L("Checking the key with %@…", kind.rawValue) : L("Waiting for the browser…"))

        let key = keyField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        let baseURL = baseURLField.stringValue
        task = Task { [weak self] in
            guard let self else { return }
            do {
                if self.kind.usesAPIKey {
                    try await self.store.connectAPIKey(self.kind, apiKey: key, baseURL: baseURL)
                } else {
                    try await self.store.connectSignIn(self.kind)
                }
                try Task.checkCancellation()
                self.spinner.stopAnimation(nil)
                self.status.textColor = .systemGreen
                self.status.stringValue = L("%@ connected.", self.kind.rawValue)
                try await Task.sleep(nanoseconds: 600_000_000)
                self.dismiss(nil)
                self.onDone()
            } catch {
                guard !Task.isCancelled else { return }
                self.showError(error)
            }
        }
    }

    override func dismissSheet() {
        task?.cancel()
        super.dismissSheet()
    }

    override func viewDidDisappear() {
        super.viewDidDisappear()
        task?.cancel()
        secureKeyField.stringValue = ""
        revealedKeyField.stringValue = ""
    }

    @objc private func toggleKeyVisibility() {
        let oldField = keyField
        let selection = (oldField.currentEditor() as? NSTextView)?.selectedRange()
        if selection != nil { view.window?.makeFirstResponder(nil) }
        isKeyRevealed.toggle()
        keyField.stringValue = oldField.stringValue
        oldField.isHidden = true
        keyField.isHidden = false
        oldField.stringValue = ""
        let label = isKeyRevealed ? L("Hide API key") : L("Show API key")
        revealButton.image = NSImage(systemSymbolName: isKeyRevealed ? "eye.slash" : "eye", accessibilityDescription: label)
        revealButton.toolTip = label
        revealButton.setAccessibilityLabel(label)
        if let selection {
            view.window?.makeFirstResponder(keyField)
            (keyField.currentEditor() as? NSTextView)?.setSelectedRange(selection)
        }
    }

    @objc private func disconnectTapped() {
        beginOperation(L("Disconnecting…"))
        task = Task { [weak self] in
            guard let self else { return }
            do {
                try await self.store.disconnectProvider(self.kind)
                try Task.checkCancellation()
                self.dismiss(nil)
                self.onDone()
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

    private func endOperation() {
        isBusy = false
        spinner.stopAnimation(nil)
        statusRow.isHidden = status.stringValue.isEmpty
        updateControls()
        fitSheetToContent()
    }

    private func showError(_ error: Error) {
        status.textColor = .systemRed
        status.stringValue = error.localizedDescription
        endOperation()
    }

    private func updateControls() {
        secureKeyField.isEnabled = !isBusy
        revealedKeyField.isEnabled = !isBusy
        revealButton.isEnabled = !isBusy
        baseURLField.isEnabled = !isBusy
        disconnectButton.isEnabled = !isBusy
        confirmButton.isEnabled = !isBusy && (!kind.usesAPIKey || !keyField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
    }

    @discardableResult
    private func addField(_ field: NSView, label: String) -> NSStackView {
        let row = Build.stack([Build.label(label, font: Theme.Font.caption, color: .secondaryLabelColor), field], spacing: 5)
        contentStack.addArrangedSubview(row)
        row.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
        field.widthAnchor.constraint(equalTo: row.widthAnchor).isActive = true
        return row
    }
}

extension ConnectProviderViewController: NSTextFieldDelegate {
    func controlTextDidChange(_ obj: Notification) {
        updateControls()
    }
}

// Reserve space inside the native bezel for the reveal button, including while editing.
private final class APIKeyTextFieldCell: NSTextFieldCell {
    override func drawingRect(forBounds rect: NSRect) -> NSRect {
        var rect = super.drawingRect(forBounds: rect)
        rect.size.width = max(0, rect.width - 28)
        return rect
    }

    override func resetCursorRect(_ cellFrame: NSRect, in controlView: NSView) {
        let (accessory, text) = cellFrame.divided(atDistance: min(28, cellFrame.width), from: .maxXEdge)
        super.resetCursorRect(text, in: controlView)
        controlView.addCursorRect(accessory, cursor: .arrow)
    }
}

private final class APIKeySecureTextFieldCell: NSSecureTextFieldCell {
    override func drawingRect(forBounds rect: NSRect) -> NSRect {
        var rect = super.drawingRect(forBounds: rect)
        rect.size.width = max(0, rect.width - 28)
        return rect
    }

    override func resetCursorRect(_ cellFrame: NSRect, in controlView: NSView) {
        let (accessory, text) = cellFrame.divided(atDistance: min(28, cellFrame.width), from: .maxXEdge)
        super.resetCursorRect(text, in: controlView)
        controlView.addCursorRect(accessory, cursor: .arrow)
    }
}

private final class APIKeyTextField: NSTextField {
    override class var cellClass: AnyClass? {
        get { APIKeyTextFieldCell.self }
        set {}
    }
}

private final class APIKeySecureTextField: NSSecureTextField {
    override class var cellClass: AnyClass? {
        get { APIKeySecureTextFieldCell.self }
        set {}
    }
}
