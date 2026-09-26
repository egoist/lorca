import AppKit

final class OnboardingWindowController: NSWindowController, NSWindowDelegate {
    private let controller: OnboardingViewController
    private let onClose: () -> Void

    /// True once the identity exists and the user is on the closing step, so an
    /// `identity.changed` event does not yank the window away mid-flow.
    var isOnFinalStep: Bool { controller.isOnFinalStep }

    /// `onFinish` runs when the user completes onboarding, `onClose` whenever its window closes.
    init(onFinish: @escaping () -> Void, onClose: @escaping () -> Void) {
        let controller = OnboardingViewController(onFinish: onFinish)
        self.controller = controller
        self.onClose = onClose
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 660, height: 720),
            styleMask: [.titled, .closable, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        window.titlebarAppearsTransparent = true
        window.titleVisibility = .hidden
        window.isMovableByWindowBackground = true
        window.contentViewController = controller
        window.backgroundColor = .windowBackgroundColor
        super.init(window: window)
        window.delegate = self
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func windowWillClose(_ notification: Notification) {
        onClose()
    }
}

final class OnboardingViewController: NSViewController {
    private enum Step {
        case welcome
        case create
        case restore
        case pair
        case bot
        case provider
        case done
    }

    private let store = AppStore.shared
    private let onFinish: () -> Void
    private let container = NSView()
    private var step: Step = .welcome
    private var savedPhrase = false
    private var phrase: [String] = []
    private var pairingTask: Task<Void, Never>?
    private var busy = false

    var isOnFinalStep: Bool { step == .done || step == .create || step == .bot || step == .provider }
    private var providerKind: ProviderCredential.Kind = .deepseek
    private var harnessKind: Bot.Harness = .lorca
    private var codexSelection = CodexSelection()

    init(onFinish: @escaping () -> Void) {
        self.onFinish = onFinish
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    deinit {
        pairingTask?.cancel()
    }

    override func loadView() {
        let root = NSView()
        root.translatesAutoresizingMaskIntoConstraints = false
        container.translatesAutoresizingMaskIntoConstraints = false
        root.addSubview(container)
        NSLayoutConstraint.activate([
            root.widthAnchor.constraint(equalToConstant: 660),
            root.heightAnchor.constraint(equalToConstant: 720),
            container.topAnchor.constraint(equalTo: root.topAnchor, constant: 28),
            container.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: 44),
            container.trailingAnchor.constraint(equalTo: root.trailingAnchor, constant: -44),
            container.bottomAnchor.constraint(equalTo: root.bottomAnchor, constant: -36),
        ])
        view = root
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        transition(to: .welcome)
        store.observe(self) { [weak self] event in
            guard let self, case .identityChanged = event, self.store.hasIdentity == true, !self.busy else { return }
            // Identity arrived from elsewhere (the CLI, or a paired Device) while we waited here.
            if self.step == .welcome || self.step == .restore || self.step == .pair {
                self.transition(to: .done)
            }
        }
    }

    // MARK: - Navigation

    private func transition(to newStep: Step) {
        step = newStep
        for subview in container.subviews { subview.removeFromSuperview() }

        let content: NSView
        switch newStep {
        case .welcome: content = welcomeView()
        case .create: content = createView()
        case .restore: content = restoreView()
        case .pair: content = pairView()
        case .bot: content = botView()
        case .provider: content = providerView()
        case .done: content = doneView()
        }

        content.alphaValue = 0
        container.addSubview(content)
        content.pin(to: container)

        NSAnimationContext.runAnimationGroup { context in
            context.duration = 0.22
            content.animator().alphaValue = 1
        }
    }

    // MARK: - Steps

    private func welcomeView() -> NSView {
        let icon = NSImageView()
        icon.image = AppIcon.make(size: 96)
        icon.translatesAutoresizingMaskIntoConstraints = false

        let title = Build.label(
            AppInfo.name, font: .systemFont(ofSize: 30, weight: .bold), alignment: .center)
        let subtitle = Build.label(
            L("Bots that run on computers you own. Your identity is a key pair on this computer — no account, no server that can read your chats."),
            font: .systemFont(ofSize: 13), color: .secondaryLabelColor, lines: 0, alignment: .center
        )

        let create = primaryButton(L("Create a New Identity"), action: #selector(createIdentity))
        let restore = secondaryButton(L("Restore from Backup Phrase"), action: #selector(goRestore))
        let pair = secondaryButton(L("Pair with Another Device"), action: #selector(goPair))

        let buttons = Build.stack([create, restore, pair], spacing: 10)
        buttons.alignment = .centerX

        let column = Build.stack([icon, title, subtitle, buttons], spacing: 14)
        column.alignment = .centerX
        column.setCustomSpacing(20, after: icon)
        column.setCustomSpacing(30, after: subtitle)

        let host = NSView()
        host.translatesAutoresizingMaskIntoConstraints = false
        host.addSubview(column)
        NSLayoutConstraint.activate([
            icon.widthAnchor.constraint(equalToConstant: 96),
            icon.heightAnchor.constraint(equalToConstant: 96),
            subtitle.widthAnchor.constraint(equalToConstant: 420),
            create.widthAnchor.constraint(equalToConstant: 280),
            restore.widthAnchor.constraint(equalToConstant: 280),
            pair.widthAnchor.constraint(equalToConstant: 280),
            column.centerXAnchor.constraint(equalTo: host.centerXAnchor),
            column.centerYAnchor.constraint(equalTo: host.centerYAnchor),
        ])
        return host
    }

    private func createView() -> NSView {
        let title = Build.label(L("Your backup phrase"), font: .systemFont(ofSize: 22, weight: .semibold))
        let subtitle = Build.label(
            L("This phrase is your master secret. It re-derives every key and unwraps everything on the relay. Write it down — nobody can reset it for you."),
            font: .systemFont(ofSize: 12.5), color: .secondaryLabelColor, lines: 0
        )

        let grid = phraseGrid(words: phrase)

        let copy = CopyFeedbackButton(title: L("Copy Phrase"), target: self, action: #selector(copyPhrase(_:)))
        copy.bezelStyle = .rounded

        let confirm = NSButton(
            checkboxWithTitle: L("I wrote this phrase down somewhere safe"), target: self,
            action: #selector(togglePhraseSaved))
        confirm.state = savedPhrase ? .on : .off
        confirm.translatesAutoresizingMaskIntoConstraints = false

        // The identity already exists on this computer; there is no way back from here.
        let back = secondaryButton(L("Back"), action: #selector(goWelcome))
        back.isHidden = true
        let next = primaryButton(L("Continue"), action: #selector(goBot))
        next.isEnabled = savedPhrase
        next.identifier = NSUserInterfaceItemIdentifier("continue")

        return stepLayout(
            title: title,
            subtitle: subtitle,
            body: Build.stack([grid, copy, confirm], spacing: 16),
            back: back,
            next: next
        )
    }

    private func restoreView() -> NSView {
        let title = Build.label(L("Restore your identity"), font: .systemFont(ofSize: 22, weight: .semibold))
        let subtitle = Build.label(
            L("Paste the twelve groups from your backup phrase. Everything else is re-derived and unwrapped from the relay."),
            font: .systemFont(ofSize: 12.5), color: .secondaryLabelColor, lines: 0
        )

        let field = NSTextField()
        field.placeholderString = "k4mq 7rth 2bnz …"
        field.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
        field.translatesAutoresizingMaskIntoConstraints = false
        field.heightAnchor.constraint(equalToConstant: 26).isActive = true
        field.widthAnchor.constraint(equalToConstant: 560).isActive = true
        field.identifier = NSUserInterfaceItemIdentifier("phrase")

        let hint = Build.label(
            store.relayURL == nil
                ? L("Restoring unwraps the account key from the relay. Set a relay URL in Settings › Advanced first.")
                : L("Restoring never contacts a login server. The relay only answers a signature challenge."),
            font: Theme.Font.caption, color: .tertiaryLabelColor, lines: 0)
        hint.widthAnchor.constraint(equalToConstant: 560).isActive = true
        hint.identifier = NSUserInterfaceItemIdentifier("status")

        let back = secondaryButton(L("Back"), action: #selector(goWelcome))
        let next = primaryButton(L("Restore"), action: #selector(restoreIdentity))

        return stepLayout(
            title: title,
            subtitle: subtitle,
            body: Build.stack([field, hint], spacing: 10),
            back: back,
            next: next
        )
    }

    private func pairView() -> NSView {
        let title = Build.label(L("Pair this computer"), font: .systemFont(ofSize: 22, weight: .semibold))
        let subtitle = Build.label(
            L("On a computer that already has your identity, choose File › Pair a Device in the desktop app or run lorca pair in a terminal, then paste the code here."),
            font: .systemFont(ofSize: 12.5), color: .secondaryLabelColor, lines: 0
        )

        let field = NSTextField()
        field.placeholderString = "lorca://pair?relay=…"
        field.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
        field.translatesAutoresizingMaskIntoConstraints = false
        field.heightAnchor.constraint(equalToConstant: 26).isActive = true
        field.widthAnchor.constraint(equalToConstant: 560).isActive = true
        field.identifier = NSUserInterfaceItemIdentifier("pairing")

        let status = Build.label(
            L("The two Devices run a handshake; the relay only carries the ciphertext. This computer joins as a Runner."),
            font: Theme.Font.caption, color: .tertiaryLabelColor, lines: 0)
        status.widthAnchor.constraint(equalToConstant: 560).isActive = true
        status.identifier = NSUserInterfaceItemIdentifier("status")

        let back = secondaryButton(L("Back"), action: #selector(goWelcome))
        let next = primaryButton(L("Pair"), action: #selector(acceptPairing))

        return stepLayout(
            title: title, subtitle: subtitle, body: Build.stack([field, status], spacing: 10), back: back,
            next: next)
    }

    /// The bot the CLI created with the identity, or the first bot in the roster.
    private var firstBot: Bot? {
        store.bots.first { $0.name == "Chef" } ?? store.bots.first
    }

    private func botView() -> NSView {
        let title = Build.label(L("Your first bot"), font: .systemFont(ofSize: 22, weight: .semibold))
        let subtitle = Build.label(
            L("It runs on this computer, plans your work, and builds the rest of the team when you ask. Give it a name and choose how it runs."),
            font: .systemFont(ofSize: 12.5), color: .secondaryLabelColor, lines: 0
        )

        let avatar = AvatarView(diameter: 40)
        avatar.content = firstBot.map { AvatarView.content(for: $0) } ?? .bot(symbolName: "sparkles", accent: .indigo)

        let nameField = NSTextField()
        // Localize the CLI's default profile while preserving any saved edits.
        if let name = firstBot?.name, name != "Chef" {
            nameField.stringValue = name
        } else {
            nameField.stringValue = L("Chef", context: "first bot name")
        }
        nameField.placeholderString = L("Name")
        nameField.identifier = NSUserInterfaceItemIdentifier("botName")
        let descriptionField = WrappingTextField()
        if let description = firstBot?.description,
            description != "Chief of staff. Plans the work and delegates each task to the right teammate, proposing a new one when none fits. Does hands-on work when necessary."
        {
            descriptionField.stringValue = description
        } else {
            descriptionField.stringValue = L("Chief of staff. Plans the work and delegates each task to the right teammate, proposing a new one when none fits. Does hands-on work when necessary.")
        }
        descriptionField.placeholderString = L("What it does and how it should work")
        descriptionField.usesSingleLineMode = false
        descriptionField.maximumNumberOfLines = 3
        descriptionField.lineBreakMode = .byWordWrapping
        descriptionField.cell?.wraps = true
        descriptionField.cell?.isScrollable = false
        descriptionField.heightAnchor.constraint(greaterThanOrEqualToConstant: 54).isActive = true
        descriptionField.identifier = NSUserInterfaceItemIdentifier("botDescription")

        let harnessPicker = NSPopUpButton()
        harnessPicker.addItems(withTitles: Bot.Harness.allCases.map(\.title))
        harnessPicker.selectItem(at: Bot.Harness.allCases.firstIndex(of: harnessKind) ?? 0)
        harnessPicker.target = self
        harnessPicker.action = #selector(harnessPicked(_:))
        harnessPicker.identifier = NSUserInterfaceItemIdentifier("harness")

        let grid = formGrid([
            ("", avatar),
            (L("Name"), nameField),
            (L("Description"), descriptionField),
            (L("Runtime"), harnessPicker),
        ] + providerRows())

        let back = secondaryButton(L("Back"), action: #selector(goWelcome))
        back.isHidden = true
        let next = primaryButton(L("Continue"), action: #selector(saveFirstBot))
        next.identifier = NSUserInterfaceItemIdentifier("continue")
        let layout = stepLayout(title: title, subtitle: subtitle, body: card(grid), back: back, next: next)
        DispatchQueue.main.async { [weak self] in self?.renderCredential() }
        return layout
    }

    private func providerView() -> NSView {
        let title = Build.label(L("Connect a provider"), font: .systemFont(ofSize: 22, weight: .semibold))
        let subtitle = Build.label(
            L("Credentials belong to your account: your bots use them on every Runner you pair. They sync encrypted with your account key; the relay cannot read them."),
            font: .systemFont(ofSize: 12.5), color: .secondaryLabelColor, lines: 0
        )
        let skip = secondaryButton(L("Skip for Now"), action: #selector(goDone))
        let next = primaryButton(L("Continue"), action: #selector(connectProvider))
        next.identifier = NSUserInterfaceItemIdentifier("continue")
        let layout = stepLayout(title: title, subtitle: subtitle, body: card(formGrid(providerRows())), back: skip, next: next)
        DispatchQueue.main.async { [weak self] in self?.renderCredential() }
        return layout
    }

    /// Provider picker plus the credential control for the chosen provider. Switching providers
    /// swaps only the credential row, so the page never re-renders.
    private func providerRows() -> [(String, NSView)] {
        let picker = NSPopUpButton()
        picker.addItems(withTitles: ProviderCredential.Kind.allCases.map(\.rawValue))
        picker.selectItem(at: ProviderCredential.Kind.allCases.firstIndex(of: providerKind) ?? 0)
        picker.target = self
        picker.action = #selector(providerPicked(_:))
        providerPicker = picker

        let host = NSView()
        host.translatesAutoresizingMaskIntoConstraints = false
        credentialHost = host

        let status = Build.label("", font: Theme.Font.caption, color: .tertiaryLabelColor, lines: 0)
        status.identifier = NSUserInterfaceItemIdentifier("status")

        return [(L("Provider"), picker), ("", host), ("", status)]
    }

    private weak var credentialHost: NSView?
    private weak var credentialLabel: NSTextField?
    private weak var providerPicker: NSPopUpButton?
    private weak var providerRow: NSGridRow?

    /// Fills the credential row for the current provider and sets the button to match.
    private func renderCredential() {
        guard let host = credentialHost else { return }
        for subview in host.subviews { subview.removeFromSuperview() }
        let usesCodex = step == .bot && harnessKind == .codex
        providerRow?.isHidden = usesCodex

        if usesCodex {
            if let bot = firstBot {
                let settings = CodexSettingsView(runnerID: bot.runnerID, botID: bot.id, selection: codexSelection)
                settings.onChange = { [weak self] in self?.codexSelection = $0 }
                host.addSubview(settings)
                settings.pin(to: host)
            }
            credentialLabel?.stringValue = ""
            setStatus("", color: .tertiaryLabelColor)
            findContinueButton()?.title = L("Continue")
            findContinueButton()?.isEnabled = true
            return
        }

        let control: NSView
        if providerKind.usesAPIKey {
            let keyField = NSSecureTextField()
            keyField.placeholderString = providerKind.keyPlaceholder
            keyField.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
            keyField.identifier = NSUserInterfaceItemIdentifier("apiKey")
            keyField.delegate = self
            control = keyField
        } else {
            control = Build.label(
                L("Your browser opens a %@ sign-in when you continue. %@", providerKind.rawValue, providerKind.signInRequirement),
                font: .systemFont(ofSize: 12), color: .secondaryLabelColor, lines: 0)
        }
        control.translatesAutoresizingMaskIntoConstraints = false
        host.addSubview(control)
        control.pin(to: host)

        credentialLabel?.stringValue = providerKind.usesAPIKey ? L("API key") : L("Account")
        setStatus(
            providerKind.usesAPIKey
                ? L("The key is checked against %@ and shared with your paired Devices, encrypted.", providerKind.rawValue)
                : L("Tokens from the sign-in are shared with your paired Devices, encrypted."),
            color: .tertiaryLabelColor)
        if let button = findContinueButton() {
            button.title = providerKind.usesAPIKey ? L("Continue") : L("Sign in with %@", providerKind.rawValue)
            button.isEnabled = !providerKind.usesAPIKey
        }
    }

    /// Label column on the left, controls on the right, every control the same width.
    private func formGrid(_ rows: [(String, NSView)]) -> NSView {
        let grid = NSGridView()
        grid.translatesAutoresizingMaskIntoConstraints = false
        grid.rowSpacing = 12
        grid.columnSpacing = 14
        grid.rowAlignment = .none
        for (title, control) in rows {
            control.translatesAutoresizingMaskIntoConstraints = false
            let label = Build.label(title, font: .systemFont(ofSize: 12, weight: .medium), color: .secondaryLabelColor)
            if control === credentialHost { credentialLabel = label }
            grid.addRow(with: [label, control])
            let row = grid.row(at: grid.numberOfRows - 1)
            if control === providerPicker { providerRow = row }
            row.yPlacement = control is WrappingTextField || (control is NSTextField && !(control as! NSTextField).isEditable) ? .top : .center
            if control is NSTextField || control is NSPopUpButton || control === credentialHost {
                control.widthAnchor.constraint(equalToConstant: 400).isActive = true
            } else if !(control is AvatarView) {
                control.widthAnchor.constraint(lessThanOrEqualToConstant: 400).isActive = true
            }
        }
        // Columns exist only once a row is added.
        grid.column(at: 0).xPlacement = .trailing
        grid.column(at: 0).width = 76
        return grid
    }

    private func card(_ content: NSView) -> NSView {
        let box = BackgroundView()
        box.cornerRadius = 12
        box.fillColor = Theme.codeBackground
        box.translatesAutoresizingMaskIntoConstraints = false
        box.addSubview(content)
        NSLayoutConstraint.activate([
            content.topAnchor.constraint(equalTo: box.topAnchor, constant: 18),
            content.leadingAnchor.constraint(equalTo: box.leadingAnchor, constant: 18),
            content.trailingAnchor.constraint(equalTo: box.trailingAnchor, constant: -18),
            content.bottomAnchor.constraint(equalTo: box.bottomAnchor, constant: -18),
        ])
        return box
    }

    private func doneView() -> NSView {
        let icon = NSImageView()
        icon.image = NSImage(systemSymbolName: "checkmark.circle.fill", accessibilityDescription: nil)
        icon.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 52, weight: .regular)
        icon.contentTintColor = .systemGreen
        icon.translatesAutoresizingMaskIntoConstraints = false

        let title = Build.label(
            L("This computer is your first Device"), font: .systemFont(ofSize: 22, weight: .semibold),
            alignment: .center)
        let subtitle = Build.label(
            firstBot.map { L("%@ is ready to talk to. Pair another Device any time from the File menu.", $0.name) }
                ?? L("Bots you create here run on this computer with your account's provider credentials. Pair another Device any time from the File menu."),
            font: .systemFont(ofSize: 12.5), color: .secondaryLabelColor, lines: 0, alignment: .center
        )

        let open = primaryButton(L("Open Lorca"), action: #selector(finish))

        let column = Build.stack([icon, title, subtitle, open], spacing: 14)
        column.alignment = .centerX
        column.setCustomSpacing(20, after: icon)
        column.setCustomSpacing(26, after: subtitle)

        let host = NSView()
        host.translatesAutoresizingMaskIntoConstraints = false
        host.addSubview(column)
        NSLayoutConstraint.activate([
            subtitle.widthAnchor.constraint(equalToConstant: 420),
            open.widthAnchor.constraint(equalToConstant: 200),
            column.centerXAnchor.constraint(equalTo: host.centerXAnchor),
            column.centerYAnchor.constraint(equalTo: host.centerYAnchor),
        ])
        return host
    }

    // MARK: - Pieces

    private func phraseGrid(words: [String]) -> NSView {
        let grid = NSGridView()
        grid.translatesAutoresizingMaskIntoConstraints = false
        grid.rowSpacing = 8
        grid.columnSpacing = 8

        let columns = 4
        var index = 0
        while index < words.count {
            var cells: [NSView] = []
            for column in 0..<columns {
                let position = index + column
                if position < words.count {
                    cells.append(phraseCell(index: position + 1, word: words[position]))
                } else {
                    let spacer = NSView()
                    spacer.translatesAutoresizingMaskIntoConstraints = false
                    cells.append(spacer)
                }
            }
            grid.addRow(with: cells)
            index += columns
        }
        return grid
    }

    private func phraseCell(index: Int, word: String) -> NSView {
        let box = BackgroundView()
        box.cornerRadius = 7
        box.fillColor = Theme.codeBackground

        let number = Build.label(
            "\(index)", font: .monospacedDigitSystemFont(ofSize: 10, weight: .regular),
            color: .tertiaryLabelColor)
        let value = Build.label(
            word, font: .monospacedSystemFont(ofSize: 13, weight: .medium), color: .labelColor)
        value.isSelectable = true

        box.addSubview(number)
        box.addSubview(value)
        NSLayoutConstraint.activate([
            box.widthAnchor.constraint(equalToConstant: 128),
            box.heightAnchor.constraint(equalToConstant: 34),
            number.leadingAnchor.constraint(equalTo: box.leadingAnchor, constant: 9),
            number.centerYAnchor.constraint(equalTo: box.centerYAnchor),
            number.widthAnchor.constraint(equalToConstant: 16),
            value.leadingAnchor.constraint(equalTo: number.trailingAnchor, constant: 2),
            value.centerYAnchor.constraint(equalTo: box.centerYAnchor),
        ])
        return box
    }

    private func stepLayout(
        title: NSView, subtitle: NSView, body: NSView, back: NSButton, next: NSButton
    ) -> NSView {
        let host = NSView()
        host.translatesAutoresizingMaskIntoConstraints = false

        let header = Build.stack([title, subtitle], spacing: 6)
        let buttons = Build.stack([back, next], orientation: .horizontal, spacing: 10)

        host.addSubview(header)
        host.addSubview(body)
        host.addSubview(buttons)

        body.translatesAutoresizingMaskIntoConstraints = false

        NSLayoutConstraint.activate([
            header.topAnchor.constraint(equalTo: host.topAnchor, constant: 12),
            header.leadingAnchor.constraint(equalTo: host.leadingAnchor),
            header.trailingAnchor.constraint(equalTo: host.trailingAnchor),

            // Wrapping labels need a width, or their intrinsic size keeps re-triggering layout.
            subtitle.widthAnchor.constraint(equalTo: host.widthAnchor),

            body.topAnchor.constraint(equalTo: header.bottomAnchor, constant: 26),
            body.leadingAnchor.constraint(equalTo: host.leadingAnchor),
            body.trailingAnchor.constraint(lessThanOrEqualTo: host.trailingAnchor),
            body.bottomAnchor.constraint(lessThanOrEqualTo: buttons.topAnchor, constant: -20),

            buttons.trailingAnchor.constraint(equalTo: host.trailingAnchor),
            buttons.bottomAnchor.constraint(equalTo: host.bottomAnchor),
            next.widthAnchor.constraint(greaterThanOrEqualToConstant: 120),
        ])
        return host
    }

    private func primaryButton(_ title: String, action: Selector) -> NSButton {
        let button = NSButton(title: title, target: self, action: action)
        button.bezelStyle = .rounded
        button.controlSize = .large
        button.keyEquivalent = "\r"
        button.translatesAutoresizingMaskIntoConstraints = false
        return button
    }

    private func secondaryButton(_ title: String, action: Selector) -> NSButton {
        let button = NSButton(title: title, target: self, action: action)
        button.bezelStyle = .rounded
        button.controlSize = .large
        button.translatesAutoresizingMaskIntoConstraints = false
        return button
    }

    // MARK: - Actions

    @objc private func goWelcome() { transition(to: .welcome) }
    @objc private func goRestore() { transition(to: .restore) }
    @objc private func goPair() { transition(to: .pair) }
    @objc private func goDone() { transition(to: .done) }
    @objc private func goBot() {
        harnessKind = firstBot?.harness ?? .lorca
        providerKind = firstBot?.provider ?? .deepseek
        codexSelection = .init(model: harnessKind == .codex ? firstBot?.model : nil,
                               thinking: harnessKind == .codex ? firstBot?.thinking : nil,
                               options: firstBot?.codexOptions ?? .init())
        transition(to: firstBot == nil ? .provider : .bot)
    }

    @objc private func harnessPicked(_ sender: NSPopUpButton) {
        harnessKind = Bot.Harness.allCases[max(0, sender.indexOfSelectedItem)]
        renderCredential()
    }

    @objc private func providerPicked(_ sender: NSPopUpButton) {
        providerKind = ProviderCredential.Kind.allCases[max(0, sender.indexOfSelectedItem)]
        renderCredential()
    }

    @objc private func saveFirstBot() {
        guard !busy, let bot = firstBot else {
            transition(to: .provider)
            return
        }
        let name = find(NSTextField.self, "botName")?.stringValue.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let description = find(NSTextField.self, "botDescription")?.stringValue.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !name.isEmpty else {
            setStatus(L("Give the bot a name."), color: .systemRed)
            return
        }
        // The bot runs with the provider chosen here, not the CLI's default.
        if name != bot.name || description != bot.description || providerKind != bot.provider {
            store.updateBot(bot.id, name: name, description: description, provider: providerKind)
        }
        store.setBotHarness(bot.id, harness: harnessKind)
        if harnessKind == .codex {
            store.setCodexOptions(bot.id, selection: codexSelection)
            transition(to: .done)
            return
        }
        connectProvider()
    }

    @objc private func connectProvider() {
        guard !busy else { return }
        if store.isMock {
            transition(to: .done)
            return
        }
        let key = find(NSTextField.self, "apiKey")?.stringValue.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if providerKind.usesAPIKey, key.isEmpty {
            setStatus(
                providerKind == .anthropic
                    ? L("Paste an %@ API key to continue.", providerKind.rawValue)
                    : L("Paste a %@ API key to continue.", providerKind.rawValue),
                color: .systemRed)
            return
        }
        busy = true
        setStatus(providerKind.usesAPIKey ? L("Checking the key with %@…", providerKind.rawValue) : L("Waiting for the browser…"), color: .secondaryLabelColor)
        Task { [weak self] in
            guard let self else { return }
            defer { self.busy = false }
            do {
                if self.providerKind.usesAPIKey {
                    try await self.store.connectAPIKey(self.providerKind, apiKey: key)
                } else {
                    try await self.store.connectSignIn(self.providerKind)
                }
                self.transition(to: .done)
            } catch {
                self.setStatus(error.localizedDescription, color: .systemRed)
            }
        }
    }

    @objc private func createIdentity() {
        guard !busy else { return }
        if store.isMock {
            phrase = MockData.backupPhrase
            transition(to: .create)
            return
        }
        guard store.isConnected else {
            presentError(
                L("The Lorca CLI is not running. Start it with `lorca serve` and try again.")
                    .replacingOccurrences(of: "lorca serve", with: AppInfo.cliCommand))
            return
        }
        busy = true
        Task { [weak self] in
            guard let self else { return }
            defer { self.busy = false }
            do {
                self.phrase = try await self.store.createIdentity()
                self.transition(to: .create)
            } catch {
                self.presentError(error.localizedDescription)
            }
        }
    }

    @objc private func restoreIdentity() {
        guard !busy, let field = find(NSTextField.self, "phrase") else { return }
        let text = field.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        if store.isMock {
            transition(to: .provider)
            return
        }
        busy = true
        setStatus(L("Re-deriving keys and unwrapping the account key…"), color: .secondaryLabelColor)
        Task { [weak self] in
            guard let self else { return }
            defer { self.busy = false }
            do {
                try await self.store.restoreIdentity(phrase: text)
                await self.continueAfterJoining()
            } catch {
                self.setStatus(error.localizedDescription, color: .systemRed)
            }
        }
    }

    /// A computer that joined an account gets its credentials with the first sync, so the provider
    /// step shows only when none arrive.
    private func continueAfterJoining() async {
        for _ in 0..<15 where !store.providers.contains(where: \.isConnected) {
            try? await Task.sleep(for: .milliseconds(200))
        }
        transition(to: store.providers.contains(where: \.isConnected) ? .done : .provider)
    }

    @objc private func acceptPairing() {
        guard !busy, let field = find(NSTextField.self, "pairing") else { return }
        let text = field.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        if store.isMock {
            transition(to: .provider)
            return
        }
        busy = true
        setStatus(L("Waiting for the other Device to wrap the account key…"), color: .secondaryLabelColor)
        Task { [weak self] in
            guard let self else { return }
            defer { self.busy = false }
            do {
                try await self.store.acceptPairing(text)
                await self.continueAfterJoining()
            } catch {
                self.setStatus(error.localizedDescription, color: .systemRed)
            }
        }
    }

    @objc private func finish() {
        pairingTask?.cancel()
        onFinish()
    }

    @objc private func copyPhrase(_ sender: CopyFeedbackButton) {
        NSPasteboard.general.clearContents()
        if NSPasteboard.general.setString(phrase.joined(separator: " "), forType: .string) {
            sender.showCopied()
        }
    }

    private func setStatus(_ text: String, color: NSColor) {
        guard let label = find(NSTextField.self, "status") else { return }
        label.stringValue = text
        label.textColor = color
    }

    private func presentError(_ message: String) {
        let alert = NSAlert()
        alert.messageText = AppInfo.name
        alert.informativeText = message
        alert.alertStyle = .warning
        alert.addButton(withTitle: L("OK"))
        if let window = view.window {
            alert.beginSheetModal(for: window)
        } else {
            alert.runModal()
        }
    }

    private func find<T: NSView>(_ type: T.Type, _ identifier: String) -> T? {
        func search(_ view: NSView) -> T? {
            if let match = view as? T, view.identifier == NSUserInterfaceItemIdentifier(identifier) {
                return match
            }
            for subview in view.subviews {
                if let found = search(subview) { return found }
            }
            return nil
        }
        return search(container)
    }

    @objc private func togglePhraseSaved(_ sender: NSButton) {
        savedPhrase = sender.state == .on
        findContinueButton()?.isEnabled = savedPhrase
    }

    private func findContinueButton() -> NSButton? {
        find(NSButton.self, "continue")
    }
}

extension OnboardingViewController: NSTextFieldDelegate {
    func controlTextDidChange(_ obj: Notification) {
        guard let field = obj.object as? NSTextField, field.identifier == NSUserInterfaceItemIdentifier("apiKey") else { return }
        findContinueButton()?.isEnabled = !field.stringValue.trimmingCharacters(in: .whitespaces).isEmpty
    }
}
