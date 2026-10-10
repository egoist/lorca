import AppKit

/// Gets the account an email address, or changes the one it has: a random name, or one of the
/// user's own when nobody has it. Changing gives the old name up.
final class EmailAddressViewController: SheetViewController, NSTextFieldDelegate {
    private let store = AppStore.shared
    private let current: MailStatus.Address?
    private let domain: String
    private let random = NSButton(radioButtonWithTitle: L("Random name"), target: nil, action: nil)
    private let own = NSButton(radioButtonWithTitle: L("Your own name"), target: nil, action: nil)
    private let field = NSTextField()
    private let problem = Build.label("", font: Theme.Font.caption, color: .systemRed, lines: 0)
    /// Where a radio button's title starts, past its circle.
    private static let titleIndent: CGFloat = 22

    init(current: MailStatus.Address?, domain: String) {
        self.current = current
        self.domain = domain
        super.init(title: current == nil ? L("Get an Email Address") : L("Change Email Address"), subtitle: "", width: 420)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()
        for button in [random, own] {
            button.target = self
            button.action = #selector(choiceChanged)
        }
        random.state = .on

        field.placeholderString = L("name")
        field.font = .systemFont(ofSize: 13)
        field.delegate = self
        field.setAccessibilityLabel(L("Your own name"))
        field.setContentHuggingPriority(.defaultLow, for: .horizontal)
        let suffix = Build.label("@\(domain)", font: .systemFont(ofSize: 13), color: .secondaryLabelColor)
        suffix.setContentCompressionResistancePriority(.required, for: .horizontal)
        suffix.setContentHuggingPriority(.required, for: .horizontal)
        let name = Build.stack([field, suffix], orientation: .horizontal, spacing: 4)
        name.alignment = .firstBaseline

        // The field and what is wrong with it line up with their radio button's title.
        let ownName = Build.stack([name, problem], spacing: 6)
        ownName.edgeInsets = NSEdgeInsets(top: 0, left: Self.titleIndent, bottom: 0, right: 0)
        problem.isHidden = true

        let choices = Build.stack([random, own], spacing: 8)
        contentStack.addArrangedSubview(choices)
        contentStack.addArrangedSubview(ownName)
        contentStack.setCustomSpacing(8, after: choices)
        ownName.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
        name.widthAnchor.constraint(equalTo: ownName.widthAnchor, constant: -Self.titleIndent).isActive = true
        problem.widthAnchor.constraint(equalTo: name.widthAnchor).isActive = true

        if let current {
            let note = Build.label(
                L("%@ stops working: mail to it bounces, and nobody else can take the name.", current.email),
                font: Theme.Font.caption, color: .secondaryLabelColor, lines: 0)
            contentStack.addArrangedSubview(note)
            contentStack.setCustomSpacing(16, after: ownName)
            note.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
        }
        setButtons(confirm: current == nil ? L("Get Address") : L("Change Address"))
        update()
    }

    @objc private func choiceChanged() {
        if own.state == .on { view.window?.makeFirstResponder(field) }
        update()
    }

    func controlTextDidChange(_ obj: Notification) {
        let lowered = field.stringValue.lowercased()
        if lowered != field.stringValue { field.stringValue = lowered }
        // Typing a name picks it.
        if !field.stringValue.isEmpty { own.state = .on }
        update()
    }

    private var name: String { MailStatus.normalized(field.stringValue) }

    /// A name with a character the relay takes no part of says so at once; one too short or
    /// with a dot at an end only keeps the button off.
    private func update() {
        let allowed = Set("abcdefghijklmnopqrstuvwxyz0123456789.-")
        let badCharacters = own.state == .on && !name.allSatisfy(allowed.contains)
        show(badCharacters ? .invalid : nil)
        confirmButton.isEnabled = random.state == .on || MailStatus.isValidName(name)
    }

    private func show(_ problem: MailNameProblem?) {
        let hidden = problem == nil
        self.problem.stringValue = problem?.text ?? ""
        guard self.problem.isHidden != hidden else { return }
        self.problem.isHidden = hidden
        fitSheetToContent()
    }

    override func confirmTapped() {
        let chosen = own.state == .on ? name : nil
        if chosen == current?.name {
            return dismiss(nil)
        }
        confirmButton.isEnabled = false
        Task { [weak self] in
            guard let self else { return }
            do {
                if let problem = try await store.applyForMail(name: chosen) {
                    show(problem)
                    confirmButton.isEnabled = true
                    view.window?.makeFirstResponder(field)
                } else {
                    dismiss(nil)
                }
            } catch {
                let failed = NSAlert()
                failed.messageText = current == nil ? L("Couldn't get an address") : L("Couldn't change the address")
                failed.informativeText = error.localizedDescription
                failed.addButton(withTitle: L("OK"))
                if let window = view.window { await failed.beginSheetModal(for: window) }
                confirmButton.isEnabled = true
            }
        }
    }
}
