import AppKit

final class ChatViewController: NSViewController {
    private let store = AppStore.shared
    private let layout = ChatLayout()

    private let tableView = TranscriptTableView()
    private let scrollView = NSScrollView()
    private let composer = ComposerView()
    private let emptyState = ChatEmptyStateView()
    private let jumpButton = NSButton()

    private var chatID: Chat.ID?
    private var rows: [ChatRow] = []
    /// Where each message sits in the chat's messages, so a row finds its message at once.
    private var messageIndex: [Message.ID: Int] = [:]
    /// Shown after the last message when a turn ended without a reply; cleared by the next message.
    private var stoppedNotice: String?
    private var isPinnedToBottom = true
    private var lastWidth: CGFloat = 0
    /// A live resize changed the width and only the rows on screen were measured again.
    private var offscreenHeightsStale = false

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

        composer.onSend = { [weak self] text, attachments, mentions in self?.send(text, attachments: attachments, mentions: mentions) }
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
        tableView.onLiveResizeEnd = { [weak self] in self?.liveResizeDidEnd() }
        tableView.rowIdentity = { [weak self] row in
            guard let self, rows.indices.contains(row) else { return row }
            return rows[row].identity
        }
        tableView.rowSpokenText = { [weak self] row in
            guard let self, rows.indices.contains(row) else { return "" }
            return spokenText(for: rows[row])
        }
        tableView.scrollRowIntoView = { [weak self] row in self?.scrollIntoView(row: row) }
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
        // In case the end of a live resize never reached the table.
        if offscreenHeightsStale, !tableView.inLiveResize { liveResizeDidEnd() }
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

        if !isSameChat {
            stoppedNotice = nil
            // Measurements outlive the chat, so coming back to one measures nothing again.
            layout.prune(keeping: chat.messages)
        }
        rebuildRows()
        tableView.reloadData()

        let members = store.bots(in: chat)
        composer.configure(placeholder: placeholder(for: chat), bots: mentionable(in: chat))
        composer.isResponding = store.isResponding(in: newChatID)

        emptyState.isHidden = !chat.messages.isEmpty
        emptyState.configure(chat: chat, bots: members)

        if !isSameChat { composer.text = "" }
        isPinnedToBottom = true
        // Before returning, not from a block on the main queue: AppKit can display the window
        // before that block runs, which shows a long transcript from its first row for a frame.
        scrollToBottom(animated: false)
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
        messageIndex.removeAll()
        guard let chatID, let chat = store.chat(chatID) else { return }

        var previousAuthor: Message.Author?
        var previousDate: Date?

        for (index, message) in chat.messages.enumerated() {
            messageIndex[message.id] = index
            // Tool calls are the bot's business; a sent message leaves a marker, and a command
            // shows as its card while it needs the user.
            if case let .tool(tool) = message.body, !tool.isShown { continue }
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

    /// Whether the cursor is in the row of `messageID`, such as a command card's answer field.
    private func holdsFocus(_ messageID: Message.ID) -> Bool {
        guard let index = rows.firstIndex(where: { $0.messageID == messageID }),
            let cell = tableView.view(atColumn: 0, row: index, makeIfNecessary: false),
            let responder = view.window?.firstResponder as? NSView
        else { return false }
        return responder.isDescendant(of: cell)
    }

    private func message(for id: Message.ID) -> Message? {
        guard let chatID, let chat = store.chat(chatID) else { return nil }
        if let index = messageIndex[id], chat.messages.indices.contains(index), chat.messages[index].id == id {
            return chat.messages[index]
        }
        return chat.messages.first { $0.id == id }
    }

    // MARK: - Store events

    private func handle(_ event: StoreEvent) {
        guard let chatID else { return }

        switch event {
        case let .messageAdded(id, _) where id == chatID:
            let wasPinned = isPinnedToBottom
            stoppedNotice = nil
            updateRows()
            emptyState.isHidden = true
            composer.isResponding = store.isResponding(in: chatID)
            if wasPinned { scrollToBottom(animated: true) }

        case let .messageChanged(id, messageID) where id == chatID:
            // A tool row shows once it has something to show (a sent message's marker, or a
            // command that needs the user) and goes once a command's card no longer does.
            if let message = message(for: messageID), case let .tool(tool) = message.body,
                tool.isShown != rows.contains(where: { $0.messageID == messageID })
            {
                // The cursor in a card's answer field goes back to the composer as the card leaves.
                let hadFocus = holdsFocus(messageID)
                layout.invalidate(messageID)
                updateRows()
                if isPinnedToBottom { scrollToBottom(animated: false) }
                if hadFocus { composer.focus() }
            } else {
                updateRow(for: messageID)
            }
            refreshWorkingRow()
            composer.isResponding = store.isResponding(in: chatID)

        case let .messageRemoved(id, messageID) where id == chatID:
            layout.invalidate(messageID)
            updateRows()
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
            updateRows()
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
            // A member's image may have landed.
            reconfigureVisibleRows()

        case .rosterChanged:
            // A bot's name or look may have changed.
            reconfigureVisibleRows()

        case let .olderMessagesLoaded(id) where id == chatID:
            olderMessagesLoaded()

        case .snapshotReplaced:
            show(chatID: chatID)

        default:
            break
        }
    }

    /// Brings the table to the rows the chat has now. Scrolled back, what the user is reading
    /// stays in place: a long transcript's table estimates the heights of rows it has not
    /// shown, and learning about new rows moves its estimates.
    private func updateRows() {
        if isPinnedToBottom {
            applyRows()
        } else {
            holdingFirstVisibleMessage(by: .bottom, applyRows)
        }
    }

    /// Replaces the one run of rows that differs. The rows around it keep their cells and
    /// heights: the table measures only what is new, and a selection in a message on screen
    /// survives a reply arriving.
    private func applyRows() {
        let old = rows
        rebuildRows()
        let run = ChatRow.changedRun(from: old, to: rows)
        if !run.removed.isEmpty || !run.inserted.isEmpty {
            tableView.beginUpdates()
            tableView.removeRows(at: IndexSet(integersIn: run.removed), withAnimation: [])
            tableView.insertRows(at: IndexSet(integersIn: run.inserted), withAnimation: [])
            tableView.endUpdates()
        }
        refreshWorkingRow()
    }

    /// Configures the cells on screen again, for what a row shows beyond its message: a bot's
    /// name and avatar, or a new width. Returns the rows on screen.
    @discardableResult
    private func reconfigureVisibleRows() -> Range<Int> {
        let visible = tableView.rows(in: scrollView.contentView.bounds)
        let onScreen = (visible.location..<NSMaxRange(visible)).clamped(to: rows.indices)
        for row in onScreen {
            guard let cell = tableView.view(atColumn: 0, row: row, makeIfNecessary: false)
            else { continue }
            configure(cell: cell, row: rows[row])
        }
        return onScreen
    }

    /// The working row reads the bot's latest step, which changes while the row stays.
    private func refreshWorkingRow() {
        guard let last = rows.indices.last, case .working = rows[last],
            let cell = tableView.view(atColumn: 0, row: last, makeIfNecessary: false)
        else { return }
        configure(cell: cell, row: rows[last])
    }

    private func updateRow(for messageID: Message.ID) {
        // A streamed reply is at the end, so look from there.
        guard let index = rows.lastIndex(where: { $0.messageID == messageID }) else { return }

        if let cell = tableView.view(atColumn: 0, row: index, makeIfNecessary: false) {
            configure(cell: cell, row: rows[index])
        }

        if isPinnedToBottom {
            noteHeights(of: IndexSet(integer: index))
            scrollToBottom(animated: false)
        } else {
            // A reply growing in the message being read grows downward.
            holdingFirstVisibleMessage(by: .top) { noteHeights(of: IndexSet(integer: index)) }
        }
    }

    private func noteHeights(of rows: IndexSet) {
        NSAnimationContext.runAnimationGroup { context in
            context.duration = 0
            tableView.noteHeightOfRows(withIndexesChanged: rows)
        }
    }

    /// Bubble widths are baked in at configure time, so a new width configures the visible
    /// cells again and measures their rows. While the user drags the window's edge only the
    /// rows on screen are measured; the rest are once the drag ends. Scrolled back, the top of
    /// what the user is reading stays in place.
    private func reloadHeights() {
        guard !rows.isEmpty else { return }

        let onScreen = reconfigureVisibleRows()
        if tableView.inLiveResize {
            noteHeights(of: IndexSet(integersIn: onScreen))
            offscreenHeightsStale = true
            return
        }
        offscreenHeightsStale = false
        let all = IndexSet(integersIn: rows.indices)
        if isPinnedToBottom {
            noteHeights(of: all)
        } else {
            holdingFirstVisibleMessage(by: .top) { noteHeights(of: all) }
        }
    }

    /// The rows off screen take their heights at the width the resize ended on. What the user
    /// was reading stays in place; a pinned transcript stays at its end.
    private func liveResizeDidEnd() {
        guard offscreenHeightsStale else { return }
        offscreenHeightsStale = false
        let all = IndexSet(integersIn: rows.indices)
        if isPinnedToBottom {
            noteHeights(of: all)
            scrollToBottom(animated: false)
        } else {
            holdingFirstVisibleMessage(by: .top) { noteHeights(of: all) }
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

    /// Older messages went in above the first row: keep what is on screen where it is. The
    /// message held keeps its bottom edge, since an older neighbor can change its top padding.
    private func olderMessagesLoaded() {
        holdingFirstVisibleMessage(by: .bottom, applyRows)
    }

    private enum Edge {
        case top, bottom
    }

    /// Runs `change`, then scrolls so the first message starting on screen has its `edge` where
    /// it was, whatever went in, came out, or changed height above it.
    private func holdingFirstVisibleMessage(by edge: Edge, _ change: () -> Void) {
        let clip = scrollView.contentView
        let visible = tableView.rows(in: clip.bounds)
        let onScreen = (visible.location..<NSMaxRange(visible)).clamped(to: rows.indices)
        let messages = onScreen.filter { rows[$0].messageID != nil }
        let first = messages.first { tableView.rect(ofRow: $0).minY >= clip.bounds.minY } ?? messages.first
        func y(of row: Int) -> CGFloat {
            let rect = tableView.rect(ofRow: row)
            return edge == .top ? rect.minY : rect.maxY
        }
        let anchor = first.flatMap { row in
            rows[row].messageID.map { (id: $0, offset: y(of: row) - clip.bounds.minY) }
        }
        change()
        guard let anchor, let row = rows.firstIndex(where: { $0.messageID == anchor.id }) else { return }
        let origin = NSPoint(x: clip.bounds.origin.x, y: y(of: row) - anchor.offset)
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

    /// Brings a row a screen reader moved to fully into view, clear of the titlebar and the
    /// composer, or its top when it is taller than the space between them. Near the first
    /// message this asks for the page before it, as scrolling there does.
    private func scrollIntoView(row: Int) {
        guard rows.indices.contains(row) else { return }
        let clip = scrollView.contentView
        let insets = scrollView.contentInsets
        let rect = tableView.rect(ofRow: row)
        let visibleTop = clip.bounds.minY + insets.top
        let visibleBottom = clip.bounds.maxY - insets.bottom
        let y: CGFloat
        if rect.minY < visibleTop || rect.height > visibleBottom - visibleTop {
            y = rect.minY - insets.top
        } else if rect.maxY > visibleBottom {
            y = rect.maxY + insets.bottom - clip.bounds.height
        } else {
            return
        }
        let documentHeight = tableView.rect(ofRow: rows.count - 1).maxY
        let lowest = max(-insets.top, documentHeight + insets.bottom - clip.bounds.height)
        let origin = NSPoint(x: clip.bounds.origin.x, y: min(max(y, -insets.top), lowest))
        NSAnimationContext.runAnimationGroup { context in
            context.duration = 0
            clip.animator().setBoundsOrigin(origin)
        }
        scrollView.reflectScrolledClipView(clip)
    }

    // MARK: - Actions

    /// Set by the split view so a message that moves to a new group chat opens it.
    var onRedirect: ((Chat.ID) -> Void)?

    private func send(_ text: String, attachments: [OutgoingAttachment], mentions: [Bot.ID]) {
        guard let chatID else { return }
        isPinnedToBottom = true
        let destination = store.send(text, attachments: attachments, mentions: mentions, in: chatID)
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
        dequeue(TransparentRowView.identifier) { TransparentRowView() }
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
            case let .tool(tool) where tool.run != nil:
                identifier = CommandCellView.identifier
                cell = dequeue(identifier) { CommandCellView() }
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

    /// What the one working bot is doing, for the working row. The activity is the bot's latest
    /// tool, running or just finished: between two commands the line keeps reading the last one
    /// instead of flashing back to the name every time a call ends. It reverts once the bot says
    /// something, or after a sent message, whose marker already tells the story.
    private func activity(of botIDs: [Bot.ID], in chat: Chat) -> String? {
        var activity: String?
        if botIDs.count == 1, let last = chat.messages.last, last.author == .bot(botIDs[0]),
            case let .tool(tool) = last.body, !tool.isSentMessage
        {
            let target = tool.targetBotID.flatMap(store.bot)
            activity = WorkingCellView.activity(for: tool, targetName: target?.name, pluginName: pluginName(of: tool, bot: botIDs[0]))
        }
        // A model thinking about its next step outranks the last tool.
        if botIDs.count == 1, store.isThinking(botIDs[0], in: chat.id) {
            activity = L("Thinking", context: "status")
        }
        // A model call being asked again outranks both: the bot is waiting, not working.
        if let note = store.retryNote(for: chat.id) {
            activity = note
        }
        return activity
    }

    /// A bot-to-bot marker and the message it carries: a sent message, one that arrived in a
    /// DM, or a handoff in a group.
    private func handoff(of message: Message, in chat: Chat) -> (mode: HandoffCellView.Mode, reason: String)? {
        switch message.body {
        case let .tool(invocation):
            return (.outgoing(to: invocation.targetBotID.flatMap(store.bot)), invocation.detail)
        case let .handoff(from, to, reason):
            let incoming = !chat.isGroup && chat.botIDs.contains(to)
            let mode: HandoffCellView.Mode = incoming
                ? .incoming(from: store.bot(from)) : .handoff(from: store.bot(from), to: store.bot(to))
            return (mode, reason)
        default:
            return nil
        }
    }

    private func botName(of message: Message) -> String {
        message.author.botID.flatMap(store.bot)?.name ?? L("The bot")
    }

    /// What a screen reader says for a row: "Chef: Saved in launch/announcement.md." It comes
    /// from the chat rather than the cell, so a client reads every row without the table
    /// building cells, and the cell on screen says the same.
    private func spokenText(for row: ChatRow) -> String {
        guard let chatID, let chat = store.chat(chatID) else { return "" }
        switch row {
        case let .day(date):
            return Format.daySeparator(date)

        case let .working(botIDs):
            return WorkingCellView.spokenText(
                names: botIDs.compactMap(store.bot).map(\.name), activity: activity(of: botIDs, in: chat),
                showsName: chat.isGroup)

        case let .status(text):
            return text

        case let .message(id, _):
            guard let message = message(for: id) else { return "" }
            switch message.body {
            case .text:
                let author: String
                switch message.author {
                case .you: author = L("You")
                case let .bot(botID): author = store.bot(botID)?.name ?? L("Bot")
                case .system: author = "Lorca"
                }
                let words = layout.rendered(for: message).plainText
                return "\(author): \(words.isEmpty ? Attachment.summary(message.attachments) : words)"
            case let .tool(tool) where tool.run != nil:
                return CommandCellView.spokenText(run: tool.run!, botName: botName(of: message))
            case .tool, .handoff:
                guard let marker = handoff(of: message, in: chat) else { return "" }
                return HandoffCellView.spokenText(mode: marker.mode, reason: marker.reason)
            case let .notice(text):
                return text
            case let .permission(request):
                return PermissionCellView.spokenText(request: request, botName: botName(of: message))
            }
        }
    }

    private func configure(cell: NSView, row: ChatRow) {
        switch row {
        case let .day(date):
            (cell as? DayCellView)?.configure(date)

        case let .working(botIDs):
            guard let chatID, let chat = store.chat(chatID) else { return }
            (cell as? WorkingCellView)?.configure(
                bots: botIDs.compactMap(store.bot), activity: activity(of: botIDs, in: chat), showsName: chat.isGroup)

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

            case let .tool(tool) where tool.run != nil:
                guard let commandCell = cell as? CommandCellView, let run = tool.run else { return }
                commandCell.configure(run: run, messageID: message.id, botName: botName(of: message), groupStart: groupStart)
                commandCell.onDecision = { [weak self] decision in
                    self?.store.answerPermission(chatID: chat.id, messageID: message.id, decision: decision)
                }
                commandCell.onSend = { [weak self] text in
                    try await self?.store.answerCommand(chatID: chat.id, messageID: message.id, text: text)
                }
                commandCell.onStop = { [weak self] in
                    try await self?.store.stopCommand(chatID: chat.id, messageID: message.id)
                }
                commandCell.onShowCommand = { [weak self] in
                    guard let self else { return }
                    presentAsSheet(CommandSheetViewController(
                        title: L("%@'s command", botName(of: message)), command: run.command))
                }

            case .tool, .handoff:
                guard let marker = handoff(of: message, in: chat) else { return }
                (cell as? HandoffCellView)?.configure(mode: marker.mode, reason: marker.reason, groupStart: groupStart)

            case let .notice(text):
                (cell as? NoticeCellView)?.configure(
                    text: text,
                    groupStart: groupStart,
                    metrics: layout.noticeMetrics(
                        for: message, tableWidth: max(tableView.bounds.width, 320)))

            case let .permission(request):
                let permissionCell = cell as? PermissionCellView
                permissionCell?.configure(request: request, botName: botName(of: message), groupStart: groupStart)
                permissionCell?.onDecision = { [weak self] decision in
                    guard let self else { return }
                    self.store.answerPermission(chatID: chat.id, messageID: message.id, decision: decision)
                }
                permissionCell?.onShowCommand = { [weak self] in
                    guard let self else { return }
                    presentAsSheet(CommandSheetViewController(
                        title: "\(botName(of: message)) \(request.verbPhrase)", command: request.fullCommand))
                }
            }

            _ = chat
        }
    }

}

final class TransparentRowView: NSTableRowView {
    static let identifier = NSUserInterfaceItemIdentifier("TransparentRow")

    override func drawBackground(in dirtyRect: NSRect) {}
    override func drawSelection(in dirtyRect: NSRect) {}
    override var isEmphasized: Bool {
        get { false }
        set {}
    }
}
