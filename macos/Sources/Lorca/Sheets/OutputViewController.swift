import AppKit
import QuickLookUI

/// What can be done with an attachment or a published output: open the file in its app or the
/// link in the browser, and save a copy of the file. The file comes from the local CLI, fetched
/// from the relay when another Device sent it.
@MainActor
enum OutputActions {
    static func open(_ message: Message, window: NSWindow?) {
        if let link = message.output?.documentURL {
            NSWorkspace.shared.open(link)
        } else if let attachment = message.attachments.first {
            open(attachment, window: window)
        }
    }

    static func open(_ attachment: Attachment, window: NSWindow?) {
        Task {
            do {
                NSWorkspace.shared.open(try await AppStore.shared.openableURL(for: attachment))
            } catch {
                fail(error, window: window)
            }
        }
    }

    static func save(_ attachment: Attachment, window: NSWindow?) {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = attachment.name
        let finish: (NSApplication.ModalResponse) -> Void = { response in
            guard response == .OK, let target = panel.url else { return }
            Task {
                do {
                    let source = try await AppStore.shared.openableURL(for: attachment)
                    guard source.standardizedFileURL != target.standardizedFileURL else { return }
                    try Data(contentsOf: source, options: .alwaysMapped).write(to: target, options: .atomic)
                } catch {
                    fail(error, window: window)
                }
            }
        }
        if let window { panel.beginSheetModal(for: window, completionHandler: finish) } else { panel.begin(completionHandler: finish) }
    }

    /// Open, Save As…, or for a link Open Link and Copy Link: a row's menu.
    static func menu(for message: Message, window: @escaping () -> NSWindow?) -> NSMenu {
        let menu = NSMenu()
        if let link = message.output?.documentURL {
            menu.addItem(ClosureMenuItem(L("Open Link")) { NSWorkspace.shared.open(link) })
            menu.addItem(ClosureMenuItem(L("Copy Link")) {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(link.absoluteString, forType: .string)
            })
        } else if let attachment = message.attachments.first {
            menu.addItem(ClosureMenuItem(L("Open")) { open(message, window: window()) })
            menu.addItem(ClosureMenuItem(L("Save As…")) { save(attachment, window: window()) })
        }
        return menu
    }

    private static func fail(_ error: Error, window: NSWindow?) {
        let alert = NSAlert()
        alert.messageText = L("Couldn't get this file")
        alert.informativeText = error.localizedDescription
        if let window { alert.beginSheetModal(for: window) } else { alert.runModal() }
    }
}

/// A menu item that runs a closure, for menus built per row.
final class ClosureMenuItem: NSMenuItem {
    private let handler: () -> Void

    init(_ title: String, handler: @escaping () -> Void) {
        self.handler = handler
        super.init(title: title, action: #selector(run), keyEquivalent: "")
        target = self
    }

    @available(*, unavailable)
    required init(coder: NSCoder) { fatalError() }

    @objc private func run() { handler() }
}

extension StatusRow {
    /// An output's row: what it is, its name, when it was published (and by whom, in a group), and
    /// how its check went. The row opens the output; its menu has the file actions.
    func configure(output series: OutputSeries, showsBot: Bool) {
        let output = series.output
        var subtitle = Format.stamp(series.latest.createdAt)
        if showsBot, let bot = AppStore.shared.bot(output.botId) { subtitle = "\(bot.name) · \(subtitle)" }
        configure(
            symbol: output.symbolName,
            title: output.name,
            subtitle: subtitle,
            state: output.evidence?.statusText,
            stateColor: output.evidence?.failed == true ? .systemRed : .secondaryLabelColor)
        toolTip = output.name
        identifier = NSUserInterfaceItemIdentifier(series.id)
        menu = OutputActions.menu(for: series.latest) { [weak self] in self?.window }
    }
}

/// One output: a preview of its file, or its link; what the bot checked; and its earlier
/// versions, any of which the sheet can show. Open hands the file to its app or the link to the
/// browser, and Save As… copies the file.
final class OutputViewController: SheetViewController {
    private let store = AppStore.shared
    private let chatID: Chat.ID
    private let outputID: String
    /// The version on screen; nil follows the latest.
    private var picked: Message.ID?

    private let previewBox = BackgroundView()
    private let quickLook = QLPreviewView(frame: .zero, style: .compact)!
    private let placeholder = Build.label("", font: .systemFont(ofSize: 13), color: .secondaryLabelColor, lines: 0, alignment: .center)
    private let link = SectionView(title: L("Link"))
    private let evidence = SectionView(title: "")
    private let versions = SectionView(title: L("Versions"))
    private let saveButton = NSButton(title: L("Save As…"), target: nil, action: nil)
    /// What the sheet last showed, so a store event that changed nothing leaves it alone.
    private var shownState: [AnyHashable] = []
    /// The file the preview shows, once the CLI named it.
    private var previewURL: URL?

    init(chatID: Chat.ID, series: OutputSeries) {
        self.chatID = chatID
        outputID = series.id
        super.init(title: series.output.name, subtitle: "", width: 480)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    private var series: OutputSeries? { store.outputs(in: chatID).first { $0.id == outputID } }

    private var message: Message? {
        guard let series else { return nil }
        return series.versions.first { $0.id == picked } ?? series.latest
    }

    override func loadView() {
        super.loadView()
        previewBox.cornerRadius = 10
        previewBox.fillColor = Theme.botBubble
        previewBox.borderColor = Theme.botBubbleBorder
        previewBox.translatesAutoresizingMaskIntoConstraints = false
        quickLook.translatesAutoresizingMaskIntoConstraints = false
        quickLook.shouldCloseWithWindow = true
        previewBox.addSubview(quickLook)
        previewBox.addSubview(placeholder)
        quickLook.pin(to: previewBox, insets: NSEdgeInsets(top: 1, left: 1, bottom: 1, right: 1))

        saveButton.bezelStyle = .rounded
        saveButton.target = self
        saveButton.action = #selector(saveCopy)

        for view in [previewBox, link, evidence, versions] {
            contentStack.addArrangedSubview(view)
            view.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
        }
        contentStack.spacing = 18
        NSLayoutConstraint.activate([
            previewBox.heightAnchor.constraint(equalToConstant: 260),
            placeholder.centerYAnchor.constraint(equalTo: previewBox.centerYAnchor),
            placeholder.leadingAnchor.constraint(equalTo: previewBox.leadingAnchor, constant: 24),
            placeholder.trailingAnchor.constraint(equalTo: previewBox.trailingAnchor, constant: -24),
        ])
        setButtons(confirm: L("Open"), cancel: L("Done"), leading: saveButton)

        store.observe(self) { [weak self] event in
            guard let self else { return }
            switch event {
            case let .outputsChanged(id), let .messageChanged(id, _), let .chatChanged(id): if id == chatID { refresh() }
            case .rosterChanged, .snapshotReplaced: refresh()
            case .chatsChanged where store.chat(chatID) == nil: dismiss(nil)
            default: break
            }
        }
        refresh()
    }

    private func refresh() {
        guard let message, let output = message.output, let series else { return }
        let attachment = message.attachments.first
        let url = attachment.flatMap { store.localURL(for: $0, in: chatID, messageID: message.id) }
        let error = attachment.flatMap { store.attachmentError(for: $0) }
        let state: [AnyHashable] = [series.versions, message.id, url, error, store.bot(output.botId)?.name]
        guard state != shownState else { return }
        shownState = state

        setSheetTitle(output.name)
        let bot = store.bot(output.botId)?.name ?? L("A bot")
        setSheetSubtitle("\(bot) · \(Format.daySeparator(message.createdAt))")

        // A file shows in Quick Look once it is here; a link is its address.
        previewBox.isHidden = attachment == nil
        link.isHidden = output.documentURL == nil
        if let address = output.documentURL {
            let row = StatusRow()
            row.configure(symbol: "link", title: address.host ?? address.absoluteString, subtitle: address.absoluteString, state: nil)
            row.toolTip = address.absoluteString
            row.addGestureRecognizer(NSClickGestureRecognizer(target: self, action: #selector(confirmTapped)))
            link.setRows([row])
        }
        if let attachment {
            if let error {
                // The CLI's reason, as a sentence under the app's own words.
                let reason = error.prefix(1).uppercased() + error.dropFirst()
                showPreview(nil, placeholder: L("This file couldn't be downloaded."), detail: reason)
            } else if url == nil {
                showPreview(nil, placeholder: L("Downloading…"))
            } else {
                showPreview(attachment, placeholder: nil)
            }
        }
        confirmButton.title = error == nil ? L("Open") : L("Try Again")
        saveButton.isHidden = attachment == nil
        saveButton.isEnabled = url != nil

        evidence.isHidden = output.evidence == nil
        if let check = output.evidence {
            evidence.title = check.title
            var rows: [NSView] = [
                KeyValueRow(key: L("Result"), value: check.statusText, tint: check.failed ? .systemRed : nil),
                NoteRow(text: check.summary, tint: .labelColor),
            ]
            if let command = check.command { rows.append(KeyValueRow(key: L("Command"), value: command, monospaced: true)) }
            evidence.setRows(rows)
        }

        versions.isHidden = series.versions.count < 2
        if series.versions.count > 1 {
            versions.setRows(series.versions.map { version in
                let row = StatusRow()
                let status = version.output?.evidence.map { " · \($0.statusText)" } ?? ""
                // A check marks the version on screen.
                row.configure(
                    symbol: "checkmark", image: version.id == message.id ? nil : NSImage(),
                    title: L("Version %d", version.output?.version ?? 1),
                    subtitle: Format.daySeparator(version.createdAt) + status, state: nil)
                row.identifier = NSUserInterfaceItemIdentifier(version.id)
                row.addGestureRecognizer(NSClickGestureRecognizer(target: self, action: #selector(pickVersion(_:))))
                return row
            })
        }
        fitSheetToContent()
    }

    /// Quick Look takes the file under its own name, which the CLI makes on the first ask.
    private func showPreview(_ attachment: Attachment?, placeholder text: String?, detail: String? = nil) {
        let words = NSMutableAttributedString(string: text ?? "", attributes: [.font: NSFont.systemFont(ofSize: 13), .foregroundColor: NSColor.secondaryLabelColor])
        if let detail {
            words.append(NSAttributedString(string: "\n" + detail, attributes: [.font: NSFont.systemFont(ofSize: 11), .foregroundColor: NSColor.tertiaryLabelColor]))
        }
        let paragraph = NSMutableParagraphStyle()
        paragraph.alignment = .center
        paragraph.paragraphSpacingBefore = 0
        paragraph.lineSpacing = 3
        words.addAttribute(.paragraphStyle, value: paragraph, range: NSRange(location: 0, length: words.length))
        placeholder.attributedStringValue = words
        placeholder.isHidden = text == nil
        quickLook.isHidden = attachment == nil
        guard let attachment else {
            previewURL = nil
            quickLook.previewItem = nil
            return
        }
        Task { [weak self] in
            guard let url = try? await AppStore.shared.openableURL(for: attachment), let self, previewURL != url else { return }
            previewURL = url
            quickLook.previewItem = url as NSURL
        }
    }

    @objc private func pickVersion(_ sender: NSClickGestureRecognizer) {
        guard let id = sender.view?.identifier?.rawValue else { return }
        picked = id == series?.latest.id ? nil : id
        refresh()
    }

    @objc private func saveCopy() {
        guard let attachment = message?.attachments.first else { return }
        OutputActions.save(attachment, window: view.window)
    }

    /// Open, or Try Again when the file could not be fetched.
    override func confirmTapped() {
        guard let message else { return }
        if let attachment = message.attachments.first, store.attachmentError(for: attachment) != nil {
            store.retryAttachment(attachment, in: chatID, messageID: message.id)
            return
        }
        OutputActions.open(message, window: view.window)
        dismiss(nil)
    }
}

/// Every output of a chat, for the inspector's View all.
final class OutputsViewController: SheetViewController {
    private let store = AppStore.shared
    private let chatID: Chat.ID
    private let list = SectionView(title: "")
    private var shown: [OutputSeries] = []

    init(chatID: Chat.ID) {
        self.chatID = chatID
        super.init(title: L("Outputs"), subtitle: "", width: 440)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()
        let document = FlippedView()
        document.translatesAutoresizingMaskIntoConstraints = false
        document.addSubview(list)
        let scroll = NSScrollView()
        scroll.translatesAutoresizingMaskIntoConstraints = false
        scroll.drawsBackground = false
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.documentView = document
        contentStack.addArrangedSubview(scroll)
        // As tall as the list, up to a height that leaves the window in view.
        let fit = scroll.heightAnchor.constraint(equalTo: document.heightAnchor)
        fit.priority = .defaultLow
        NSLayoutConstraint.activate([
            scroll.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            scroll.heightAnchor.constraint(lessThanOrEqualToConstant: 440),
            fit,
            document.widthAnchor.constraint(equalTo: scroll.contentView.widthAnchor),
            list.topAnchor.constraint(equalTo: document.topAnchor),
            list.leadingAnchor.constraint(equalTo: document.leadingAnchor),
            list.trailingAnchor.constraint(equalTo: document.trailingAnchor),
            list.bottomAnchor.constraint(equalTo: document.bottomAnchor),
        ])
        setButtons(confirm: L("Done"), cancel: nil)
        store.observe(self) { [weak self] event in
            guard let self else { return }
            switch event {
            case let .outputsChanged(id): if id == chatID { refresh() }
            case .rosterChanged, .snapshotReplaced: refresh()
            case .chatsChanged where store.chat(chatID) == nil: dismiss(nil)
            default: break
            }
        }
        refresh()
    }

    private func refresh() {
        let all = store.outputs(in: chatID)
        guard all != shown else { return }
        shown = all
        let isGroup = store.chat(chatID)?.isGroup == true
        list.setRows(all.map { series in
            let row = StatusRow()
            row.configure(output: series, showsBot: isGroup)
            row.addGestureRecognizer(NSClickGestureRecognizer(target: self, action: #selector(openOutput(_:))))
            return row
        })
        fitSheetToContent()
    }

    @objc private func openOutput(_ sender: NSClickGestureRecognizer) {
        guard let id = sender.view?.identifier?.rawValue, let series = shown.first(where: { $0.id == id }) else { return }
        presentAsSheet(OutputViewController(chatID: chatID, series: series))
    }
}
