import AppKit

/// One channel's details: whether it listens, and what holds it when a message's turn didn't
/// finish or the account can't be read; where it listens and what it takes; the task the bot
/// does with each message; and its conversations, each a click away. The bot sets a channel up
/// and changes it when asked in chat; here the user pauses it, settles a held message, or
/// removes it.
final class ChannelViewController: SheetViewController {
    private let store = AppStore.shared
    private let channelID: String
    private let bot: Bot

    private let status = SectionView(title: "")
    private let task = SectionView(title: L("Task"))
    private let taskText = Build.label("", font: .systemFont(ofSize: 12), lines: 0)
    private let conversations = SectionView(title: L("Conversations"))
    private let toggle = NSSwitch()
    private let removeButton = NSButton()

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

        for section in [status, task, conversations] {
            contentStack.addArrangedSubview(section)
            section.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
        }
        // Wrapping text needs its width before it can say how tall it is.
        taskText.preferredMaxLayoutWidth = 480 - 48 - 24

        removeButton.bezelStyle = .rounded
        removeButton.attributedTitle = NSAttributedString(
            string: L("Remove…"), attributes: [.foregroundColor: NSColor.systemRed, .font: NSFont.systemFont(ofSize: NSFont.systemFontSize)])
        removeButton.target = self
        removeButton.action = #selector(confirmRemove)
        setButtons(confirm: L("Done"), cancel: nil, leading: removeButton)
        refresh()
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
            ? L("Every chat the bot is in")
            : channel.chats.map { $0.title.isEmpty ? $0.id : $0.title }.joined(separator: L(", ", context: "list"))
        rows += [
            KeyValueRow(key: L("Account"), value: store.accountName(of: channel)),
            KeyValueRow(key: L("Chats"), value: chats),
            KeyValueRow(key: L("Messages"), value: channel.listen.summary),
        ]
        status.setRows(rows)
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
