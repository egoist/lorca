import AppKit

final class ComputerViewController: NSViewController {
    private let store = AppStore.shared

    private let scrollView = NSScrollView()
    private let column = Build.stack([], spacing: 22)
    private let header = ComputerHeaderView()
    private let botsSection = SectionView(title: "Bots assigned here")
    private let providersSection = SectionView(title: "Provider credentials")
    private let machineSection = SectionView(title: "Machine")
    private let note = Build.label(
        "", font: Theme.Font.caption, color: .tertiaryLabelColor, lines: 0)

    private var computerID: Computer.ID?

    var onOpenChat: ((Chat.ID) -> Void)?

    override func loadView() {
        let container = BackgroundView()
        container.fillColor = Theme.transcriptBackground
        container.cornerRadius = 0

        column.orientation = .vertical
        column.alignment = .leading
        column.edgeInsets = NSEdgeInsets(top: 24, left: 28, bottom: 32, right: 28)
        column.addArrangedSubview(header)
        column.addArrangedSubview(botsSection)
        column.addArrangedSubview(providersSection)
        column.addArrangedSubview(machineSection)
        column.addArrangedSubview(note)

        let documentView = NSView()
        documentView.translatesAutoresizingMaskIntoConstraints = false
        documentView.addSubview(column)

        scrollView.documentView = documentView
        scrollView.drawsBackground = false
        scrollView.hasVerticalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.translatesAutoresizingMaskIntoConstraints = false

        container.addSubview(scrollView)
        scrollView.pin(to: container)

        NSLayoutConstraint.activate([
            documentView.widthAnchor.constraint(equalTo: scrollView.widthAnchor),
            column.topAnchor.constraint(equalTo: documentView.topAnchor),
            column.leadingAnchor.constraint(equalTo: documentView.leadingAnchor),
            column.trailingAnchor.constraint(equalTo: documentView.trailingAnchor),
            column.bottomAnchor.constraint(equalTo: documentView.bottomAnchor),
        ])

        for section in [header, botsSection, providersSection, machineSection, note] as [NSView] {
            section.widthAnchor.constraint(equalTo: column.widthAnchor, constant: -56).isActive = true
        }

        view = container
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        store.observe(self) { [weak self] event in
            switch event {
            case .snapshotReplaced, .chatsChanged:
                self?.reload()
            default:
                break
            }
        }
    }

    func show(computerID newID: Computer.ID) {
        computerID = newID
        guard isViewLoaded else { return }
        reload()
        scrollView.contentView.scroll(to: .zero)
    }

    override func viewWillAppear() {
        super.viewWillAppear()
        reload()
    }

    private func reload() {
        guard let computerID, let computer = store.computer(computerID) else { return }

        header.configure(computer: computer)

        let bots = store.bots(on: computer.id)
        if bots.isEmpty {
            let empty = KeyValueRow(key: "No bots assigned", value: "")
            botsSection.setRows([empty])
        } else {
            botsSection.setRows(
                bots.map { bot in
                    let row = BotRow()
                    row.configure(
                        bot: bot,
                        detailText: "\(bot.tagline) · \(bot.provider.rawValue)",
                        accessorySymbol: "bubble.left",
                        tooltip: "Open chat"
                    )
                    row.onAccessory = { [weak self] in self?.openChat(with: bot) }
                    row.onClick = { [weak self] in self?.openChat(with: bot) }
                    return row
                })
        }

        providersSection.setRows(
            computer.providers.map { credential in
                let row = StatusRow()
                row.configure(
                    symbol: credential.kind.symbolName,
                    title: credential.kind.rawValue,
                    subtitle: "\(credential.kind.subtitle) · \(credential.detail)",
                    state: credential.isConnected ? "Connected" : nil,
                    stateColor: .systemGreen,
                    actionTitle: credential.isConnected ? nil : "Connect…"
                )
                row.onAction = { [weak self] in self?.explainProviderSetup(on: computer) }
                return row
            })

        machineSection.setRows([
            KeyValueRow(key: "Machine key", value: computer.machineKey, monospaced: true),
            KeyValueRow(key: "System", value: computer.osVersion),
            KeyValueRow(
                key: "Last seen",
                value: computer.status == .online ? "Active now" : Format.lastSeen(computer.lastSeen)),
            KeyValueRow(key: "Relay", value: Preferences.relayURL, monospaced: true),
        ])

        note.stringValue =
            computer.isThisComputer
            ? "Keys for this Computer live in the CLI on this Mac. Connecting a provider here never leaves the machine."
            : "Provider credentials live on \(computer.name). Connect DeepSeek or ChatGPT from the Tinybot app running there — this Mac only sends encrypted job envelopes."
    }

    private func openChat(with bot: Bot) {
        onOpenChat?(store.dm(with: bot.id))
    }

    private func explainProviderSetup(on computer: Computer) {
        let alert = NSAlert()
        alert.messageText =
            computer.isThisComputer
            ? "Connect a provider on this Mac" : "Connect it on \(computer.name)"
        alert.informativeText =
            computer.isThisComputer
            ? "The CLI stores the key in the keychain on this Computer. Bots assigned here use it directly."
            : "Credentials never sync. Open Tinybot on \(computer.name) and connect the provider there; bots assigned to it pick it up on the next turn."
        alert.addButton(withTitle: "OK")
        if let window = view.window {
            alert.beginSheetModal(for: window)
        } else {
            alert.runModal()
        }
    }
}

final class ComputerHeaderView: NSView {
    private let icon = NSImageView()
    private let name = Build.label("", font: .systemFont(ofSize: 22, weight: .semibold))
    private let model = Build.label("", font: .systemFont(ofSize: 12.5), color: .secondaryLabelColor)
    private let dot = StatusDotView(size: 8)
    private let status = Build.label("", font: .systemFont(ofSize: 11.5, weight: .medium))

    init() {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        icon.translatesAutoresizingMaskIntoConstraints = false
        icon.contentTintColor = .secondaryLabelColor

        let statusLine = Build.stack([dot, status], orientation: .horizontal, spacing: 6)
        let text = Build.stack([name, model, statusLine], spacing: 3)
        text.setCustomSpacing(8, after: model)

        addSubview(icon)
        addSubview(text)

        NSLayoutConstraint.activate([
            icon.leadingAnchor.constraint(equalTo: leadingAnchor),
            icon.topAnchor.constraint(equalTo: topAnchor, constant: 4),
            icon.widthAnchor.constraint(equalToConstant: 44),
            icon.heightAnchor.constraint(equalToConstant: 44),
            text.leadingAnchor.constraint(equalTo: icon.trailingAnchor, constant: 14),
            text.topAnchor.constraint(equalTo: topAnchor),
            text.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor),
            text.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func configure(computer: Computer) {
        icon.image = NSImage(systemSymbolName: computer.symbolName, accessibilityDescription: nil)
        icon.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 34, weight: .regular)
        name.stringValue = computer.name
        model.stringValue = "\(computer.model) · \(computer.osVersion)"
        dot.status = computer.status

        switch computer.status {
        case .online:
            status.stringValue = computer.isThisComputer ? "This Mac · CLI running" : "Online · paired"
            status.textColor = .systemGreen
        case .pairing:
            status.stringValue = "Pairing…"
            status.textColor = .systemOrange
        case .offline:
            status.stringValue = Format.lastSeen(computer.lastSeen) + " · jobs wait on the relay"
            status.textColor = .secondaryLabelColor
        }
    }
}
