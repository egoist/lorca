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
    /// Shown after the last message when a turn ended without a reply; cleared by the next message.
    private var stoppedNotice: String?
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
            systemSymbolName: "arrow.down", accessibilityDescription: L("Scroll to latest"))
        jumpButton.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 11, weight: .semibold)
        // A glass disc on macOS 26+, like the composer it floats above.
        if #available(macOS 26, *) {
            jumpButton.bezelStyle = .glass
            jumpButton.borderShape = .circle
        } else {
            jumpButton.bezelStyle = .circular
        }
        jumpButton.target = self
        jumpButton.action = #selector(scrollToLatest(_:))
        jumpButton.toolTip = L("Scroll to latest (⌘J)")
        jumpButton.isHidden = true
        jumpButton.translatesAutoresizingMaskIntoConstraints = false

        composer.onSend = { [weak self] text, attachments in self?.send(text, attachments: attachments) }
        composer.onStop = { [weak self] in self?.stopResponding(nil) }
        // The composer floats over the transcript, as on the phone: the scroll view runs to
        // the bottom of the window and keeps an inset the height of the composer, so the last
        // message clears it and the messages show through the glass while they scroll under.
        composer.postsFrameChangedNotifications = true
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(composerFrameDidChange),
            name: NSView.frameDidChangeNotification,
            object: composer
        )

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
            scrollView.bottomAnchor.constraint(equalTo: container.bottomAnchor),

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
        // Re-measured heights move the end of the document; a pinned transcript follows it.
        if isPinnedToBottom { scrollToBottom(animated: false) }
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

        if !isSameChat { stoppedNotice = nil }
        layout.invalidateAll()
        rebuildRows()
        tableView.reloadData()

        let members = store.bots(in: chat)
        composer.configure(placeholder: placeholder(for: chat), bots: mentionable(in: chat))
        composer.isResponding = store.isResponding(in: newChatID)

        emptyState.isHidden = !chat.messages.isEmpty
        emptyState.configure(chat: chat, bots: members)

        if !isSameChat { composer.text = "" }
        isPinnedToBottom = true
        DispatchQueue.main.async { [weak self] in self?.scrollToBottom(animated: false) }
    }

    /// Puts text in the composer for the user to finish, as the inspector's "Edit in chat" does.
    func prefill(_ text: String) {
        composer.text = text
        composer.focus()
    }

    func focusComposer() {
        composer.focus()
    }

    /// Who `@` can address: members first, then every other bot. In a group an outsider joins
    /// when the message is sent; in a DM the message moves to a new group with both bots.
    private func mentionable(in chat: Chat) -> [Bot] {
        let members = store.bots(in: chat)
        return members + store.bots.filter { bot in !members.contains { $0.id == bot.id } }
    }

    private func placeholder(for chat: Chat) -> String {
        let members = store.bots(in: chat)
        if chat.isDM, let only = members.first {
            return L("Message %@", only.name)
        }
        let title = store.title(for: chat)
        return members.count > 1 ? L("Message %@ — @ to address one bot", title) : L("Message %@", title)
    }

    private func rebuildRows() {
        rows.removeAll()
        guard let chatID, let chat = store.chat(chatID) else { return }

        var previousAuthor: Message.Author?
        var previousDate: Date?

        for message in chat.messages {
            // Tool calls are the bot's business; only a sent message leaves a marker.
            if case let .tool(tool) = message.body, !tool.isSentMessage { continue }
            let silence = previousDate.map { message.createdAt.timeIntervalSince($0) } ?? .infinity
            if silence > ChatMetrics.separatorGap
                || previousDate.map({ !Format.isSameDay($0, message.createdAt) }) ?? true
            {
                rows.append(.day(message.createdAt))
                previousAuthor = nil
            }

            let sameAuthor = previousAuthor == message.author
            let isChrome: Bool
            switch message.body {
            case .text: isChrome = false
            default: isChrome = true
            }

            rows.append(.message(id: message.id, groupStart: isChrome || !sameAuthor))
            previousAuthor = isChrome ? nil : message.author
            previousDate = message.createdAt
        }

        let working = store.workingBots(in: chat.id)
        if !working.isEmpty {
            rows.append(.working(working))
        } else if let stoppedNotice {
            rows.append(.status(stoppedNotice))
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
            stoppedNotice = nil
            rebuildRows()
            tableView.reloadData()
            emptyState.isHidden = true
            composer.isResponding = store.isResponding(in: chatID)
            if wasPinned { scrollToBottom(animated: true) }

        case let .messageChanged(id, messageID) where id == chatID:
            layout.invalidate(messageID)
            updateRow(for: messageID)
            if let index = rows.firstIndex(where: { if case .working = $0 { return true } else { return false } }),
                let cell = tableView.view(atColumn: 0, row: index, makeIfNecessary: false)
            {
                configure(cell: cell, row: rows[index])
            }
            composer.isResponding = store.isResponding(in: chatID)

        case let .messageRemoved(id, messageID) where id == chatID:
            layout.invalidate(messageID)
            rebuildRows()
            tableView.reloadData()
            emptyState.isHidden = !(store.chat(chatID)?.messages.isEmpty ?? true)
            if isPinnedToBottom { scrollToBottom(animated: false) }

        case let .respondingChanged(id) where id == chatID:
            composer.isResponding = store.isResponding(in: chatID)
            if !store.isResponding(in: chatID), let chat = store.chat(chatID),
                case .you = chat.messages.last?.author
            {
                stoppedNotice = L("%@ stopped without replying", store.title(for: chat))
            }
            let wasPinned = isPinnedToBottom
            rebuildRows()
            tableView.reloadData()
            // Turns hand over quickly (one member finishes as the next starts), so the pin
            // is re-applied once the table has laid out the new last row.
            if wasPinned {
                scrollToBottom(animated: false)
                DispatchQueue.main.async { [weak self] in
                    guard let self, self.isPinnedToBottom else { return }
                    self.scrollToBottom(animated: false)
                }
            }

        case let .chatChanged(id) where id == chatID:
            guard let chat = store.chat(chatID) else { return }
            composer.configure(placeholder: placeholder(for: chat), bots: mentionable(in: chat))

        case let .olderMessagesLoaded(id) where id == chatID:
            olderMessagesLoaded()

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

    /// Keeps the bottom inset the height of the floating composer, which grows with its text
    /// and attachments; a pinned transcript stays pinned as the inset changes.
    @objc private func composerFrameDidChange() {
        let bottom = composer.frame.height
        guard abs(scrollView.contentInsets.bottom - bottom) > 0.5 else { return }
        scrollView.contentInsets.bottom = bottom
        if isPinnedToBottom { scrollToBottom(animated: false) }
    }

    @objc private func scrollDidChange() {
        let clip = scrollView.contentView
        let documentHeight = tableView.bounds.height
        let distance = documentHeight - (clip.bounds.origin.y + clip.bounds.height)
        isPinnedToBottom = distance < 48
        jumpButton.isHidden = isPinnedToBottom || rows.isEmpty
        // Nearing the first message: ask for the page before it.
        if let chatID, clip.bounds.origin.y + scrollView.contentInsets.top < 600 {
            store.loadOlderMessages(in: chatID)
        }
    }

    /// Older messages went in above the first row: keep what is on screen where it is by
    /// holding the distance to the end of the document.
    private func olderMessagesLoaded() {
        let clip = scrollView.contentView
        let fromEnd = (rows.isEmpty ? 0 : tableView.rect(ofRow: rows.count - 1).maxY) - clip.bounds.origin.y
        rebuildRows()
        tableView.reloadData()
        guard !rows.isEmpty else { return }
        let origin = NSPoint(x: clip.bounds.origin.x, y: tableView.rect(ofRow: rows.count - 1).maxY - fromEnd)
        NSAnimationContext.runAnimationGroup { context in
            context.duration = 0
            clip.animator().setBoundsOrigin(origin)
        }
        scrollView.reflectScrolledClipView(clip)
    }

    /// Scrolls the clip view to the end of the document itself rather than to the last row:
    /// `scrollRowToVisible` aligns a row taller than the viewport to its top, so a long final
    /// reply opened with its tail out of view, and it leaves the bottom inset unscrolled.
    private func scrollToBottom(animated: Bool) {
        guard !rows.isEmpty else { return }
        // Forces the row positions so the document height is the laid-out one.
        let documentHeight = tableView.rect(ofRow: rows.count - 1).maxY
        let clip = scrollView.contentView
        let insets = scrollView.contentInsets
        let bottom = max(-insets.top, documentHeight + insets.bottom - clip.bounds.height)
        let origin = NSPoint(x: clip.bounds.origin.x, y: bottom)
        // Always through the animator: a new animation on the bounds origin replaces one in
        // flight. A plain `scroll(to:)` holds for a frame and is then dragged back to the
        // earlier animation's target, which a send makes stale (the working row lands right
        // after the message, one row below where that animation was heading).
        NSAnimationContext.runAnimationGroup { context in
            context.duration = animated ? 0.18 : 0
            context.allowsImplicitAnimation = animated
            clip.animator().setBoundsOrigin(origin)
        }
        scrollView.reflectScrolledClipView(clip)
        isPinnedToBottom = true
        jumpButton.isHidden = true
    }

    // MARK: - Actions

    /// Set by the split view so a message that moves to a new group chat opens it.
    var onRedirect: ((Chat.ID) -> Void)?

    private func send(_ text: String, attachments: [OutgoingAttachment]) {
        guard let chatID else { return }
        isPinnedToBottom = true
        let destination = store.send(text, attachments: attachments, in: chatID)
        composer.isResponding = store.isResponding(in: chatID)
        if destination != chatID { onRedirect?(destination) }
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
        let showsName = showsName(for: message)
        return max(
            1,
            layout.height(
                for: chatRow,
                message: message,
                tableWidth: max(tableView.bounds.width, 320),
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

        case .working:
            let cell = dequeue(WorkingCellView.identifier) { WorkingCellView() }
            configure(cell: cell, row: chatRow)
            return cell

        case .status:
            let cell = dequeue(StatusCellView.identifier) { StatusCellView() }
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
            case .tool, .handoff:
                identifier = HandoffCellView.identifier
                cell = dequeue(identifier) { HandoffCellView() }
            case .notice:
                identifier = NoticeCellView.identifier
                cell = dequeue(identifier) { NoticeCellView() }
            case .permission:
                identifier = PermissionCellView.identifier
                cell = dequeue(identifier) { PermissionCellView() }
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

    /// A bot's text in a group carries its name and avatar; a DM's bot needs neither.
    private func showsName(for message: Message?) -> Bool {
        guard let chatID, store.chat(chatID)?.isGroup == true, let message,
            case .bot = message.author, case .text = message.body
        else { return false }
        return true
    }

    /// The plugin behind a `<plugin>__<tool>` row, by name, from the bot's Runner.
    private func pluginName(of tool: ToolInvocation, bot botID: Bot.ID) -> String? {
        guard let range = tool.name.range(of: "__"), let bot = store.bot(botID) else { return nil }
        let pluginID = String(tool.name[..<range.lowerBound])
        return store.device(bot.runnerID)?.plugins.first { $0.id == pluginID }?.name ?? pluginID.capitalized
    }

    private func configure(cell: NSView, row: ChatRow) {
        switch row {
        case let .day(date):
            (cell as? DayCellView)?.configure(date)

        case let .working(botIDs):
            guard let chatID, let chat = store.chat(chatID) else { return }
            // The activity is the bot's latest tool, running or just finished: between two
            // commands the line keeps reading "Running commands…" instead of flashing back to
            // the name every time a call ends. It reverts once the bot says something, or
            // after a sent message, whose marker already tells the story.
            var activity: String?
            if botIDs.count == 1, let last = chat.messages.last, last.author == .bot(botIDs[0]),
                case let .tool(tool) = last.body, !tool.isSentMessage
            {
                let target = store.bots.first { tool.detail.localizedCaseInsensitiveContains("\"bot\": \"\($0.name)\"") }
                activity = WorkingCellView.activity(for: tool, targetName: target?.name, pluginName: pluginName(of: tool, bot: botIDs[0]))
            }
            // A model call being asked again outranks the last tool: the bot is waiting, not working.
            if let note = store.retryNote(for: chatID) {
                activity = note
            }
            (cell as? WorkingCellView)?.configure(
                bots: botIDs.compactMap(store.bot), activity: activity, showsName: chat.isGroup)

        case let .status(text):
            (cell as? StatusCellView)?.configure(text)

        case let .message(id, groupStart):
            guard let message = message(for: id), let chatID, let chat = store.chat(chatID) else {
                return
            }

            switch message.body {
            case .text:
                guard let messageCell = cell as? MessageCellView else { return }
                let showsName = showsName(for: message)
                let name: String
                let nameColor: NSColor
                if showsName, case let .bot(botID) = message.author {
                    name = store.bot(botID)?.name ?? L("Bot")
                    nameColor = store.bot(botID)?.accent.color ?? .secondaryLabelColor
                } else {
                    name = ""
                    nameColor = .secondaryLabelColor
                }
                let metrics = layout.metrics(
                    for: message,
                    showsName: showsName,
                    tableWidth: max(tableView.bounds.width, 320))
                let items = zip(message.attachments, metrics.attachmentFrames).map { attachment, frame in
                    AttachmentsView.Item(
                        attachment: attachment,
                        url: store.localURL(for: attachment, in: chatID, messageID: message.id),
                        frame: frame)
                }
                messageCell.configure(
                    message: message,
                    groupStart: groupStart,
                    authorName: name,
                    nameColor: nameColor,
                    avatarContent: AvatarView.content(for: message.author, store: store),
                    segments: layout.rendered(for: message).segments,
                    attachments: items,
                    metrics: metrics
                )

            case let .tool(invocation):
                let recipient = store.bots.first { $0.name.caseInsensitiveCompare(invocation.recipientName) == .orderedSame }
                (cell as? HandoffCellView)?.configure(
                    mode: .outgoing(to: recipient), reason: invocation.detail, groupStart: groupStart)

            case let .handoff(from, to, reason):
                let incoming = !chat.isGroup && chat.botIDs.contains(to)
                (cell as? HandoffCellView)?.configure(
                    mode: incoming ? .incoming(from: store.bot(from)) : .handoff(from: store.bot(from), to: store.bot(to)),
                    reason: reason, groupStart: groupStart)

            case let .notice(text):
                (cell as? NoticeCellView)?.configure(
                    text: text,
                    groupStart: groupStart,
                    metrics: layout.noticeMetrics(
                        for: text, tableWidth: max(tableView.bounds.width, 320)))

            case let .permission(request):
                let permissionCell = cell as? PermissionCellView
                permissionCell?.configure(
                    request: request,
                    botName: message.author.botID.flatMap(store.bot)?.name ?? L("The bot"),
                    groupStart: groupStart)
                permissionCell?.onDecision = { [weak self] decision in
                    guard let self else { return }
                    self.store.answerPermission(chatID: chat.id, messageID: message.id, decision: decision)
                }
            }

            _ = chat
        }
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
