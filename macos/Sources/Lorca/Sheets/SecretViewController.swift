import AppKit

/// One of a Runner's secrets: what it is and where its bot uses it, a field for a new value, and
/// Delete apart from Replace. The value is never shown; a new one goes sealed to the Runner.
final class SecretViewController: SheetViewController, NSTextFieldDelegate {
    private let store = AppStore.shared
    private let secret: SavedSecret
    private let runner: Device
    private let botName: String?
    private let field = NSSecureTextField()
    private let error = Build.label("", font: Theme.Font.caption, color: .systemRed, lines: 0)
    private let deleteButton = NSButton()
    private var busy = false { didSet { updateControls() } }

    /// The secret changed or went: the list asks the Runner again.
    var onChange: (() -> Void)?

    init(secret: SavedSecret, botName: String?, runner: Device) {
        self.secret = secret
        self.runner = runner
        self.botName = botName
        let who = botName ?? L("The bot")
        let subtitle = switch secret.use {
        case .browser: L("%@ signs in to %@ with it.", who, secret.site ?? L("its sign-in page"))
        case .command, .plugin: L("%@'s commands use it.", who)
        }
        super.init(title: secret.label, subtitle: subtitle)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        field.placeholderString = L("New value")
        field.setAccessibilityLabel(L("New value for %@", secret.label))
        field.isAutomaticTextCompletionEnabled = false
        field.delegate = self
        field.translatesAutoresizingMaskIntoConstraints = false
        error.isHidden = true
        contentStack.addArrangedSubview(field)
        contentStack.addArrangedSubview(error)
        field.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
        error.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true

        deleteButton.bezelStyle = .rounded
        deleteButton.hasDestructiveAction = true
        deleteButton.attributedTitle = NSAttributedString(
            string: L("Delete…"), attributes: [.foregroundColor: NSColor.systemRed, .font: NSFont.systemFont(ofSize: NSFont.systemFontSize)])
        deleteButton.target = self
        deleteButton.action = #selector(deleteTapped)
        setButtons(confirm: L("Replace"), leading: deleteButton)
        updateControls()
    }

    override func viewDidAppear() {
        super.viewDidAppear()
        view.window?.makeFirstResponder(field)
    }

    func controlTextDidChange(_ notification: Notification) {
        error.isHidden = true
        updateControls()
    }

    private func updateControls() {
        let value = field.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        confirmButton.isEnabled = !busy && !value.isEmpty
        deleteButton.isEnabled = !busy
        field.isEnabled = !busy
    }

    override func confirmTapped() {
        guard confirmButton.isEnabled else { return }
        let value = field.stringValue
        busy = true
        Task { [weak self] in
            guard let self else { return }
            defer { busy = false }
            do {
                try await store.replaceSecret(secret.id, value: value, on: runner.id)
                onChange?()
                dismiss(nil)
            } catch {
                show(error)
            }
        }
    }

    private func show(_ failure: Error) {
        error.stringValue = failure.localizedDescription
        error.isHidden = false
        fitSheetToContent()
    }

    @objc private func deleteTapped() {
        guard let window = view.window else { return }
        Self.confirmDelete(secret, botName: botName, on: runner, in: window) { [weak self] in
            self?.onChange?()
            self?.dismiss(nil)
        }
    }

    /// Asks before deleting `secret` from `runner`, then deletes it; a failure says why.
    static func confirmDelete(_ secret: SavedSecret, botName: String?, on runner: Device, in window: NSWindow, then done: @escaping () -> Void) {
        let alert = NSAlert()
        alert.messageText = L("Delete “%@”?", secret.label)
        alert.informativeText = L("%@ asks for it again the next time it needs it.", botName ?? L("The bot"))
        alert.addButton(withTitle: L("Delete"))
        alert.addButton(withTitle: L("Cancel"))
        alert.buttons.first?.hasDestructiveAction = true
        alert.beginSheetModal(for: window) { response in
            guard response == .alertFirstButtonReturn else { return }
            Task { @MainActor in
                do {
                    try await AppStore.shared.deleteSecret(secret.id, on: runner.id)
                    done()
                } catch {
                    let failed = NSAlert()
                    failed.messageText = L("Couldn't delete “%@”", secret.label)
                    failed.informativeText = error.localizedDescription
                    _ = await failed.beginSheetModal(for: window)
                }
            }
        }
    }
}
