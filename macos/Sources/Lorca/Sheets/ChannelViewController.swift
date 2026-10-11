import AppKit

/// One channel's details: whether it listens, and what holds it when a message's turn didn't
/// finish or the account can't be read; where it listens and what it takes; the chats asking to
/// be allowed; the task the bot does with each message; and its conversations, each a click away.
/// The bot sets a channel up and changes it when asked in chat; here the user pauses it, allows
/// or ignores a chat, settles a held message, or removes it.
final class ChannelViewController: SheetViewController {
    private let store = AppStore.shared
    private let channelID: String
    private let bot: Bot

    private let status = SectionView(title: "")
    private let requests = SectionView(title: L("Asking to Chat"))
    private let task = SectionView(title: L("Task"))
    private let taskText = Build.label("", font: .systemFont(ofSize: 12), lines: 0)
    private let conversations = SectionView(title: L("Conversations"))
    private let toggle = NSSwitch()
    private let removeButton = NSButton()
    private let scroll = NSScrollView()
    /// The most the sections take before they scroll: a long task or many chats asking would
    /// otherwise push the sheet past its window.
    private lazy var scrollCap = scroll.heightAnchor.constraint(lessThanOrEqualToConstant: Self.tallest)
    private static let tallest: CGFloat = 520

    /// Opens one of the channel's conversations.
    var onOpenChat: ((Chat.ID) -> Void)?

    init(channelID: String, bot: Bot) {
        self.channelID = channelID
        self.bot = bot
        super.init(title: AppStore.shared.channel(channelID)?.name ?? L("Channel"), subtitle: "", width: 480)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()
        toggle.controlSize = .small
        toggle.target = self
        toggle.action = #selector(toggled)
        toggle.setAccessibilityLabel(L("Listening"))

        taskText.isSelectable = true
        let taskRow = NSView()
        taskRow.translatesAutoresizingMaskIntoConstraints = false
        taskRow.addSubview(taskText)
        NSLayoutConstraint.activate([
            taskText.leadingAnchor.constraint(equalTo: taskRow.leadingAnchor, constant: 12),
            taskText.trailingAnchor.constraint(equalTo: taskRow.trailingAnchor, constant: -12),
            taskText.topAnchor.constraint(equalTo: taskRow.topAnchor, constant: 10),
            taskText.bottomAnchor.constraint(equalTo: taskRow.bottomAnchor, constant: -10),
        ])
        task.setRows([taskRow])

        let sections = Build.stack([status, requests, task, conversations], spacing: 12)
        sections.orientation = .vertical
        sections.alignment = .leading
        for section in sections.arrangedSubviews {
            section.widthAnchor.constraint(equalTo: sections.widthAnchor).isActive = true
        }
        let document = FlippedView()
        document.translatesAutoresizingMaskIntoConstraints = false
        document.addSubview(sections)
        scroll.translatesAutoresizingMaskIntoConstraints = false
        scroll.drawsBackground = false
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.documentView = document
        contentStack.addArrangedSubview(scroll)
        // As tall as the sections, up to a height that keeps the sheet inside its window.
        let fit = scroll.heightAnchor.constraint(equalTo: document.heightAnchor)
        fit.priority = .defaultLow
        NSLayoutConstraint.activate([
            scroll.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            scrollCap,
            fit,
            document.widthAnchor.constraint(equalTo: scroll.contentView.widthAnchor),
            sections.topAnchor.constraint(equalTo: document.topAnchor),
            sections.leadingAnchor.constraint(equalTo: document.leadingAnchor),
            sections.trailingAnchor.constraint(equalTo: document.trailingAnchor),
            sections.bottomAnchor.constraint(equalTo: document.bottomAnchor),
        ])
        // Wrapping text needs its width before it can say how tall it is.
        taskText.preferredMaxLayoutWidth = 480 - 48 - 24

        removeButton.bezelStyle = .rounded
        removeButton.attributedTitle = NSAttributedString(
            string: L("Remove…"), attributes: [.foregroundColor: NSColor.systemRed, .font: NSFont.systemFont(ofSize: NSFont.systemFontSize)])
        removeButton.target = self
        removeButton.action = #selector(confirmRemove)
        setButtons(confirm: L("Done"), cancel: nil, leading: removeButton)
        capToWindow()
        refresh()
    }

    /// Fits the sections to the window the sheet comes down from, before the sheet takes its
    /// size: its header and buttons take about 120 pt, and the sheet keeps a margin.
    private func capToWindow() {
        guard let parent = presentingViewController?.view.window else { return }
        scrollCap.constant = max(160, min(Self.tallest, parent.contentLayoutRect.height - 190))
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        store.observe(self) { [weak self] event in
            switch event {
            case .rosterChanged, .chatsChanged, .snapshotReplaced:
                self?.refresh()
            default:
                break
            }
        }
    }

    /// Redraws from the store; the sheet closes when the channel is gone.
    private func refresh() {
        guard isViewLoaded else { return }
        guard let channel = store.channel(channelID) else {
            dismiss(nil)
            return
        }
        toggle.state = channel.isPaused ? .off : .on
        var rows: [NSView] = [AccessoryRow(key: L("Listening"), accessory: toggle)]
        switch channel.state {
        case .held where channel.heldDelivery == nil:
            // Nothing to settle: it waits for the user, or for its bot to come back.
            rows.append(KeyValueRow(key: L("State"), value: L("On hold"), tint: .systemOrange))
            if !channel.detail.isEmpty { rows.append(NoteRow(text: channel.detail)) }
        case .held:
            let held = ActionRow(key: L("State"), value: L("On hold"), tint: .systemOrange, actionTitle: L("Try Again"), secondActionTitle: L("Skip"))
            held.onAction = { [weak self] in self?.settle(retry: true) }
            held.onSecondAction = { [weak self] in self?.settle(retry: false) }
            rows.append(held)
            rows.append(NoteRow(text: L("A message’s turn didn’t finish, so later messages wait. Read its conversation, then try it again or skip it.")))
        case .offline:
            rows.append(KeyValueRow(key: L("State"), value: L("Can’t connect"), tint: .systemOrange))
            if !channel.detail.isEmpty { rows.append(NoteRow(text: channel.detail)) }
        case .listening where !channel.detail.isEmpty:
            rows.append(NoteRow(text: channel.detail))
        default:
            break
        }
        let chats = channel.chats.isEmpty
            ? L("None yet")
            : channel.chats.map { $0.title.isEmpty ? $0.id : $0.title }.joined(separator: L(", ", context: "list"))
        rows += [
            KeyValueRow(key: L("Account"), value: store.accountName(of: channel)),
            KeyValueRow(key: L("Chats"), value: chats),
            KeyValueRow(key: L("Messages"), value: channel.listen.summary),
        ]
        if channel.chats.isEmpty && channel.requests.isEmpty {
            rows.append(NoteRow(text: channel.service == "slack"
                ? L("Invite the bot to a channel or send it a message, then allow the chat here.")
                : L("Add the bot to a group or send it a message, then allow the chat here.")))
        }
        status.setRows(rows)

        // Anyone can message the bot or add it to a group; it answers only where the user allows.
        requests.isHidden = channel.requests.isEmpty
        var asking: [NSView] = channel.requests.map { request in
            let kind = request.isPrivate ? L("Direct message") : request.from.isEmpty ? L("Group") : L("Group · %@", request.from)
            let row = ActionRow(key: request.title, value: kind, tint: .secondaryLabelColor, actionTitle: L("Allow"), secondActionTitle: L("Ignore"))
            row.onAction = { [weak self] in self?.answer(request.id, allow: true) }
            row.onSecondAction = { [weak self] in self?.answer(request.id, allow: false) }
            return row
        }
        asking.append(NoteRow(text: L("%@ answers only in chats you allow. An ignored chat doesn’t ask again.", bot.name)))
        requests.setRows(asking)
        if taskText.stringValue != channel.task { taskText.stringValue = channel.task }

        let kept = store.conversations(of: channel.id)
        conversations.isHidden = kept.isEmpty
        conversations.setRows(kept.prefix(6).map { chat in
            let row = DisclosureRow(key: store.title(for: chat))
            row.setValue(Format.stamp(chat.lastActivity))
            row.onClick = { [weak self] in
                self?.dismiss(nil)
                self?.onOpenChat?(chat.id)
            }
            return row
        })
        fitSheetToContent()
    }

    @objc private func toggled() {
        store.setChannelPaused(channelID, toggle.state == .off)
    }

    private func answer(_ chatID: String, allow: Bool) {
        store.answerChatRequest(of: channelID, chatID: chatID, allow: allow)
    }

    private func settle(retry: Bool) {
        store.settleHeldMessage(of: channelID, retry: retry)
    }

    @objc private func confirmRemove() {
        guard let channel = store.channel(channelID), let window = view.window else { return }
        let alert = NSAlert()
        alert.messageText = L("Remove “%@”?", channel.name)
        alert.informativeText = L("%@ stops listening there. Its conversations stay.", bot.name)
        alert.addButton(withTitle: L("Remove"))
        alert.addButton(withTitle: L("Cancel"))
        alert.buttons[0].hasDestructiveAction = true
        alert.alertStyle = .warning
        alert.beginSheetModal(for: window) { [weak self] response in
            guard let self, response == .alertFirstButtonReturn else { return }
            self.store.removeChannel(self.channelID)
            self.dismiss(nil)
        }
    }
}
