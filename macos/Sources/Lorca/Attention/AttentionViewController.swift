import AppKit

struct AttentionRevision: Codable, Equatable {
    var counter: UInt64
    var deviceId: String
    var params: [String: Any] { ["counter": counter, "device_id": deviceId] }
}
struct AttentionSource: Decodable, Equatable {
    var chatId: String
    var taskId: String?
    var messageId: String?
    var reviewId: String?
}
struct AttentionItem: Decodable, Equatable {
    var id: String
    var category: String
    var title: String
    var summary: String
    var nextAction: String
    var coordinatorBotId: String
    var sources: [AttentionSource]
    var urgent: Bool
    var revision: AttentionRevision
}
struct AttentionBrief: Decodable, Equatable {
    var coordinatorBotId: String
    var chatId: String
    var decisions: [String]
    var changes: [String]
    var nextAction: String
    var updatedAt: Double
}
struct AttentionPreferences: Decodable, Equatable {
    var summaries = true
    var urgentDirect = true
    var defaultCoordinatorBotId: String?
}
struct AttentionView: Decodable, Equatable {
    var items: [AttentionItem] = []
    var briefs: [AttentionBrief] = []
    var preferences = AttentionPreferences()
}

extension AttentionItem {
    /// The word for the row's status line and its symbol.
    var categoryTitle: String {
        switch category {
        case "review": L("Review")
        case "blocker": L("Blocker")
        case "commitment": L("Commitment")
        default: L("Change")
        }
    }

    var symbolName: String {
        switch category {
        case "review": "doc.text.magnifyingglass"
        case "blocker": "hand.raised"
        case "commitment": "calendar"
        default: "arrow.triangle.2.circlepath"
        }
    }
}

/// The account's attention, in a popover under the toolbar's Attention button: each
/// coordinator's latest brief, then what waits on the user, urgent first. A row opens its
/// chat; its menu opens another source chat or marks it resolved. The options menu holds the
/// notification switches and the coordinator. Bots keep the list; the CLI sends it whole
/// (`attention.changed`), and the popover follows it while it shows.
final class AttentionViewController: NSViewController, NSMenuDelegate {
    static let width: CGFloat = 380
    private static let maxHeight: CGFloat = 520
    private static let headerHeight: CGFloat = 40

    private let store = AppStore.shared
    private let options = HoverButton(
        symbol: "ellipsis.circle", tooltip: L("Options"), target: nil, action: #selector(showOptions(_:)))
    private let scrollView = NSScrollView()
    private let document = FlippedView()
    private let list = Build.stack([], spacing: 0)
    /// What the rows show, so an unrelated roster or chat change leaves them, and the pointer's
    /// highlight, alone.
    private var rendered: (attention: AttentionView, names: [String])?

    /// A row was clicked: show its chat.
    var onOpen: ((Chat.ID) -> Void)?

    override func loadView() {
        let root = NSView()
        let title = Build.label(L("Attention"), font: .systemFont(ofSize: 13, weight: .semibold))
        let menu = NSMenu()
        menu.delegate = self
        options.menu = menu
        options.target = self
        options.setAccessibilityLabel(L("Options"))
        scrollView.drawsBackground = false
        scrollView.hasVerticalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.documentView = document
        document.addSubview(list)
        for view: NSView in [title, options, scrollView] {
            view.translatesAutoresizingMaskIntoConstraints = false
            root.addSubview(view)
        }
        NSLayoutConstraint.activate([
            title.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: 16),
            title.centerYAnchor.constraint(equalTo: root.topAnchor, constant: Self.headerHeight / 2 + 2),
            options.trailingAnchor.constraint(equalTo: root.trailingAnchor, constant: -8),
            options.centerYAnchor.constraint(equalTo: title.centerYAnchor),
            options.widthAnchor.constraint(equalToConstant: 28),
            options.heightAnchor.constraint(equalToConstant: 28),
            scrollView.topAnchor.constraint(equalTo: root.topAnchor, constant: Self.headerHeight),
            scrollView.leadingAnchor.constraint(equalTo: root.leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: root.trailingAnchor),
            scrollView.bottomAnchor.constraint(equalTo: root.bottomAnchor),
            list.topAnchor.constraint(equalTo: document.topAnchor),
            list.leadingAnchor.constraint(equalTo: document.leadingAnchor),
            list.widthAnchor.constraint(equalToConstant: Self.width),
        ])
        view = root
        store.observe(self) { [weak self] event in
            switch event {
            case .attentionChanged, .snapshotReplaced, .rosterChanged, .chatsChanged: self?.reload()
            default: break
            }
        }
        reload()
    }

    private func reload() {
        let attention = store.attention
        let names = attention.briefs.map { store.bot($0.coordinatorBotId)?.name ?? "" }
            + attention.items.flatMap { sourceChats($0).map(store.title(for:)) }
        guard rendered?.attention != attention || rendered?.names != names else { return }
        rendered = (attention, names)
        for view in list.arrangedSubviews { view.removeFromSuperview() }
        for brief in attention.briefs { add(briefRow(brief)) }
        for item in attention.items { add(itemRow(item)) }
        if attention.items.isEmpty && attention.briefs.isEmpty {
            let empty = Build.label(
                L("Nothing needs your attention"), font: .systemFont(ofSize: 12.5), color: .secondaryLabelColor,
                alignment: .center)
            let holder = NSView()
            holder.addSubview(empty)
            NSLayoutConstraint.activate([
                holder.heightAnchor.constraint(equalToConstant: 52),
                empty.centerXAnchor.constraint(equalTo: holder.centerXAnchor),
                empty.centerYAnchor.constraint(equalTo: holder.centerYAnchor, constant: -4),
            ])
            list.addArrangedSubview(holder)
        }
        Self.fill(list)
        list.layoutSubtreeIfNeeded()
        let height = list.fittingSize.height + 6
        document.frame = NSRect(x: 0, y: 0, width: Self.width, height: height)
        preferredContentSize = NSSize(width: Self.width, height: min(Self.headerHeight + height, Self.maxHeight))
    }

    /// A hairline between two rows, inset as the rows' text is.
    private func add(_ row: AttentionRowView) {
        if !list.arrangedSubviews.isEmpty {
            let line = HairlineView()
            let holder = NSView()
            holder.addSubview(line)
            line.translatesAutoresizingMaskIntoConstraints = false
            NSLayoutConstraint.activate([
                holder.heightAnchor.constraint(equalToConstant: 1),
                line.leadingAnchor.constraint(equalTo: holder.leadingAnchor, constant: 16),
                line.trailingAnchor.constraint(equalTo: holder.trailingAnchor, constant: -16),
                line.centerYAnchor.constraint(equalTo: holder.centerYAnchor),
            ])
            list.addArrangedSubview(holder)
        }
        list.addArrangedSubview(row)
    }

    // MARK: - Rows

    /// "Chef · 9:41 AM", then what was decided, what changed, and what comes next.
    private func briefRow(_ brief: AttentionBrief) -> AttentionRowView {
        let bot = store.bot(brief.coordinatorBotId)
        let avatar = AvatarView(diameter: 18)
        avatar.content = bot.map { AvatarView.content(for: $0, store: store) } ?? .system
        avatar.translatesAutoresizingMaskIntoConstraints = false
        let name = Build.label(bot?.name ?? L("Coordinator"), font: .systemFont(ofSize: 12.5, weight: .semibold))
        let stamp = Build.label(
            Format.stamp(Date(timeIntervalSince1970: brief.updatedAt)), font: .systemFont(ofSize: 11),
            color: .secondaryLabelColor)
        stamp.setContentCompressionResistancePriority(.required, for: .horizontal)
        let header = Build.stack([avatar, name, NSView(), stamp], orientation: .horizontal, spacing: 6)
        NSLayoutConstraint.activate([avatar.widthAnchor.constraint(equalToConstant: 18), avatar.heightAnchor.constraint(equalToConstant: 18)])

        // Each line's word in a column as wide as the widest, its text beside it.
        let lines =
            brief.decisions.map { (L("Decision"), $0) } + brief.changes.map { (L("Changed"), $0) }
            + [(L("Next"), brief.nextAction)]
        let keys = lines.map { Build.label($0.0, font: .systemFont(ofSize: 11.5), color: .secondaryLabelColor) }
        let keyWidth = keys.map { ceil($0.cell?.cellSize.width ?? 0) }.max() ?? 0
        let body = Build.stack(zip(keys, lines).map { key, line in
            key.widthAnchor.constraint(equalToConstant: keyWidth).isActive = true
            let value = Self.wrapping(line.1, font: .systemFont(ofSize: 12), lines: 4, width: Self.width - 32 - keyWidth - 8)
            let pair = Build.stack([key, value], orientation: .horizontal, spacing: 8, alignment: .firstBaseline)
            pair.distribution = .fill
            value.setContentHuggingPriority(.defaultLow - 1, for: .horizontal)
            return pair
        }, spacing: 4)
        let stack = Build.stack([header, body], spacing: 8)
        Self.fill(body)
        Self.fill(stack)

        let row = AttentionRowView(content: stack)
        row.setAccessibilityLabel(([L("%@’s brief", bot?.name ?? L("Coordinator"))] + lines.map { "\($0): \($1)" }).joined(separator: "\n"))
        row.onClick = { [weak self] in self?.onOpen?(brief.chatId) }
        return row
    }

    /// The category's symbol, the title, the next action, and "Review · Orchard launch".
    private func itemRow(_ item: AttentionItem) -> AttentionRowView {
        let icon = NSImageView()
        icon.image = NSImage(systemSymbolName: item.symbolName, accessibilityDescription: nil)
        icon.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 13, weight: .medium)
        icon.contentTintColor = item.urgent ? .systemRed : .controlAccentColor
        icon.translatesAutoresizingMaskIntoConstraints = false
        // The text column: past the inset, the symbol's 18 points, and the gap, as a brief's name
        // is past its avatar; to the inset.
        let textWidth = Self.width - 16 - 24 - 16
        let title = Self.wrapping(item.title, font: .systemFont(ofSize: 12.5, weight: .semibold), lines: 2, width: textWidth)
        let next = Self.wrapping(item.nextAction, font: .systemFont(ofSize: 12), lines: 3, width: textWidth)
        let status = NSMutableAttributedString(
            string: item.urgent ? L("Urgent") : item.categoryTitle,
            attributes: [.foregroundColor: item.urgent ? NSColor.systemRed : NSColor.secondaryLabelColor])
        let chats = sourceChats(item)
        if !chats.isEmpty {
            status.append(NSAttributedString(
                string: " · " + chats.map(store.title(for:)).joined(separator: ", "),
                attributes: [.foregroundColor: NSColor.secondaryLabelColor]))
        }
        status.addAttribute(.font, value: NSFont.systemFont(ofSize: 11), range: NSRange(location: 0, length: status.length))
        let statusLabel = Build.label("", font: .systemFont(ofSize: 11))
        statusLabel.attributedStringValue = status
        let text = Build.stack([title, next, statusLabel], spacing: 3)
        text.setCustomSpacing(4, after: next)
        Self.fill(text)
        let iconHolder = NSView()
        iconHolder.addSubview(icon)
        let content = Build.stack([iconHolder, text], orientation: .horizontal, spacing: 6, alignment: .top)
        content.distribution = .fill
        text.setContentHuggingPriority(.defaultLow - 1, for: .horizontal)
        // The symbol sits on the title's first line.
        NSLayoutConstraint.activate([
            iconHolder.widthAnchor.constraint(equalToConstant: 18),
            iconHolder.heightAnchor.constraint(equalTo: content.heightAnchor),
            icon.centerXAnchor.constraint(equalTo: iconHolder.centerXAnchor),
            icon.centerYAnchor.constraint(equalTo: title.firstBaselineAnchor, constant: -4),
        ])

        let row = AttentionRowView(content: content)
        row.toolTip = item.summary
        row.setAccessibilityLabel([item.urgent ? L("Urgent") : item.categoryTitle, item.title, item.nextAction, item.summary].joined(separator: ". "))
        if let first = chats.first { row.onClick = { [weak self] in self?.onOpen?(first.id) } }
        row.contextMenu = { [weak self] in self?.menu(for: item) }
        row.setAccessibilityCustomActions([
            NSAccessibilityCustomAction(name: L("Mark as Resolved")) { [weak self] in
                self?.resolve(item)
                return true
            }
        ])
        return row
    }

    private func sourceChats(_ item: AttentionItem) -> [Chat] {
        var seen = Set<Chat.ID>()
        return item.sources.compactMap { store.chat($0.chatId) }.filter { seen.insert($0.id).inserted }
    }

    /// Open each chat the item came from, then Mark as Resolved.
    private func menu(for item: AttentionItem) -> NSMenu {
        let menu = NSMenu()
        for chat in sourceChats(item) {
            let open = NSMenuItem(title: L("Open “%@”", store.title(for: chat)), action: #selector(openFromMenu(_:)), keyEquivalent: "")
            open.target = self
            open.representedObject = chat.id
            menu.addItem(open)
        }
        if !menu.items.isEmpty { menu.addItem(.separator()) }
        let resolve = NSMenuItem(title: L("Mark as Resolved"), action: #selector(resolveFromMenu(_:)), keyEquivalent: "")
        resolve.target = self
        resolve.representedObject = item.id
        menu.addItem(resolve)
        return menu
    }

    @objc private func openFromMenu(_ sender: NSMenuItem) {
        guard let id = sender.representedObject as? Chat.ID else { return }
        onOpen?(id)
    }

    @objc private func resolveFromMenu(_ sender: NSMenuItem) {
        guard let id = sender.representedObject as? String, let item = store.attention.items.first(where: { $0.id == id }) else { return }
        resolve(item)
    }

    private func resolve(_ item: AttentionItem) {
        perform { try await $0.resolveAttention(item) }
    }

    /// Stretches a vertical stack's views across it.
    private static func fill(_ stack: NSStackView) {
        for view in stack.arrangedSubviews {
            view.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true
        }
    }

    /// A label that wraps to `lines` at `width` and ends the last in an ellipsis.
    private static func wrapping(_ text: String, font: NSFont, lines: Int, width: CGFloat) -> NSTextField {
        let label = Build.label(text, font: font, lines: lines)
        label.lineBreakMode = .byTruncatingTail
        label.cell?.wraps = true
        label.cell?.truncatesLastVisibleLine = true
        label.preferredMaxLayoutWidth = width
        return label
    }

    // MARK: - Options

    /// Notifications: Briefs and Urgent Items. Coordinator: Automatic (a group's owner, a direct
    /// chat's bot) or one bot for every chat without an owner.
    func menuNeedsUpdate(_ menu: NSMenu) {
        menu.removeAllItems()
        let preferences = store.attention.preferences
        menu.addItem(.sectionHeader(title: L("Notifications")))
        for (title, isOn, key) in [
            (L("Briefs"), preferences.summaries, "summaries"), (L("Urgent Items"), preferences.urgentDirect, "urgent_direct"),
        ] {
            let item = NSMenuItem(title: title, action: #selector(toggleNotification(_:)), keyEquivalent: "")
            item.target = self
            item.state = isOn ? .on : .off
            item.representedObject = (key, !isOn)
            menu.addItem(item)
        }
        menu.addItem(.separator())
        menu.addItem(.sectionHeader(title: L("Coordinator")))
        let automatic = NSMenuItem(title: L("Automatic"), action: #selector(pickCoordinator(_:)), keyEquivalent: "")
        automatic.toolTip = L("A group’s owner, or the bot of a direct chat")
        automatic.target = self
        automatic.state = preferences.defaultCoordinatorBotId == nil ? .on : .off
        menu.addItem(automatic)
        for bot in store.bots {
            let item = NSMenuItem(title: bot.name, action: #selector(pickCoordinator(_:)), keyEquivalent: "")
            item.target = self
            item.representedObject = bot.id
            item.state = preferences.defaultCoordinatorBotId == bot.id ? .on : .off
            menu.addItem(item)
        }
    }

    /// A press from the keyboard or a screen reader; a click opens the menu on mouse-down.
    @objc private func showOptions(_ sender: Any?) {
        options.menu?.popUp(positioning: nil, at: NSPoint(x: 0, y: options.bounds.maxY), in: options)
    }

    @objc private func toggleNotification(_ sender: NSMenuItem) {
        guard let (key, isOn) = sender.representedObject as? (String, Bool) else { return }
        perform { try await $0.setAttentionPreference(key, isOn) }
    }

    @objc private func pickCoordinator(_ sender: NSMenuItem) {
        let id = sender.representedObject as? Bot.ID
        perform { try await $0.setAttentionPreference("default_coordinator_bot_id", id) }
    }

    /// What went wrong reaches the user in an alert on the main window.
    private func perform(_ change: @escaping (AppStore) async throws -> Void) {
        let window = view.window?.parent ?? NSApp.mainWindow
        Task { @MainActor in
            do {
                try await change(store)
            } catch {
                let alert = NSAlert()
                alert.messageText = L("Couldn’t update Attention")
                alert.informativeText = error.localizedDescription
                if let window { alert.beginSheetModal(for: window, completionHandler: nil) } else { alert.runModal() }
            }
        }
    }
}

/// A row of the Attention popover, lit under the pointer as a menu item is. A click runs
/// `onClick`; a right-click shows `contextMenu`.
final class AttentionRowView: NSView {
    var onClick: (() -> Void)?
    var contextMenu: (() -> NSMenu?)?
    private var isHovered = false { didSet { if isHovered != oldValue { needsDisplay = true } } }

    init(content: NSView) {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        content.translatesAutoresizingMaskIntoConstraints = false
        addSubview(content)
        NSLayoutConstraint.activate([
            content.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 16),
            content.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -16),
            content.topAnchor.constraint(equalTo: topAnchor, constant: 10),
            content.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -10),
        ])
        setAccessibilityElement(true)
        setAccessibilityRole(.button)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        for area in trackingAreas { removeTrackingArea(area) }
        addTrackingArea(NSTrackingArea(rect: .zero, options: [.mouseEnteredAndExited, .activeAlways, .inVisibleRect], owner: self))
    }

    override func mouseEntered(with event: NSEvent) { isHovered = onClick != nil }
    override func mouseExited(with event: NSEvent) { isHovered = false }

    // Claimed here, or the click goes to the view underneath.
    override func mouseDown(with event: NSEvent) {}

    override func mouseUp(with event: NSEvent) {
        guard bounds.contains(convert(event.locationInWindow, from: nil)) else { return }
        onClick?()
    }

    override func menu(for event: NSEvent) -> NSMenu? { contextMenu?() }

    override func accessibilityPerformPress() -> Bool {
        onClick?()
        return onClick != nil
    }

    override func draw(_ dirtyRect: NSRect) {
        guard isHovered else { return }
        NSColor.labelColor.withAlphaComponent(0.06).setFill()
        NSBezierPath(roundedRect: bounds.insetBy(dx: 6, dy: 1), xRadius: 6, yRadius: 6).fill()
    }
}
