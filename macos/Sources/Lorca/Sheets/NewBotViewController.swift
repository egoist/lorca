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
    private let descriptionField = WrappingTextField()
    private let runnerPopup = NSPopUpButton()
    private let providerPopup = NSPopUpButton()
    private let modelPopup = NSPopUpButton()
    private let thinkingPopup = NSPopUpButton()
    /// The Thinking row, hidden for a model without levels.
    private var thinkingRow: NSView?
    /// The levels in the thinking pop-up after its Default item.
    private var shownLevels: [(id: String, label: String)] = []
    private let lookRow = Build.stack([], orientation: .horizontal, spacing: 8)
    private let note = Build.label("", font: Theme.Font.caption, color: .tertiaryLabelColor, lines: 0)

    private var lookButtons: [NSButton] = []
    private var selectedLook = 0

    private let onCreate: (Bot.ID) -> Void

    init(onCreate: @escaping (Bot.ID) -> Void) {
        self.onCreate = onCreate
        super.init(
            title: L("New Bot"),
            subtitle: L("A bot runs on one Runner and uses that machine's credentials. Phones and tablets are not Runners."),
            width: 440
        )
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()

        nameField.placeholderString = L("Name")
        descriptionField.placeholderString = L("What it does and how it should work")
        descriptionField.usesSingleLineMode = false
        descriptionField.maximumNumberOfLines = 0
        descriptionField.lineBreakMode = .byWordWrapping
        descriptionField.cell?.wraps = true
        descriptionField.cell?.isScrollable = false
        for field in [nameField, descriptionField] {
            field.translatesAutoresizingMaskIntoConstraints = false
            field.delegate = self
        }
        descriptionField.heightAnchor.constraint(greaterThanOrEqualToConstant: 54).isActive = true

        runnerPopup.translatesAutoresizingMaskIntoConstraints = false
        for device in store.runners {
            let title =
                device.isThisDevice ? L("%@ (this computer)", device.name) : device.name
            runnerPopup.addItem(withTitle: title)
        }
        runnerPopup.target = self
        runnerPopup.action = #selector(runnerChanged)

        providerPopup.translatesAutoresizingMaskIntoConstraints = false
        for kind in providerKinds {
            providerPopup.addItem(withTitle: "\(kind.name) (\(kind.subtitle))")
        }
        providerPopup.target = self
        providerPopup.action = #selector(providerChanged)
        modelPopup.translatesAutoresizingMaskIntoConstraints = false
        modelPopup.target = self
        modelPopup.action = #selector(modelChanged)
        thinkingPopup.translatesAutoresizingMaskIntoConstraints = false

        buildLookRow()

        let thinkingRow = formRow(L("Thinking"), thinkingPopup)
        self.thinkingRow = thinkingRow
        let rows = [
            formRow(L("Name"), nameField),
            formRow(L("Description"), descriptionField, topAligned: true),
            formRow(L("Look"), lookRow),
            formRow(L("Runner"), runnerPopup),
            formRow(L("Provider"), providerPopup),
            formRow(L("Model"), modelPopup),
            thinkingRow,
            note,
        ]
        // Width constraints need a common ancestor, so they go on after each row joins the stack.
        for row in rows {
            contentStack.addArrangedSubview(row)
            row.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
        }
        reloadModels()

        setButtons(confirm: L("Create Bot"))
        runnerChanged()
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

    /// The providers as the sheet opened, the built-in ones and then the custom ones, so the
    /// pop-up's indexes hold while the account's list changes.
    private lazy var providerKinds = store.providerKinds

    private var selectedProvider: ProviderCredential.Kind {
        providerKinds[max(0, providerPopup.indexOfSelectedItem)]
    }

    /// nil means the provider's default model.
    private var selectedModel: String? {
        let models = store.models(for: selectedProvider)
        let index = modelPopup.indexOfSelectedItem
        return index <= 0 || index > models.count ? nil : models[index - 1].id
    }

    /// nil means the model's default thinking level.
    private var selectedThinking: String? {
        let index = thinkingPopup.indexOfSelectedItem
        return index <= 0 || index > shownLevels.count ? nil : shownLevels[index - 1].id
    }

    @objc private func providerChanged() {
        reloadModels()
        runnerChanged()
    }

    /// A new model keeps the thinking level only if it takes it too.
    @objc private func modelChanged() {
        reloadThinking(keeping: selectedThinking)
    }

    private func reloadModels() {
        let models = store.models(for: selectedProvider)
        modelPopup.removeAllItems()
        modelPopup.addItem(withTitle: L("Default (%@)", models.first?.label ?? ""))
        for model in models { modelPopup.addItem(withTitle: model.label) }
        modelPopup.selectItem(at: 0)
        reloadThinking(keeping: nil)
    }

    /// Fills the thinking pop-up with the levels the selected model takes, selecting `level`
    /// when it is one of them and Default otherwise.
    private func reloadThinking(keeping level: String?) {
        shownLevels = store.thinkingLevels(for: selectedProvider, model: selectedModel)
        thinkingPopup.removeAllItems()
        thinkingPopup.addItem(withTitle: L("Default"))
        for level in shownLevels { thinkingPopup.addItem(withTitle: level.label) }
        thinkingPopup.selectItem(at: level.flatMap { id in shownLevels.firstIndex { $0.id == id } }.map { $0 + 1 } ?? 0)
        // A model without levels has no choice to make.
        thinkingRow?.isHidden = shownLevels.isEmpty
    }

    /// The Runner picked in the pop-up; nil while none is paired.
    private var selectedRunner: Device? {
        let runners = store.runners
        let index = runnerPopup.indexOfSelectedItem
        return runners.indices.contains(index) ? runners[index] : nil
    }

    private var trimmedName: String {
        nameField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// Create Bot needs a name and a Runner to run the bot on.
    private func updateConfirmButton() {
        confirmButton.isEnabled = !trimmedName.isEmpty && selectedRunner != nil
    }

    @objc private func runnerChanged() {
        updateConfirmButton()
        guard let runner = selectedRunner else {
            note.stringValue = L("No Runner is paired. Bots run on a Device with macOS, Linux, or Windows.")
            note.textColor = .systemOrange
            return
        }
        let provider = selectedProvider
        if store.credential(for: provider)?.isConnected == true {
            note.stringValue = L("%@ is connected. Turns run on %@.", provider.name, runner.name)
            note.textColor = .tertiaryLabelColor
        } else {
            note.stringValue =
                L("%@ is not connected yet. The bot is created now and its first turn waits until you connect it in Settings.", provider.name)
            note.textColor = .systemOrange
        }
    }

    override func confirmTapped() {
        let name = trimmedName
        guard !name.isEmpty, let runner = selectedRunner else { return }
        let look = Self.looks[selectedLook]
        let description = descriptionField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)

        let botID = store.createBot(
            name: name,
            description: description,
            symbolName: look.symbolName,
            accent: look.accent,
            runnerID: runner.id,
            provider: selectedProvider,
            model: selectedModel,
            thinking: selectedThinking
        )
        dismiss(nil)
        onCreate(botID)
    }
}

extension NewBotViewController: NSTextFieldDelegate {
    func controlTextDidChange(_ obj: Notification) {
        updateConfirmButton()
    }
}

// MARK: - Pairing

/// Shows a pairing string for another Device to paste. The CLI runs the handshake and wraps the
/// account key to the joining machine; this sheet only polls for the outcome.
final class PairingSheetViewController: SheetViewController {
    private let store = AppStore.shared
    private var pairingString = ""
    private var nonce: String?
    private let qr = NSImageView()
    private let code = Build.label(
        L("Asking the CLI for a pairing code…"), font: .monospacedSystemFont(ofSize: 10, weight: .regular),
        color: .secondaryLabelColor, lines: 3)
    private let statusLabel = Build.label(
        L("Waiting for the other Device… Done keeps this code good for ten minutes; Cancel retires it."),
        font: .systemFont(ofSize: 12), color: .secondaryLabelColor, lines: 0)
    private let spinner = NSProgressIndicator()
    /// Covers the code once a Device has used it: a code pairs one Device, and scanning it
    /// again would only fail on the phone.
    private let pairedOverlay = BackgroundView()
    private var copy: CopyFeedbackButton?
    private var task: Task<Void, Never>?

    init() {
        super.init(
            title: L("Pair a Device"),
            subtitle:
                L("On the other Device, choose Pair in onboarding (or run `lorca pair <code>`) and paste this code. The Devices run a handshake; the relay only carries ciphertext."),
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
        qr.translatesAutoresizingMaskIntoConstraints = false
        frame.addSubview(qr)

        pairedOverlay.cornerRadius = 12
        pairedOverlay.fillColor = NSColor.white.withAlphaComponent(0.9)
        pairedOverlay.isHidden = true
        let check = NSImageView()
        check.image = NSImage(systemSymbolName: "checkmark.circle.fill", accessibilityDescription: L("Paired"))
        check.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 64, weight: .regular)
        check.contentTintColor = .systemGreen
        check.translatesAutoresizingMaskIntoConstraints = false
        pairedOverlay.addSubview(check)
        frame.addSubview(pairedOverlay)

        let codeBox = BackgroundView()
        codeBox.cornerRadius = 8
        codeBox.fillColor = Theme.codeBackground
        code.isSelectable = true
        code.lineBreakMode = .byCharWrapping
        let copy = CopyFeedbackButton()
        copy.image = NSImage(systemSymbolName: "doc.on.doc", accessibilityDescription: L("Copy pairing string"))
        copy.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 11, weight: .medium)
        copy.bezelStyle = .accessoryBarAction
        copy.isBordered = false
        copy.toolTip = L("Copy pairing string")
        copy.target = self
        copy.action = #selector(copyPairingString(_:))
        copy.translatesAutoresizingMaskIntoConstraints = false
        self.copy = copy
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
            pairedOverlay.topAnchor.constraint(equalTo: frame.topAnchor),
            pairedOverlay.leadingAnchor.constraint(equalTo: frame.leadingAnchor),
            pairedOverlay.trailingAnchor.constraint(equalTo: frame.trailingAnchor),
            pairedOverlay.bottomAnchor.constraint(equalTo: frame.bottomAnchor),
            check.centerXAnchor.constraint(equalTo: pairedOverlay.centerXAnchor),
            check.centerYAnchor.constraint(equalTo: pairedOverlay.centerYAnchor),

            codeBox.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            code.leadingAnchor.constraint(equalTo: codeBox.leadingAnchor, constant: 10),
            code.topAnchor.constraint(equalTo: codeBox.topAnchor, constant: 8),
            code.bottomAnchor.constraint(equalTo: codeBox.bottomAnchor, constant: -8),
            copy.leadingAnchor.constraint(equalTo: code.trailingAnchor, constant: 8),
            copy.trailingAnchor.constraint(equalTo: codeBox.trailingAnchor, constant: -8),
            copy.centerYAnchor.constraint(equalTo: codeBox.centerYAnchor),
            status.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
        ])

        setButtons(confirm: L("Done"))
        begin()
    }

    private func begin() {
        if store.isMock {
            pairingString = MockData.pairingString()
            qr.image = QRCode.image(for: pairingString, size: 180)
            code.stringValue = pairingString
            return
        }
        task = Task { [weak self] in
            guard let self else { return }
            do {
                let started = try await self.store.startPairing()
                self.nonce = started.nonce
                self.pairingString = started.pairingString
                self.qr.image = QRCode.image(for: started.pairingString, size: 180)
                self.code.stringValue = started.pairingString
                while !Task.isCancelled {
                    try await Task.sleep(nanoseconds: 1_500_000_000)
                    let status = try await self.store.pairingStatus(nonce: started.nonce)
                    switch status.state {
                    case "completed":
                        self.markPaired(with: status.device?.name)
                        return
                    case "failed":
                        throw CLIClient.RequestError(message: status.error ?? L("Pairing failed"))
                    default:
                        continue
                    }
                }
            } catch is CancellationError {
            } catch {
                self.spinner.stopAnimation(nil)
                self.spinner.isHidden = true
                self.statusLabel.stringValue = error.localizedDescription
                self.statusLabel.textColor = .systemRed
            }
        }
    }

    /// The code is spent: cover it, retire the copy button, and say who joined.
    private func markPaired(with deviceName: String?) {
        spinner.stopAnimation(nil)
        spinner.isHidden = true
        pairedOverlay.isHidden = false
        copy?.isEnabled = false
        copy?.isHidden = true
        code.stringValue = L("Paired with %@. This code is used up; pair another Device with a fresh one.", deviceName ?? L("the other Device"))
        code.font = .systemFont(ofSize: 12)
        code.textColor = .labelColor
        statusLabel.stringValue = L("Paired. The account key is wrapped to that machine.")
        statusLabel.textColor = .systemGreen
    }

    @objc private func copyPairingString(_ sender: CopyFeedbackButton) {
        NSPasteboard.general.clearContents()
        if NSPasteboard.general.setString(pairingString, forType: .string) {
            sender.showCopied()
        }
    }

    /// Cancel retires the code: the CLI stops waiting and the relay drops the mailbox, so a
    /// Device that pastes it afterwards is told at once.
    override func dismissSheet() {
        task?.cancel()
        if let nonce, statusLabel.textColor != .systemGreen {
            store.cancelPairing(nonce: nonce)
        }
        super.dismissSheet()
    }

    /// Done keeps the code good: the CLI goes on waiting for ten minutes, so copying the code
    /// and closing this sheet before pasting it on the phone is fine.
    override func confirmTapped() {
        task?.cancel()
        dismiss(nil)
    }
}
