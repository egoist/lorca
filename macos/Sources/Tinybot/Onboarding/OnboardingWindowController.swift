import AppKit

final class OnboardingWindowController: NSWindowController {
    private let controller: OnboardingViewController

    /// True once the identity exists and the user is on the closing step, so an
    /// `identity.changed` event does not yank the window away mid-flow.
    var isOnFinalStep: Bool { controller.isOnFinalStep }

    init(onFinish: @escaping () -> Void) {
        let controller = OnboardingViewController(onFinish: onFinish)
        self.controller = controller
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 660, height: 520),
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
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }
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
            root.heightAnchor.constraint(equalToConstant: 520),
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
            "Tinybot", font: .systemFont(ofSize: 30, weight: .bold), alignment: .center)
        let subtitle = Build.label(
            """
            Bots that run on Macs you own. Your identity is a key pair on this machine — \
            no account, no server that can read your chats.
            """,
            font: .systemFont(ofSize: 13), color: .secondaryLabelColor, lines: 0, alignment: .center
        )

        let create = primaryButton("Create a New Identity", action: #selector(createIdentity))
        let restore = secondaryButton("Restore from Backup Phrase", action: #selector(goRestore))
        let pair = secondaryButton("Pair with Another Device", action: #selector(goPair))

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
        let title = Build.label("Your backup phrase", font: .systemFont(ofSize: 22, weight: .semibold))
        let subtitle = Build.label(
            """
            This phrase is your master secret. It re-derives every key and unwraps everything on the \
            relay. Write it down — nobody can reset it for you.
            """,
            font: .systemFont(ofSize: 12.5), color: .secondaryLabelColor, lines: 0
        )

        let grid = phraseGrid(words: phrase)

        let copy = NSButton(title: "Copy Phrase", target: self, action: #selector(copyPhrase))
        copy.bezelStyle = .rounded

        let confirm = NSButton(
            checkboxWithTitle: "I wrote this phrase down somewhere safe", target: self,
            action: #selector(togglePhraseSaved))
        confirm.state = savedPhrase ? .on : .off
        confirm.translatesAutoresizingMaskIntoConstraints = false

        // The identity already exists on this Mac; there is no way back from here.
        let back = secondaryButton("Back", action: #selector(goWelcome))
        back.isHidden = true
        let next = primaryButton("Continue", action: #selector(goBot))
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
        let title = Build.label("Restore your identity", font: .systemFont(ofSize: 22, weight: .semibold))
        let subtitle = Build.label(
            "Paste the twelve groups from your backup phrase. Everything else is re-derived and unwrapped from the relay.",
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
                ? "Restoring unwraps the account key from the relay. Set a relay URL in Settings › Advanced first."
                : "Restoring never contacts a login server. The relay only answers a signature challenge.",
            font: Theme.Font.caption, color: .tertiaryLabelColor, lines: 0)
        hint.widthAnchor.constraint(equalToConstant: 560).isActive = true
        hint.identifier = NSUserInterfaceItemIdentifier("status")

        let back = secondaryButton("Back", action: #selector(goWelcome))
        let next = primaryButton("Restore", action: #selector(restoreIdentity))

        return stepLayout(
            title: title,
            subtitle: subtitle,
            body: Build.stack([field, hint], spacing: 10),
            back: back,
            next: next
        )
    }

    private func pairView() -> NSView {
        let title = Build.label("Pair with another Mac", font: .systemFont(ofSize: 22, weight: .semibold))
        let subtitle = Build.label(
            "On the Mac that already has your identity, choose File › Pair a Device and paste the code it shows here.",
            font: .systemFont(ofSize: 12.5), color: .secondaryLabelColor, lines: 0
        )

        let field = NSTextField()
        field.placeholderString = "tinybot://pair?relay=…"
        field.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
        field.translatesAutoresizingMaskIntoConstraints = false
        field.heightAnchor.constraint(equalToConstant: 26).isActive = true
        field.widthAnchor.constraint(equalToConstant: 560).isActive = true
        field.identifier = NSUserInterfaceItemIdentifier("pairing")

        let status = Build.label(
            "The two Devices run a handshake; the relay only carries the ciphertext. This Mac joins as a Runner.",
            font: Theme.Font.caption, color: .tertiaryLabelColor, lines: 0)
        status.widthAnchor.constraint(equalToConstant: 560).isActive = true
        status.identifier = NSUserInterfaceItemIdentifier("status")

        let back = secondaryButton("Back", action: #selector(goWelcome))
        let next = primaryButton("Pair", action: #selector(acceptPairing))

        return stepLayout(
            title: title, subtitle: subtitle, body: Build.stack([field, status], spacing: 10), back: back,
            next: next)
    }

    /// The bot the CLI created with the identity, or the first bot in the roster.
    private var firstBot: Bot? {
        store.bots.first { $0.name == "Chef" } ?? store.bots.first
    }

    private func botView() -> NSView {
        let title = Build.label("Your first bot", font: .systemFont(ofSize: 22, weight: .semibold))
        let subtitle = Build.label(
            "It runs on this Mac, plans your work, and builds the rest of the team when you ask. Give it a name and the credentials it runs with.",
            font: .systemFont(ofSize: 12.5), color: .secondaryLabelColor, lines: 0
        )

        let avatar = AvatarView(diameter: 40)
        avatar.content = .bot(symbolName: firstBot?.symbolName ?? "sparkles", accent: firstBot?.accent ?? .indigo)

        let nameField = NSTextField()
        nameField.stringValue = firstBot?.name ?? "Chef"
        nameField.placeholderString = "Name"
        nameField.identifier = NSUserInterfaceItemIdentifier("botName")
        let taglineField = NSTextField()
        taglineField.stringValue = firstBot?.tagline ?? "Chief of staff · plans, delegates, builds the team"
        taglineField.placeholderString = "What it is good at"
        taglineField.identifier = NSUserInterfaceItemIdentifier("botTagline")

        let grid = formGrid([
            ("", avatar),
            ("Name", nameField),
            ("Tagline", taglineField),
        ] + providerRows())

        let back = secondaryButton("Back", action: #selector(goWelcome))
        back.isHidden = true
        let next = primaryButton("Continue", action: #selector(saveFirstBot))
        next.identifier = NSUserInterfaceItemIdentifier("continue")
        let layout = stepLayout(title: title, subtitle: subtitle, body: card(grid), back: back, next: next)
        DispatchQueue.main.async { [weak self] in self?.renderCredential() }
        return layout
    }

    private func providerView() -> NSView {
        let title = Build.label("Connect a provider", font: .systemFont(ofSize: 22, weight: .semibold))
        let subtitle = Build.label(
            "Bots assigned to this Mac run with credentials stored here, in the CLI. Nothing is sent to a Tinybot server.",
            font: .systemFont(ofSize: 12.5), color: .secondaryLabelColor, lines: 0
        )
        let skip = secondaryButton("Skip for Now", action: #selector(goDone))
        let next = primaryButton("Continue", action: #selector(connectProvider))
        next.identifier = NSUserInterfaceItemIdentifier("continue")
        let layout = stepLayout(title: title, subtitle: subtitle, body: card(formGrid(providerRows())), back: skip, next: next)
        DispatchQueue.main.async { [weak self] in self?.renderCredential() }
        return layout
    }

    /// Provider picker plus the credential control for the chosen provider. Switching providers
    /// swaps only the credential row, so the page never re-renders.
    private func providerRows() -> [(String, NSView)] {
        let picker = NSSegmentedControl(
            labels: ProviderCredential.Kind.allCases.map { "\($0.rawValue) · \($0.subtitle)" }, trackingMode: .selectOne,
            target: self, action: #selector(providerPicked(_:)))
        picker.selectedSegment = ProviderCredential.Kind.allCases.firstIndex(of: providerKind) ?? 0
        picker.segmentStyle = .rounded

        let host = NSView()
        host.translatesAutoresizingMaskIntoConstraints = false
        credentialHost = host

        let status = Build.label("", font: Theme.Font.caption, color: .tertiaryLabelColor, lines: 0)
        status.identifier = NSUserInterfaceItemIdentifier("status")

        return [("Provider", picker), ("", host), ("", status)]
    }

    private weak var credentialHost: NSView?
    private weak var credentialLabel: NSTextField?

    /// Fills the credential row for the current provider and sets the button to match.
    private func renderCredential() {
        guard let host = credentialHost else { return }
        for subview in host.subviews { subview.removeFromSuperview() }

        let control: NSView
        switch providerKind {
        case .deepseek:
            let keyField = NSSecureTextField()
            keyField.placeholderString = "sk-… from platform.deepseek.com"
            keyField.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
            keyField.identifier = NSUserInterfaceItemIdentifier("apiKey")
            keyField.delegate = self
            control = keyField
        case .chatgpt:
            control = Build.label(
                "Your browser opens a ChatGPT sign-in when you continue. Needs a ChatGPT subscription.",
                font: .systemFont(ofSize: 12), color: .secondaryLabelColor, lines: 0)
        }
        control.translatesAutoresizingMaskIntoConstraints = false
        host.addSubview(control)
        control.pin(to: host)

        credentialLabel?.stringValue = providerKind == .deepseek ? "API key" : "Account"
        setStatus(
            providerKind == .deepseek
                ? "The key is checked against DeepSeek and kept in the CLI's credential file on this Mac."
                : "Tokens from the sign-in stay in the CLI's credential file on this Mac.",
            color: .tertiaryLabelColor)
        if let button = findContinueButton() {
            button.title = providerKind == .deepseek ? "Continue" : "Sign in with ChatGPT"
            button.isEnabled = providerKind == .chatgpt
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
            row.yPlacement = control is NSTextField && !(control as! NSTextField).isEditable ? .top : .center
            if control is NSTextField || control is NSSegmentedControl || control === credentialHost {
                control.widthAnchor.constraint(equalToConstant: 400).isActive = true
            } else if !(control is AvatarView) {
                control.widthAnchor.constraint(lessThanOrEqualToConstant: 400).isActive = true
            }
        }
        // Columns exist only once a row is added.
        grid.column(at: 0).xPlacement = .trailing
        grid.column(at: 0).width = 64
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
            "This Mac is your first Device", font: .systemFont(ofSize: 22, weight: .semibold),
            alignment: .center)
        let subtitle = Build.label(
            """
            \(firstBot.map { "\($0.name) is ready to talk to." } ?? "Bots you create here run on this machine with its own provider credentials.") \
            Pair another Mac any time from the File menu.
            """,
            font: .systemFont(ofSize: 12.5), color: .secondaryLabelColor, lines: 0, alignment: .center
        )

        let open = primaryButton("Open Tinybot", action: #selector(finish))

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
    @objc private func goBot() { transition(to: firstBot == nil ? .provider : .bot) }

    @objc private func providerPicked(_ sender: NSSegmentedControl) {
        providerKind = ProviderCredential.Kind.allCases[max(0, sender.selectedSegment)]
        renderCredential()
    }

    @objc private func saveFirstBot() {
        guard !busy, let bot = firstBot else {
            transition(to: .provider)
            return
        }
        let name = find(NSTextField.self, "botName")?.stringValue.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let tagline = find(NSTextField.self, "botTagline")?.stringValue.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !name.isEmpty else {
            setStatus("Give the bot a name.", color: .systemRed)
            return
        }
        // The bot runs with the provider chosen here, not the CLI's default.
        if name != bot.name || tagline != bot.tagline || providerKind != bot.provider {
            store.updateBot(bot.id, name: name, tagline: tagline.isEmpty ? bot.tagline : tagline, provider: providerKind)
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
        if providerKind == .deepseek, key.isEmpty {
            setStatus("Paste a DeepSeek API key to continue.", color: .systemRed)
            return
        }
        busy = true
        setStatus(providerKind == .deepseek ? "Checking the key with DeepSeek…" : "Waiting for the browser…", color: .secondaryLabelColor)
        Task { [weak self] in
            guard let self else { return }
            defer { self.busy = false }
            do {
                switch self.providerKind {
                case .deepseek: try await self.store.connectDeepSeek(apiKey: key)
                case .chatgpt: try await self.store.connectChatGPT()
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
            presentError("The Tinybot CLI is not running. Start it with `tinybot serve` and try again.")
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
        setStatus("Re-deriving keys and unwrapping the account key…", color: .secondaryLabelColor)
        Task { [weak self] in
            guard let self else { return }
            defer { self.busy = false }
            do {
                try await self.store.restoreIdentity(phrase: text)
                self.transition(to: .provider)
            } catch {
                self.setStatus(error.localizedDescription, color: .systemRed)
            }
        }
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
        setStatus("Waiting for the other Device to wrap the account key…", color: .secondaryLabelColor)
        Task { [weak self] in
            guard let self else { return }
            defer { self.busy = false }
            do {
                try await self.store.acceptPairing(text)
                self.transition(to: .provider)
            } catch {
                self.setStatus(error.localizedDescription, color: .systemRed)
            }
        }
    }

    @objc private func finish() {
        pairingTask?.cancel()
        onFinish()
    }

    @objc private func copyPhrase() {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(phrase.joined(separator: " "), forType: .string)
    }

    private func setStatus(_ text: String, color: NSColor) {
        guard let label = find(NSTextField.self, "status") else { return }
        label.stringValue = text
        label.textColor = color
    }

    private func presentError(_ message: String) {
        let alert = NSAlert()
        alert.messageText = "Tinybot"
        alert.informativeText = message
        alert.alertStyle = .warning
        alert.addButton(withTitle: "OK")
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
