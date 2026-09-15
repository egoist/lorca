import AppKit

final class ChatViewController: NSViewController {
    private let store = AppStore.shared
    private let layout = ChatLayout()

    private let tableView = NSTableView()
    private let scrollView = NSScrollView()
    private let composer = ComposerView()
    private let emptyState = ChatEmptyStateView()
    private let jumpButton = NSButton()

    private var chatID: Chat.ID?
    private var rows: [ChatRow] = []
    private var expandedTools: Set<Message.ID> = []
    private var isPinnedToBottom = true
    private var lastWidth: CGFloat = 0

    // MARK: - Lifecycle

    override func loadView() {
        let container = BackgroundView()
        container.cornerRadius = 0
        container.fillColor = Theme.transcriptBackground

        configureTable()

        scrollView.documentView = tableView
        scrollView.hasVerticalScroller = true
        scrollView.drawsBackground = false
        scrollView.translatesAutoresizingMaskIntoConstraints = false
        scrollView.automaticallyAdjustsContentInsets = false
        scrollView.contentInsets = NSEdgeInsets(top: 8, left: 0, bottom: 14, right: 0)
        scrollView.contentView.postsBoundsChangedNotifications = true

        jumpButton.image = NSImage(
            systemSymbolName: "arrow.down", accessibilityDescription: "Scroll to latest")
        jumpButton.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 11, weight: .semibold)
        jumpButton.bezelStyle = .circular
        jumpButton.target = self
        jumpButton.action = #selector(scrollToLatest(_:))
        jumpButton.toolTip = "Scroll to latest (⌘J)"
        jumpButton.isHidden = true
        jumpButton.translatesAutoresizingMaskIntoConstraints = false

        composer.onSend = { [weak self] text in self?.send(text) }
        composer.onStop = { [weak self] in self?.stopResponding(nil) }

        emptyState.isHidden = true
        emptyState.onPick = { [weak self] prompt in
            self?.composer.text = prompt
            self?.composer.focus()
        }

        container.addSubview(scrollView)
        container.addSubview(emptyState)
        container.addSubview(jumpButton)
        container.addSubview(composer)

        NSLayoutConstraint.activate([
            scrollView.topAnchor.constraint(equalTo: container.topAnchor),
            scrollView.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            scrollView.bottomAnchor.constraint(equalTo: composer.topAnchor),

            emptyState.topAnchor.constraint(equalTo: container.topAnchor),
            emptyState.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            emptyState.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            emptyState.bottomAnchor.constraint(equalTo: composer.topAnchor),

            jumpButton.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -22),
            jumpButton.bottomAnchor.constraint(equalTo: composer.topAnchor, constant: -14),
            jumpButton.widthAnchor.constraint(equalToConstant: 28),
            jumpButton.heightAnchor.constraint(equalToConstant: 28),

            composer.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            composer.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            composer.bottomAnchor.constraint(equalTo: container.bottomAnchor),
        ])

        view = container
    }

    private func configureTable() {
        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("message"))
        column.resizingMask = .autoresizingMask
        tableView.addTableColumn(column)
        tableView.headerView = nil
        tableView.style = .plain
        tableView.backgroundColor = .clear
        tableView.selectionHighlightStyle = .none
        tableView.gridStyleMask = []
        tableView.intercellSpacing = .zero
        tableView.usesAutomaticRowHeights = false
        tableView.rowSizeStyle = .custom
        tableView.dataSource = self
        tableView.delegate = self
    }

    override func viewDidLoad() {
        super.viewDidLoad()

        store.observe(self) { [weak self] event in self?.handle(event) }

        NotificationCenter.default.addObserver(
            self,
            selector: #selector(scrollDidChange),
            name: NSView.boundsDidChangeNotification,
            object: scrollView.contentView
        )
    }

    override func viewDidLayout() {
        super.viewDidLayout()
        // The transcript runs under the titlebar, so its first row rests just below it rather than
        // blurred behind it.
        let topInset = view.safeAreaInsets.top + 8
        if scrollView.contentInsets.top != topInset {
            scrollView.contentInsets.top = topInset
            if isPinnedToBottom { scrollToBottom(animated: false) }
        }
        let width = tableView.bounds.width
        guard abs(width - lastWidth) > 0.5 else { return }
        lastWidth = width
        reloadHeights()
    }

    override func viewDidAppear() {
        super.viewDidAppear()
        // Take the cursor only when nothing else holds it. Arriving from the sidebar (a click
        // or arrow keys) leaves focus there, so its selection keeps the emphasized highlight.
        guard let window = view.window else { return }
        let responder = window.firstResponder
        let heldElsewhere = responder != nil && responder !== window
            && (responder as? NSView)?.isDescendant(of: view) != true
        if !heldElsewhere { composer.focus() }
    }

    // MARK: - Content

    func show(chatID newChatID: Chat.ID) {
        let isSameChat = chatID == newChatID
        chatID = newChatID
        guard let chat = store.chat(newChatID) else { return }

        layout.invalidateAll()
        expandedTools.removeAll()
        rebuildRows()
        tableView.reloadData()

        let members = store.bots(in: chat)
        composer.configure(placeholder: placeholder(for: chat), bots: members)
        composer.isResponding = store.isResponding(in: newChatID)

        emptyState.isHidden = !chat.messages.isEmpty
        emptyState.configure(chat: chat, bots: members)

        if !isSameChat { composer.text = "" }
        isPinnedToBottom = true
        DispatchQueue.main.async { [weak self] in self?.scrollToBottom(animated: false) }
    }

    func focusComposer() {
        composer.focus()
    }

    private func placeholder(for chat: Chat) -> String {
        let members = store.bots(in: chat)
        if chat.isDM, let only = members.first {
            return "Message \(only.name)"
        }
        let title = store.title(for: chat)
        return members.count > 1 ? "Message \(title) — @ to address one bot" : "Message \(title)"
    }

    private func rebuildRows() {
        rows.removeAll()
        guard let chatID, let chat = store.chat(chatID) else { return }

        var previousAuthor: Message.Author?
        var previousDate: Date?

        for message in chat.messages {
            if previousDate.map({ !Format.isSameDay($0, message.createdAt) }) ?? true {
                rows.append(.day(message.createdAt))
                previousAuthor = nil
            }

            let sameAuthor = previousAuthor == message.author
            let gap = previousDate.map { message.createdAt.timeIntervalSince($0) > 300 } ?? true
            let isChrome: Bool
            switch message.body {
            case .text: isChrome = false
            default: isChrome = true
            }

            rows.append(.message(id: message.id, groupStart: isChrome || !sameAuthor || gap))
            previousAuthor = isChrome ? nil : message.author
            previousDate = message.createdAt
        }
    }

    private func message(for id: Message.ID) -> Message? {
        guard let chatID, let chat = store.chat(chatID) else { return nil }
        return chat.messages.first { $0.id == id }
    }

    // MARK: - Store events

    private func handle(_ event: StoreEvent) {
        guard let chatID else { return }

        switch event {
        case let .messageAdded(id, _) where id == chatID:
            let wasPinned = isPinnedToBottom
            rebuildRows()
            tableView.reloadData()
            emptyState.isHidden = true
            composer.isResponding = store.isResponding(in: chatID)
            if wasPinned { scrollToBottom(animated: true) }

        case let .messageChanged(id, messageID) where id == chatID:
            layout.invalidate(messageID)
            updateRow(for: messageID)
            composer.isResponding = store.isResponding(in: chatID)

        case let .messageRemoved(id, messageID) where id == chatID:
            layout.invalidate(messageID)
            rebuildRows()
            tableView.reloadData()
            emptyState.isHidden = !(store.chat(chatID)?.messages.isEmpty ?? true)
            if isPinnedToBottom { scrollToBottom(animated: false) }

        case let .respondingChanged(id) where id == chatID:
            composer.isResponding = store.isResponding(in: chatID)

        case let .chatChanged(id) where id == chatID:
            guard let chat = store.chat(chatID) else { return }
            composer.configure(placeholder: placeholder(for: chat), bots: store.bots(in: chat))

        case .snapshotReplaced:
            show(chatID: chatID)

        default:
            break
        }
    }

    private func updateRow(for messageID: Message.ID) {
        guard let index = rows.firstIndex(where: { $0.messageID == messageID }) else { return }

        if let cell = tableView.view(atColumn: 0, row: index, makeIfNecessary: false) {
            configure(cell: cell, row: rows[index])
        }

        NSAnimationContext.runAnimationGroup { context in
            context.duration = 0
            tableView.noteHeightOfRows(withIndexesChanged: IndexSet(integer: index))
        }

        if isPinnedToBottom { scrollToBottom(animated: false) }
    }

    /// Bubble widths are baked in at configure time, so a resize has to re-measure
    /// the visible cells as well as their row heights.
    private func reloadHeights() {
        guard !rows.isEmpty else { return }

        let visible = tableView.rows(in: scrollView.contentView.bounds)
        if visible.length > 0 {
            for row in visible.lowerBound..<visible.upperBound where rows.indices.contains(row) {
                guard let cell = tableView.view(atColumn: 0, row: row, makeIfNecessary: false)
                else { continue }
                configure(cell: cell, row: rows[row])
            }
        }

        NSAnimationContext.runAnimationGroup { context in
            context.duration = 0
            tableView.noteHeightOfRows(withIndexesChanged: IndexSet(integersIn: 0..<rows.count))
        }
    }

    // MARK: - Scrolling

    @objc private func scrollDidChange() {
        let clip = scrollView.contentView
        let documentHeight = tableView.bounds.height
        let distance = documentHeight - (clip.bounds.origin.y + clip.bounds.height)
        isPinnedToBottom = distance < 48
        jumpButton.isHidden = isPinnedToBottom || rows.isEmpty
    }

    private func scrollToBottom(animated: Bool) {
        guard !rows.isEmpty else { return }
        let lastRow = rows.count - 1
        if animated {
            NSAnimationContext.runAnimationGroup { context in
                context.duration = 0.18
                context.allowsImplicitAnimation = true
                tableView.scrollRowToVisible(lastRow)
            }
        } else {
            tableView.scrollRowToVisible(lastRow)
        }
        isPinnedToBottom = true
        jumpButton.isHidden = true
    }

    // MARK: - Actions

    private func send(_ text: String) {
        guard let chatID else { return }
        isPinnedToBottom = true
        store.send(text, in: chatID)
        composer.isResponding = store.isResponding(in: chatID)
    }

    @objc func scrollToLatest(_ sender: Any?) {
        scrollToBottom(animated: true)
    }

    @objc func stopResponding(_ sender: Any?) {
        guard let chatID else { return }
        store.stopResponding(in: chatID)
        composer.isResponding = false
    }
}

// MARK: - Table

extension ChatViewController: NSTableViewDataSource, NSTableViewDelegate {
    func numberOfRows(in tableView: NSTableView) -> Int {
        rows.count
    }

    func tableView(_ tableView: NSTableView, heightOfRow row: Int) -> CGFloat {
        guard rows.indices.contains(row) else { return 1 }
        let chatRow = rows[row]
        let message = chatRow.messageID.flatMap(message(for:))
        let expanded = chatRow.messageID.map(expandedTools.contains) ?? false
        let isGroup = chatID.flatMap(store.chat)?.isGroup ?? false
        let showsName: Bool = {
            guard isGroup, let message, case .bot = message.author, case .text = message.body else {
                return false
            }
            return true
        }()
        return max(
            1,
            layout.height(
                for: chatRow,
                message: message,
                tableWidth: max(tableView.bounds.width, 320),
                expanded: expanded,
                showsName: showsName
            ))
    }

    func tableView(_ tableView: NSTableView, rowViewForRow row: Int) -> NSTableRowView? {
        let rowView = TransparentRowView()
        return rowView
    }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        guard rows.indices.contains(row) else { return nil }
        let chatRow = rows[row]

        switch chatRow {
        case .day:
            let cell = dequeue(DayCellView.identifier) { DayCellView() }
            configure(cell: cell, row: chatRow)
            return cell

        case let .message(id, _):
            guard let message = message(for: id) else { return nil }
            let identifier: NSUserInterfaceItemIdentifier
            let cell: NSView
            switch message.body {
            case .text:
                identifier = MessageCellView.identifier
                cell = dequeue(identifier) { MessageCellView() }
            case .tool:
                identifier = ToolCellView.identifier
                cell = dequeue(identifier) { ToolCellView() }
            case .handoff:
                identifier = HandoffCellView.identifier
                cell = dequeue(identifier) { HandoffCellView() }
            case .notice:
                identifier = NoticeCellView.identifier
                cell = dequeue(identifier) { NoticeCellView() }
            }
            configure(cell: cell, row: chatRow)
            return cell
        }
    }

    private func dequeue<T: NSView>(_ identifier: NSUserInterfaceItemIdentifier, make: () -> T) -> T {
        if let existing = tableView.makeView(withIdentifier: identifier, owner: self) as? T {
            return existing
        }
        let view = make()
        view.identifier = identifier
        return view
    }

    private func configure(cell: NSView, row: ChatRow) {
        switch row {
        case let .day(date):
            (cell as? DayCellView)?.configure(date)

        case let .message(id, groupStart):
            guard let message = message(for: id), let chatID, let chat = store.chat(chatID) else {
                return
            }

            switch message.body {
            case .text:
                guard let messageCell = cell as? MessageCellView else { return }
                let name: String
                let nameColor: NSColor
                if chat.isGroup, case let .bot(botID) = message.author {
                    name = store.bot(botID)?.name ?? "Bot"
                    nameColor = store.bot(botID)?.accent.color ?? .secondaryLabelColor
                } else {
                    name = ""
                    nameColor = .secondaryLabelColor
                }
                messageCell.configure(
                    message: message,
                    groupStart: groupStart,
                    authorName: name,
                    nameColor: nameColor,
                    avatarContent: AvatarView.content(for: message.author, store: store),
                    segments: layout.rendered(for: message).segments,
                    metrics: layout.metrics(
                        for: message,
                        authorName: name,
                        tableWidth: max(tableView.bounds.width, 320))
                )

            case let .tool(invocation):
                guard let toolCell = cell as? ToolCellView else { return }
                toolCell.configure(
                    invocation: invocation, groupStart: groupStart,
                    expanded: expandedTools.contains(id))
                toolCell.onToggle = { [weak self] in self?.toggleTool(id) }

            case let .handoff(from, to, reason):
                (cell as? HandoffCellView)?.configure(
                    from: store.bot(from), to: store.bot(to), reason: reason, groupStart: groupStart)

            case let .notice(text):
                (cell as? NoticeCellView)?.configure(
                    text: text,
                    groupStart: groupStart,
                    metrics: layout.noticeMetrics(
                        for: text, tableWidth: max(tableView.bounds.width, 320)))
            }

            _ = chat
        }
    }

    private func toggleTool(_ id: Message.ID) {
        if expandedTools.contains(id) {
            expandedTools.remove(id)
        } else {
            expandedTools.insert(id)
        }
        guard let index = rows.firstIndex(where: { $0.messageID == id }) else { return }
        if let cell = tableView.view(atColumn: 0, row: index, makeIfNecessary: false) {
            configure(cell: cell, row: rows[index])
        }
        tableView.noteHeightOfRows(withIndexesChanged: IndexSet(integer: index))
    }
}

final class TransparentRowView: NSTableRowView {
    override func drawBackground(in dirtyRect: NSRect) {}
    override func drawSelection(in dirtyRect: NSRect) {}
    override var isEmphasized: Bool {
        get { false }
        set {}
    }
}
