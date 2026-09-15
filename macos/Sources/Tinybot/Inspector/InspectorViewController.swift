import AppKit

final class InspectorViewController: NSViewController {
    private let store = AppStore.shared

    private let scrollView = NSScrollView()
    private let column = Build.stack([], spacing: 20)
    private let participants = SectionView(title: "Bots in this chat")
    private let routing = SectionView(title: "Where turns run")
    private let security = SectionView(title: "Encryption")
    private let addButton = NSButton()

    private var selection: Selection?

    var onOpenDevice: ((Device.ID) -> Void)?
    var onRemoveBot: ((Bot.ID) -> Void)?
    var onAddBot: (() -> Void)?

    override func loadView() {
        let container = NSView()

        column.orientation = .vertical
        column.alignment = .leading
        column.edgeInsets = NSEdgeInsets(top: 18, left: 16, bottom: 20, right: 16)

        addButton.title = "Add Bot…"
        addButton.bezelStyle = .rounded
        addButton.controlSize = .regular
        addButton.target = self
        addButton.action = #selector(addBot)
        addButton.translatesAutoresizingMaskIntoConstraints = false

        column.addArrangedSubview(participants)
        column.addArrangedSubview(addButton)
        column.addArrangedSubview(routing)
        column.addArrangedSubview(security)
        column.setCustomSpacing(10, after: participants)

        let documentView = NSView()
        documentView.translatesAutoresizingMaskIntoConstraints = false
        documentView.addSubview(column)

        scrollView.documentView = documentView
        scrollView.drawsBackground = false
        scrollView.hasVerticalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.translatesAutoresizingMaskIntoConstraints = false
        // Kept below the header rather than under it: a scroll view that runs under the glass
        // header gets AppKit's scroll pocket, which draws a hard edge line whenever the pointer
        // rests on the header.
        scrollView.automaticallyAdjustsContentInsets = false

        container.addSubview(scrollView)

        NSLayoutConstraint.activate([
            scrollView.topAnchor.constraint(equalTo: container.safeAreaLayoutGuide.topAnchor),
            scrollView.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            scrollView.bottomAnchor.constraint(equalTo: container.bottomAnchor),
            documentView.widthAnchor.constraint(equalTo: scrollView.widthAnchor),
            column.topAnchor.constraint(equalTo: documentView.topAnchor),
            column.leadingAnchor.constraint(equalTo: documentView.leadingAnchor),
            column.trailingAnchor.constraint(equalTo: documentView.trailingAnchor),
            column.bottomAnchor.constraint(equalTo: documentView.bottomAnchor),
            participants.widthAnchor.constraint(equalTo: column.widthAnchor, constant: -32),
            routing.widthAnchor.constraint(equalTo: column.widthAnchor, constant: -32),
            security.widthAnchor.constraint(equalTo: column.widthAnchor, constant: -32),
        ])

        view = container
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        store.observe(self) { [weak self] event in
            switch event {
            case .chatChanged, .chatsChanged, .snapshotReplaced:
                self?.reload()
            default:
                break
            }
        }
    }

    func show(selection newSelection: Selection) {
        selection = newSelection
        reload()
    }

    func reload() {
        guard isViewLoaded, case let .chat(chatID) = selection, let chat = store.chat(chatID) else {
            return
        }

        let members = store.bots(in: chat)

        participants.setRows(
            members.map { bot in
                let host = store.device(bot.runnerID)
                let row = BotRow()
                row.configure(
                    bot: bot,
                    detailText: "\(bot.provider.rawValue) · \(host?.name ?? "unassigned")",
                    accessorySymbol: chat.canRemoveBot ? "minus.circle" : nil,
                    tooltip: "Remove from chat"
                )
                row.onAccessory = { [weak self] in self?.onRemoveBot?(bot.id) }
                return row
            })

        // A DM never takes another bot; a group does until it is full or every bot is in it.
        addButton.isHidden = chat.isDM
        addButton.isEnabled = chat.canAddBot && members.count < store.bots.count

        let hosts = Dictionary(grouping: members, by: \.runnerID)
        routing.setRows(
            hosts.keys.sorted().compactMap { runnerID in
                guard let runner = store.device(runnerID) else { return nil }
                let botNames = (hosts[runnerID] ?? []).map(\.name).joined(separator: ", ")
                let row = StatusRow()
                row.configure(
                    symbol: runner.symbolName,
                    title: runner.name,
                    subtitle: botNames,
                    state: runner.status == .online ? "Online" : Format.lastSeen(runner.lastSeen),
                    stateColor: runner.status == .online ? .systemGreen : .secondaryLabelColor
                )
                let click = NSClickGestureRecognizer(target: self, action: #selector(openDevice(_:)))
                row.addGestureRecognizer(click)
                row.identifier = NSUserInterfaceItemIdentifier(runner.id)
                return row
            })

        security.setRows([
            KeyValueRow(key: "Transcript", value: "Encrypted on device"),
            KeyValueRow(key: "Relay sees", value: "Ciphertext only", tint: .systemGreen),
            KeyValueRow(key: "Chat blob", value: "chat · seq \(chat.messages.count)", monospaced: true),
        ])
    }

    @objc private func addBot() {
        onAddBot?()
    }

    @objc private func openDevice(_ sender: NSClickGestureRecognizer) {
        guard let id = sender.view?.identifier?.rawValue else { return }
        onOpenDevice?(id)
    }
}
