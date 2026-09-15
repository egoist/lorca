import AppKit

final class OnboardingWindowController: NSWindowController {
    init(onFinish: @escaping () -> Void) {
        let controller = OnboardingViewController(onFinish: onFinish)
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
        case done
    }

    private let onFinish: () -> Void
    private let container = NSView()
    private var step: Step = .welcome
    private var savedPhrase = false
    private var isPaired = false
    private var pairingTask: Task<Void, Never>?

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

        let create = primaryButton("Create a New Identity", action: #selector(goCreate))
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

        let grid = phraseGrid()

        let copy = NSButton(title: "Copy Phrase", target: self, action: #selector(copyPhrase))
        copy.bezelStyle = .rounded

        let confirm = NSButton(
            checkboxWithTitle: "I wrote this phrase down somewhere safe", target: self,
            action: #selector(togglePhraseSaved))
        confirm.state = savedPhrase ? .on : .off
        confirm.translatesAutoresizingMaskIntoConstraints = false

        let back = secondaryButton("Back", action: #selector(goWelcome))
        let next = primaryButton("Continue", action: #selector(goDone))
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

        let hint = Build.label(
            "Restoring never contacts a login server. The relay only answers a signature challenge.",
            font: Theme.Font.caption, color: .tertiaryLabelColor, lines: 0)
        hint.widthAnchor.constraint(equalToConstant: 560).isActive = true

        let back = secondaryButton("Back", action: #selector(goWelcome))
        let next = primaryButton("Restore", action: #selector(goDone))

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
            "On the Mac that already has your identity, choose File › Pair a Device and scan this code.",
            font: .systemFont(ofSize: 12.5), color: .secondaryLabelColor, lines: 0
        )

        let frame = BackgroundView()
        frame.cornerRadius = 12
        frame.fillColor = .white
        let qr = NSImageView()
        qr.image = QRCode.image(for: MockData.pairingString(), size: 160)
        qr.translatesAutoresizingMaskIntoConstraints = false
        frame.addSubview(qr)

        let status = Build.label(
            isPaired ? "Paired. Account key unwrapped." : "Waiting for the other Device…",
            font: .systemFont(ofSize: 12),
            color: isPaired ? .systemGreen : .secondaryLabelColor)

        let back = secondaryButton("Back", action: #selector(goWelcome))
        let next = primaryButton("Continue", action: #selector(goDone))
        next.isEnabled = isPaired

        let body = Build.stack([frame, status], spacing: 14)
        body.alignment = .centerX

        NSLayoutConstraint.activate([
            qr.widthAnchor.constraint(equalToConstant: 160),
            qr.heightAnchor.constraint(equalToConstant: 160),
            qr.topAnchor.constraint(equalTo: frame.topAnchor, constant: 10),
            qr.leadingAnchor.constraint(equalTo: frame.leadingAnchor, constant: 10),
            qr.trailingAnchor.constraint(equalTo: frame.trailingAnchor, constant: -10),
            qr.bottomAnchor.constraint(equalTo: frame.bottomAnchor, constant: -10),
        ])

        if !isPaired {
            pairingTask?.cancel()
            pairingTask = Task { [weak self] in
                try? await Task.sleep(nanoseconds: 3_200_000_000)
                guard let self, !Task.isCancelled, self.step == .pair else { return }
                self.isPaired = true
                self.transition(to: .pair)
            }
        }

        return stepLayout(title: title, subtitle: subtitle, body: body, back: back, next: next)
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
            Bots you create here run on this machine with its own provider credentials. \
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

    private func phraseGrid() -> NSView {
        let grid = NSGridView()
        grid.translatesAutoresizingMaskIntoConstraints = false
        grid.rowSpacing = 8
        grid.columnSpacing = 8

        let words = MockData.backupPhrase
        for rowIndex in 0..<3 {
            var cells: [NSView] = []
            for columnIndex in 0..<4 {
                let index = rowIndex * 4 + columnIndex
                cells.append(phraseCell(index: index + 1, word: words[index]))
            }
            grid.addRow(with: cells)
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
    @objc private func goCreate() { transition(to: .create) }
    @objc private func goRestore() { transition(to: .restore) }
    @objc private func goPair() { transition(to: .pair) }
    @objc private func goDone() { transition(to: .done) }

    @objc private func finish() {
        pairingTask?.cancel()
        onFinish()
    }

    @objc private func copyPhrase() {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(MockData.backupPhrase.joined(separator: " "), forType: .string)
    }

    @objc private func togglePhraseSaved(_ sender: NSButton) {
        savedPhrase = sender.state == .on
        findContinueButton()?.isEnabled = savedPhrase
    }

    private func findContinueButton() -> NSButton? {
        func search(_ view: NSView) -> NSButton? {
            if let button = view as? NSButton,
                button.identifier == NSUserInterfaceItemIdentifier("continue")
            {
                return button
            }
            for subview in view.subviews {
                if let found = search(subview) { return found }
            }
            return nil
        }
        return search(container)
    }
}
