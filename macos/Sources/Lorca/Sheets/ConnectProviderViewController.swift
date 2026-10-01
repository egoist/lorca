import AppKit

/// Connects a provider for the account. API-key providers take a pasted key; ChatGPT and Grok
/// sign in through the browser, driven by the CLI. The credential reaches every paired Device
/// encrypted with the account key.
final class ConnectProviderViewController: SheetViewController {
    private let store = AppStore.shared
    private let kind: ProviderCredential.Kind
    private let keyField = APIKeyField()
    private let disconnectButton = NSButton()
    private let baseURLField = NSTextField()
    private let initialBaseURL: String?
    private let isEditing: Bool
    private let status = Build.label("", font: Theme.Font.caption, color: .secondaryLabelColor, lines: 0)
    private let spinner = NSProgressIndicator()
    private lazy var statusRow = Build.stack([spinner, status], orientation: .horizontal, spacing: 8)
    private var task: Task<Void, Never>?
    private var isBusy = false
    /// The CLI is running a browser sign-in for this sheet.
    private var isSigningIn = false

    private let onDone: () -> Void

    /// Fetch the saved credential before presenting, so the first visible frame is populated
    /// and masked, with no loading row changing the sheet's size.
    static func present(
        kind: ProviderCredential.Kind, from presenter: NSViewController,
        baseURL: String? = nil, onDone: @escaping () -> Void = {}
    ) {
        if kind.isCustom {
            CustomProviderViewController.present(kind: kind, from: presenter) { _ in onDone() }
            return
        }
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
                L("Your browser opens a %@ sign-in. The tokens are shared with your paired Devices, encrypted with your account key; the relay cannot read them.", kind.name)
            }
        super.init(title: isEditing ? kind.name : L("Connect %@", kind.name), subtitle: subtitle, width: 420)
        keyField.stringValue = credential?.apiKey ?? ""
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
            keyField.placeholderString = kind.keyPlaceholder
            keyField.onChange = { [weak self] in self?.updateControls() }
            addField(keyField, label: L("API key"))

            baseURLField.placeholderString = kind.defaultBaseURL
            baseURLField.stringValue = initialBaseURL ?? ""
            baseURLField.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
            baseURLField.translatesAutoresizingMaskIntoConstraints = false
            let baseURLNote = Build.label(
                L("Leave empty to use %@’s API.", kind.name),
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
            setButtons(confirm: L("Sign in with %@…", kind.name))
        }

        contentStack.addArrangedSubview(statusRow)
        statusRow.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
    }

    override func confirmTapped() {
        guard confirmButton.isEnabled else { return }
        beginOperation(kind.usesAPIKey ? L("Checking the key with %@…", kind.name) : L("Waiting for the browser…"))

        let key = keyField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        let baseURL = baseURLField.stringValue
        task = Task { [weak self] in
            guard let self else { return }
            do {
                if self.kind.usesAPIKey {
                    try await self.store.connectAPIKey(self.kind, apiKey: key, baseURL: baseURL)
                } else {
                    self.isSigningIn = true
                    defer { self.isSigningIn = false }
                    try await self.store.connectSignIn(self.kind)
                }
                try Task.checkCancellation()
                self.spinner.stopAnimation(nil)
                self.status.textColor = .systemGreen
                self.status.stringValue = L("%@ connected.", self.kind.name)
                try await Task.sleep(nanoseconds: 600_000_000)
                self.dismiss(nil)
                self.onDone()
            } catch {
                guard !Task.isCancelled else { return }
                self.showError(error)
            }
        }
    }

    /// Cancel stops a sign-in waiting on the browser as well: the CLI drops it, so finishing
    /// there afterwards connects nothing.
    override func dismissSheet() {
        if isSigningIn { store.cancelSignIn() }
        task?.cancel()
        super.dismissSheet()
    }

    override func viewDidDisappear() {
        super.viewDidDisappear()
        task?.cancel()
        keyField.clear()
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
        keyField.isEnabled = !isBusy
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
