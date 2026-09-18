import AppKit

/// Connects a provider on this Runner. DeepSeek and Anthropic take an API key; ChatGPT and Grok
/// sign in through the browser, driven by the CLI. Credentials never leave this Mac.
final class ConnectProviderViewController: SheetViewController {
    private let store = AppStore.shared
    private let kind: ProviderCredential.Kind
    private let keyField = NSSecureTextField()
    private let baseURLField = NSTextField()
    private let initialBaseURL: String?
    private let status = Build.label("", font: Theme.Font.caption, color: .secondaryLabelColor, lines: 0)
    private let spinner = NSProgressIndicator()
    private var task: Task<Void, Never>?

    private let onDone: () -> Void

    /// `baseURL` is the custom API root the Runner already uses for this provider, if any.
    init(kind: ProviderCredential.Kind, baseURL: String? = nil, onDone: @escaping () -> Void = {}) {
        self.kind = kind
        self.initialBaseURL = baseURL
        self.onDone = onDone
        let subtitle =
            if kind.usesAPIKey {
                "The key is checked against \(kind.rawValue), then stored in the CLI on this Mac with mode 0600. Bots assigned here use it directly."
            } else {
                "Your browser opens a \(kind.rawValue) sign-in. The CLI on this Mac keeps the resulting tokens; nothing is sent to a Lorca server."
            }
        super.init(title: "Connect \(kind.rawValue)", subtitle: subtitle, width: 420)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()

        spinner.style = .spinning
        spinner.controlSize = .small
        spinner.isDisplayedWhenStopped = false
        spinner.translatesAutoresizingMaskIntoConstraints = false

        let statusRow = Build.stack([spinner, status], orientation: .horizontal, spacing: 8)
        statusRow.alignment = .centerY

        if kind.usesAPIKey {
            keyField.placeholderString = kind.keyPlaceholder
            keyField.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
            keyField.translatesAutoresizingMaskIntoConstraints = false
            keyField.delegate = self
            contentStack.addArrangedSubview(keyField)
            keyField.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true

            baseURLField.placeholderString = kind.defaultBaseURL
            baseURLField.stringValue = initialBaseURL ?? ""
            baseURLField.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
            baseURLField.translatesAutoresizingMaskIntoConstraints = false
            let baseURLNote = Build.label(
                "API base URL. Leave it empty for \(kind.rawValue); set it for a proxy or a compatible server.",
                font: Theme.Font.caption, color: .tertiaryLabelColor, lines: 0)
            contentStack.addArrangedSubview(baseURLField)
            contentStack.addArrangedSubview(baseURLNote)
            contentStack.setCustomSpacing(4, after: baseURLField)
            baseURLField.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
            baseURLNote.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
            setButtons(confirm: "Connect")
            confirmButton.isEnabled = false
        } else {
            let flow = kind == .chatgpt ? "the same OAuth flow as the Codex CLI" : "the same OAuth flow as xAI's Grok CLI"
            let note = Build.label(
                "Sign-in uses \(flow). \(kind.signInRequirement)",
                font: Theme.Font.caption, color: .tertiaryLabelColor, lines: 0)
            contentStack.addArrangedSubview(note)
            note.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
            setButtons(confirm: "Sign in with \(kind.rawValue)…")
        }

        contentStack.addArrangedSubview(statusRow)
        statusRow.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
    }

    override func confirmTapped() {
        confirmButton.isEnabled = false
        spinner.startAnimation(nil)
        status.textColor = .secondaryLabelColor
        status.stringValue = kind.usesAPIKey ? "Checking the key with \(kind.rawValue)…" : "Waiting for the browser…"

        let key = keyField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        task = Task { [weak self] in
            guard let self else { return }
            do {
                if self.kind.usesAPIKey {
                    try await self.store.connectAPIKey(self.kind, apiKey: key, baseURL: self.baseURLField.stringValue)
                } else {
                    try await self.store.connectSignIn(self.kind)
                }
                self.spinner.stopAnimation(nil)
                self.status.textColor = .systemGreen
                self.status.stringValue = "\(self.kind.rawValue) connected on this Mac."
                try? await Task.sleep(nanoseconds: 600_000_000)
                self.dismiss(nil)
                self.onDone()
            } catch {
                self.spinner.stopAnimation(nil)
                self.status.textColor = .systemRed
                self.status.stringValue = error.localizedDescription
                self.confirmButton.isEnabled = true
            }
        }
    }

    override func dismissSheet() {
        task?.cancel()
        super.dismissSheet()
    }
}

extension ConnectProviderViewController: NSTextFieldDelegate {
    func controlTextDidChange(_ obj: Notification) {
        confirmButton.isEnabled = !keyField.stringValue.trimmingCharacters(in: .whitespaces).isEmpty
    }
}
