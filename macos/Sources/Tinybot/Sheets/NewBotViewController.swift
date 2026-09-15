import AppKit

final class NewBotViewController: SheetViewController {
    private struct Look {
        let symbolName: String
        let accent: Accent
    }

    private static let looks: [Look] = [
        Look(symbolName: "sparkles", accent: .indigo),
        Look(symbolName: "chevron.left.forwardslash.chevron.right", accent: .blue),
        Look(symbolName: "binoculars.fill", accent: .teal),
        Look(symbolName: "pencil.and.scribble", accent: .pink),
        Look(symbolName: "bolt.horizontal.fill", accent: .orange),
        Look(symbolName: "leaf.fill", accent: .green),
        Look(symbolName: "wand.and.stars", accent: .purple),
        Look(symbolName: "flame.fill", accent: .red),
    ]

    private let store = AppStore.shared
    private let nameField = NSTextField()
    private let taglineField = NSTextField()
    private let computerPopup = NSPopUpButton()
    private let providerPopup = NSPopUpButton()
    private let lookRow = Build.stack([], orientation: .horizontal, spacing: 8)
    private let note = Build.label("", font: Theme.Font.caption, color: .tertiaryLabelColor, lines: 0)

    private var lookButtons: [NSButton] = []
    private var selectedLook = 0

    init() {
        super.init(
            title: "New Bot",
            subtitle: "A bot runs on one Computer and uses that machine's credentials.",
            width: 440
        )
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()

        nameField.placeholderString = "Name"
        taglineField.placeholderString = "What it is good at"
        for field in [nameField, taglineField] {
            field.translatesAutoresizingMaskIntoConstraints = false
            field.delegate = self
        }

        computerPopup.translatesAutoresizingMaskIntoConstraints = false
        for computer in store.computers {
            let title =
                computer.isThisComputer ? "\(computer.name) (this Mac)" : computer.name
            computerPopup.addItem(withTitle: title)
        }
        computerPopup.target = self
        computerPopup.action = #selector(computerChanged)

        providerPopup.translatesAutoresizingMaskIntoConstraints = false
        for kind in ProviderCredential.Kind.allCases {
            providerPopup.addItem(withTitle: "\(kind.rawValue) (\(kind.subtitle))")
        }

        buildLookRow()

        contentStack.addArrangedSubview(labeled("Name", nameField))
        contentStack.addArrangedSubview(labeled("Tagline", taglineField))
        contentStack.addArrangedSubview(labeled("Look", lookRow))
        contentStack.addArrangedSubview(labeled("Computer", computerPopup))
        contentStack.addArrangedSubview(labeled("Provider", providerPopup))
        contentStack.addArrangedSubview(note)
        note.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true

        setButtons(confirm: "Create Bot")
        confirmButton.isEnabled = false
        computerChanged()
    }

    private func labeled(_ title: String, _ control: NSView) -> NSView {
        let container = NSView()
        container.translatesAutoresizingMaskIntoConstraints = false
        let label = Build.label(title, font: .systemFont(ofSize: 12), color: .secondaryLabelColor)
        container.addSubview(label)
        container.addSubview(control)
        NSLayoutConstraint.activate([
            label.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            label.centerYAnchor.constraint(equalTo: control.centerYAnchor),
            label.widthAnchor.constraint(equalToConstant: 76),
            control.leadingAnchor.constraint(equalTo: label.trailingAnchor, constant: 10),
            control.trailingAnchor.constraint(lessThanOrEqualTo: container.trailingAnchor),
            control.topAnchor.constraint(equalTo: container.topAnchor),
            control.bottomAnchor.constraint(equalTo: container.bottomAnchor),
        ])
        if control is NSTextField || control is NSPopUpButton {
            control.trailingAnchor.constraint(equalTo: container.trailingAnchor).isActive = true
        }
        container.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
        return container
    }

    private func buildLookRow() {
        for (index, look) in Self.looks.enumerated() {
            let button = NSButton()
            button.isBordered = false
            button.title = ""
            button.target = self
            button.action = #selector(pickLook(_:))
            button.tag = index
            button.translatesAutoresizingMaskIntoConstraints = false

            let avatar = AvatarView(diameter: 26)
            avatar.content = .bot(symbolName: look.symbolName, accent: look.accent)
            button.addSubview(avatar)
            avatar.pin(to: button)

            NSLayoutConstraint.activate([
                button.widthAnchor.constraint(equalToConstant: 26),
                button.heightAnchor.constraint(equalToConstant: 26),
            ])

            lookButtons.append(button)
            lookRow.addArrangedSubview(button)
        }
        updateLookSelection()
    }

    @objc private func pickLook(_ sender: NSButton) {
        selectedLook = sender.tag
        updateLookSelection()
    }

    private func updateLookSelection() {
        for (index, button) in lookButtons.enumerated() {
            button.alphaValue = index == selectedLook ? 1 : 0.35
        }
    }

    @objc private func computerChanged() {
        let computer = store.computers[computerPopup.indexOfSelectedItem]
        if computer.connectedProviders.isEmpty {
            note.stringValue =
                "\(computer.name) has no provider connected yet. The bot is created now and its first turn waits until you connect one there."
            note.textColor = .systemOrange
        } else {
            let names = computer.connectedProviders.map(\.kind.rawValue).joined(separator: " and ")
            note.stringValue = "\(computer.name) has \(names) connected. Turns run there."
            note.textColor = .tertiaryLabelColor
        }
    }

    override func confirmTapped() {
        let name = nameField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty else { return }
        let look = Self.looks[selectedLook]
        let tagline = taglineField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)

        store.createBot(
            name: name,
            tagline: tagline.isEmpty ? "New bot" : tagline,
            symbolName: look.symbolName,
            accent: look.accent,
            computerID: store.computers[computerPopup.indexOfSelectedItem].id,
            provider: ProviderCredential.Kind.allCases[providerPopup.indexOfSelectedItem]
        )
        dismiss(nil)
    }
}

extension NewBotViewController: NSTextFieldDelegate {
    func controlTextDidChange(_ obj: Notification) {
        confirmButton.isEnabled = !nameField.stringValue
            .trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }
}

// MARK: - Pairing

final class PairingSheetViewController: SheetViewController {
    private let pairingString = MockData.pairingString()
    private let statusLabel = Build.label(
        "Waiting for the other Computer…", font: .systemFont(ofSize: 12),
        color: .secondaryLabelColor)
    private let spinner = NSProgressIndicator()
    private var task: Task<Void, Never>?

    init() {
        super.init(
            title: "Pair a Computer",
            subtitle:
                "Open Tinybot on the other Mac and scan this code. The two machines run a handshake; the relay only carries the ciphertext.",
            width: 400
        )
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()
        contentStack.alignment = .centerX

        let frame = BackgroundView()
        frame.cornerRadius = 12
        frame.fillColor = .white
        let qr = NSImageView()
        qr.image = QRCode.image(for: pairingString, size: 180)
        qr.translatesAutoresizingMaskIntoConstraints = false
        frame.addSubview(qr)

        let codeBox = BackgroundView()
        codeBox.cornerRadius = 8
        codeBox.fillColor = Theme.codeBackground
        let code = Build.label(
            pairingString, font: .monospacedSystemFont(ofSize: 10, weight: .regular),
            color: .secondaryLabelColor, lines: 2)
        code.isSelectable = true
        let copy = Build.imageButton(
            symbol: "doc.on.doc", pointSize: 11, tooltip: "Copy pairing string", target: self,
            action: #selector(copyPairingString))
        codeBox.addSubview(code)
        codeBox.addSubview(copy)

        spinner.style = .spinning
        spinner.controlSize = .small
        spinner.translatesAutoresizingMaskIntoConstraints = false
        spinner.startAnimation(nil)

        let status = Build.stack([spinner, statusLabel], orientation: .horizontal, spacing: 8)

        contentStack.addArrangedSubview(frame)
        contentStack.addArrangedSubview(codeBox)
        contentStack.addArrangedSubview(status)

        NSLayoutConstraint.activate([
            qr.widthAnchor.constraint(equalToConstant: 180),
            qr.heightAnchor.constraint(equalToConstant: 180),
            qr.topAnchor.constraint(equalTo: frame.topAnchor, constant: 10),
            qr.leadingAnchor.constraint(equalTo: frame.leadingAnchor, constant: 10),
            qr.trailingAnchor.constraint(equalTo: frame.trailingAnchor, constant: -10),
            qr.bottomAnchor.constraint(equalTo: frame.bottomAnchor, constant: -10),

            codeBox.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            code.leadingAnchor.constraint(equalTo: codeBox.leadingAnchor, constant: 10),
            code.topAnchor.constraint(equalTo: codeBox.topAnchor, constant: 8),
            code.bottomAnchor.constraint(equalTo: codeBox.bottomAnchor, constant: -8),
            copy.leadingAnchor.constraint(equalTo: code.trailingAnchor, constant: 8),
            copy.trailingAnchor.constraint(equalTo: codeBox.trailingAnchor, constant: -8),
            copy.centerYAnchor.constraint(equalTo: codeBox.centerYAnchor),
        ])

        setButtons(confirm: "Done")
        simulateHandshake()
    }

    private func simulateHandshake() {
        task = Task { [weak self] in
            try? await Task.sleep(nanoseconds: 3_400_000_000)
            guard let self, !Task.isCancelled else { return }
            self.spinner.stopAnimation(nil)
            self.spinner.isHidden = true
            self.statusLabel.stringValue = "Paired. The account key is wrapped to that machine."
            self.statusLabel.textColor = .systemGreen
        }
    }

    @objc private func copyPairingString() {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(pairingString, forType: .string)
    }

    override func dismissSheet() {
        task?.cancel()
        super.dismissSheet()
    }

    override func confirmTapped() {
        task?.cancel()
        dismiss(nil)
    }
}
