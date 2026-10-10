import AppKit

/// A coding agent's transcript, opened from its card: what it was sent, what it said, and each
/// command and edit, or what its pane shows when it runs in a terminal host. It follows the
/// agent while the sheet is up, asking its Runner again every two seconds, five through the
/// relay, and keeps to the end unless the user scrolled up to read. On the Runner itself, an
/// agent in a terminal host offers to show its pane there.
final class AgentTranscriptViewController: SheetViewController {
    private static let font = NSFont.monospacedSystemFont(ofSize: 11.5, weight: .regular)
    private static let inset = NSSize(width: 10, height: 9)

    private let store = AppStore.shared
    private let chatID: Chat.ID
    private let messageID: Message.ID
    private let scroll = NSTextView.scrollableTextView()
    private let placeholder = Build.label("", font: .systemFont(ofSize: 12), color: .secondaryLabelColor)
    private let showButton = NSButton()
    private var refresh: Task<Void, Never>?

    init(chatID: Chat.ID, messageID: Message.ID, agent: AgentRun) {
        self.chatID = chatID
        self.messageID = messageID
        super.init(title: agent.place.isEmpty ? agent.name : "\(agent.name) · \(agent.place)", subtitle: "", width: 640)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    private var textView: NSTextView? { scroll.documentView as? NSTextView }

    override func loadView() {
        super.loadView()
        scroll.drawsBackground = false
        scroll.borderType = .noBorder
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.translatesAutoresizingMaskIntoConstraints = false
        if let text = textView {
            text.font = Self.font
            text.textColor = .labelColor
            text.isEditable = false
            text.isSelectable = true
            text.drawsBackground = false
            text.textContainerInset = Self.inset
        }
        let box = BackgroundView()
        box.fillColor = Theme.codeBackground
        box.cornerRadius = 8
        box.addSubview(scroll)
        box.addSubview(placeholder.framePositioned())
        placeholder.stringValue = L("Loading…")
        placeholder.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            scroll.topAnchor.constraint(equalTo: box.topAnchor),
            scroll.leadingAnchor.constraint(equalTo: box.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: box.trailingAnchor),
            scroll.bottomAnchor.constraint(equalTo: box.bottomAnchor),
            placeholder.centerXAnchor.constraint(equalTo: box.centerXAnchor),
            placeholder.centerYAnchor.constraint(equalTo: box.centerYAnchor),
            box.heightAnchor.constraint(equalToConstant: 420),
        ])
        contentStack.addArrangedSubview(box)
        box.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true

        showButton.bezelStyle = .rounded
        showButton.target = self
        showButton.action = #selector(showPane)
        let message = store.chat(chatID)?.messages.first { $0.id == messageID }
        if case let .tool(tool)? = message?.body, let agent = tool.agent, let host = agent.hostName, agent.isRunning, let message, store.runsHere(message) {
            showButton.title = L("Show in %@", host)
            setButtons(confirm: L("Done"), cancel: nil, leading: showButton)
        } else {
            setButtons(confirm: L("Done"), cancel: nil)
        }
    }

    override func viewDidAppear() {
        super.viewDidAppear()
        refresh?.cancel()
        let remote = store.chat(chatID)?.messages.first { $0.id == messageID }.map { !store.runsHere($0) } ?? false
        refresh = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                await self?.load()
                try? await Task.sleep(for: .seconds(remote ? 5 : 2))
            }
        }
    }

    override func viewWillDisappear() {
        super.viewWillDisappear()
        refresh?.cancel()
        refresh = nil
    }

    private func load() async {
        do {
            let text = try await store.agentTranscript(chatID: chatID, messageID: messageID)
            show(text.isEmpty ? nil : text, error: nil)
        } catch {
            // What was shown stays; the next try may reach the Runner.
            if textView?.string.isEmpty ?? true { show(nil, error: error.localizedDescription) }
        }
    }

    private func show(_ text: String?, error: String?) {
        placeholder.isHidden = text != nil
        placeholder.stringValue = error ?? L("Nothing yet")
        guard let textView, let text, textView.string != text else { return }
        // Keeps to the end unless the user scrolled up to read.
        let clip = scroll.contentView
        let atEnd = clip.bounds.maxY >= (scroll.documentView?.frame.height ?? 0) - 24
        textView.string = text
        if atEnd || clip.bounds.minY == 0 { textView.scrollToEndOfDocument(nil) }
    }

    @objc private func showPane() {
        Task { @MainActor in
            do {
                try await store.showAgent(chatID: chatID, messageID: messageID)
            } catch {
                guard let window = view.window else { return }
                let alert = NSAlert()
                alert.messageText = L("Could not show it")
                alert.informativeText = error.localizedDescription
                alert.beginSheetModal(for: window) { _ in }
            }
        }
    }
}
