import AppKit

/// A draft or an exact call a bot left for the user, after the permission card it would have asked
/// with: who wants to do what and why, the command, call, or draft, editable, and one decision.
/// Approve runs what the editor shows on the bot's Runner, saving an edit first as a new version;
/// Reject drops it. A change from another Device shows here as it lands, unless the user is
/// editing, and then Approve offers the new version instead. Once decided, the sheet shows how it
/// went.
final class ReviewViewController: SheetViewController {
    private let store = AppStore.shared
    private var item: ReviewItem

    private let editor = NSTextView()
    private var editorHeight: NSLayoutConstraint!
    private var outputHeight: NSLayoutConstraint!
    private let details = SectionView(title: L("Details"))
    private let outputSection = SectionView(title: L("Output"))
    private let output = NSTextView()
    private let rejectButton = NSButton()
    private var busy = false

    init(item: ReviewItem) {
        self.item = item
        super.init(title: Self.title(for: item), subtitle: item.rationale, width: 520)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    /// "Chef wants to run a command on Workbench", as the permission card says it.
    private static func title(for item: ReviewItem) -> String {
        let store = AppStore.shared
        let bot = store.bot(item.botId)?.name ?? L("The bot")
        let runner = store.device(item.runnerId)
        switch item.payload.kind {
        case "draft":
            return L("%@ wrote a draft", bot)
        case "shell":
            return "\(bot) \(L("wants to run a command on %@", runner?.name ?? L("its Runner")))"
        default:
            return "\(bot) \(L("wants to use %@", store.pluginName(of: item)))"
        }
    }

    override func loadView() {
        super.loadView()

        // The editor is a field, as the memory sheet's is; what the bot said and what came of it
        // are cards, as the routine sheet's are.
        let scroll = Self.textBlock(editor, font: item.payload.isDraft ? .systemFont(ofSize: 13) : .monospacedSystemFont(ofSize: 12, weight: .regular))
        scroll.borderType = .bezelBorder
        scroll.drawsBackground = true
        editor.drawsBackground = true
        editor.backgroundColor = .textBackgroundColor
        editor.setAccessibilityLabel(item.payload.isDraft ? L("Draft") : (item.payload.isShell ? L("Command") : L("Arguments")))
        contentStack.addArrangedSubview(scroll)
        scroll.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
        let outputScroll = Self.textBlock(output, font: .monospacedSystemFont(ofSize: 11, weight: .regular))
        output.isEditable = false
        outputSection.setRows([outputScroll])

        for section in [details, outputSection] {
            contentStack.addArrangedSubview(section)
            section.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
        }
        editorHeight = scroll.heightAnchor.constraint(equalToConstant: 200)
        outputHeight = outputScroll.heightAnchor.constraint(equalToConstant: 120)
        NSLayoutConstraint.activate([editorHeight, outputHeight])

        // Rejecting is the one way to say no, so it stands apart from Cancel, which only closes.
        rejectButton.bezelStyle = .rounded
        rejectButton.hasDestructiveAction = true
        rejectButton.attributedTitle = NSAttributedString(
            string: L("Reject"), attributes: [.foregroundColor: NSColor.systemRed, .font: NSFont.systemFont(ofSize: NSFont.systemFontSize)])
        rejectButton.target = self
        rejectButton.action = #selector(rejectTapped)
        setButtons(confirm: L("Approve"), leading: rejectButton)
        display(item)
    }

    /// A text view in a card, scrolling inside a fixed height.
    private static func textBlock(_ text: NSTextView, font: NSFont) -> NSScrollView {
        text.isRichText = false
        text.font = font
        text.textColor = .labelColor
        text.drawsBackground = false
        text.isAutomaticQuoteSubstitutionEnabled = false
        text.isAutomaticDashSubstitutionEnabled = false
        text.isAutomaticTextReplacementEnabled = false
        text.allowsUndo = true
        text.textContainerInset = NSSize(width: 8, height: 8)
        text.isVerticallyResizable = true
        text.isHorizontallyResizable = false
        text.autoresizingMask = [.width]
        text.textContainer?.widthTracksTextView = true
        let scroll = NSScrollView()
        scroll.documentView = text
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.drawsBackground = false
        scroll.translatesAutoresizingMaskIntoConstraints = false
        return scroll
    }

    /// The height that shows all of `text`'s text, between `minimum` and `maximum`.
    private static func fittedHeight(_ text: NSTextView, width: CGFloat, minimum: CGFloat, maximum: CGFloat) -> CGFloat {
        guard let container = text.textContainer, let layout = text.layoutManager else { return minimum }
        container.containerSize = NSSize(width: width - text.textContainerInset.width * 2, height: .greatestFiniteMagnitude)
        layout.ensureLayout(for: container)
        let height = ceil(layout.usedRect(for: container).height) + text.textContainerInset.height * 2
        return min(max(height, minimum), maximum)
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        store.observe(self) { [weak self] event in
            switch event {
            case .reviewsChanged, .snapshotReplaced: self?.storeChanged()
            default: break
            }
        }
    }

    override func viewDidAppear() {
        super.viewDidAppear()
        if item.isPending { view.window?.makeFirstResponder(editor) }
    }

    /// Whether the editor holds something other than the version on screen.
    private var isEdited: Bool { editor.string != item.payload.editorText }

    /// The item as another Device or the Runner left it. An edit in progress keeps its version
    /// until Approve, which then offers the new one.
    private func storeChanged() {
        guard !busy else { return }
        guard let latest = store.review(item.id) else { return dismiss(nil) }
        guard latest.revision != item.revision else { return }
        if latest.isPending && latest.version == item.version || !(latest.isPending && isEdited) {
            display(latest, keepingEdits: latest.isPending && latest.version == item.version)
        }
    }

    private func display(_ item: ReviewItem, keepingEdits: Bool = false) {
        self.item = item
        if !keepingEdits { editor.string = item.payload.editorText }
        editor.isEditable = item.isPending && !busy
        // A command is usually a line; a draft or a call's arguments get room to edit.
        editorHeight.constant = item.payload.isShell ? Self.fittedHeight(editor, width: 480, minimum: 52, maximum: 140) : 200
        editor.drawsBackground = item.isPending

        var rows: [NSView] = []
        if let state = item.stateText {
            let attention = item.state == "uncertain" || item.state == "failed"
            rows.append(KeyValueRow(key: L("Status"), value: state, tint: attention ? .systemOrange : .secondaryLabelColor))
        }
        rows.append(KeyValueRow(key: L("Account"), value: item.target.account))
        rows.append(KeyValueRow(key: L("Resource"), value: item.target.resource))
        if !item.preconditions.files.isEmpty {
            let files = KeyValueRow(key: L("Files"), value: item.preconditions.files.map(\.path).joined(separator: "\n"), monospaced: true)
            files.toolTip = L("If these files change, this needs another review.")
            rows.append(files)
        }
        // Why it needs another look, or why it may have run without a result.
        if let note = item.outcome?.summary, item.isPending || item.state == "uncertain" {
            rows.append(NoteRow(text: note))
        }
        details.setRows(rows)

        outputSection.isHidden = item.output == nil
        output.string = item.output ?? ""
        outputHeight.constant = Self.fittedHeight(output, width: 480, minimum: 40, maximum: 160)

        rejectButton.isHidden = !item.isPending
        cancelButton?.isHidden = !item.isPending
        confirmButton.title = item.isPending ? L("Approve") : L("Done")
        setBusy(false)
        fitSheetToContent()
    }

    private func setBusy(_ value: Bool) {
        busy = value
        editor.isEditable = item.isPending && !value
        for button in [rejectButton, confirmButton, cancelButton].compactMap({ $0 }) { button.isEnabled = !value }
    }

    override func confirmTapped() {
        guard item.isPending else { return dismiss(nil) }
        guard !busy else { return }
        // Edited here while it changed elsewhere: approving would mean the wrong text.
        if let latest = store.review(item.id), latest.version != item.version, isEdited {
            return offerNewVersion(latest)
        }
        let edit: [String: Any]?
        do { edit = isEdited ? try item.payload.parameters(editedText: editor.string) : nil } catch {
            return showError(L("Couldn't approve"), error.localizedDescription)
        }
        decide(L("Couldn't approve")) { [store] item in try await store.approveReview(item, payload: edit) }
    }

    @objc private func rejectTapped() {
        guard item.isPending, !busy else { return }
        decide(L("Couldn't reject")) { [store] item in try await store.rejectReview(item) }
    }

    /// Sends the decision on the version shown. Done, the sheet closes; refused, it shows the item
    /// as it now is and says why, keeping an edit when the version is the same.
    private func decide(_ failure: String, _ request: @escaping (ReviewItem) async throws -> ReviewItem) {
        let shown = item
        setBusy(true)
        Task { [weak self] in
            do {
                _ = try await request(shown)
                self?.dismiss(nil)
            } catch {
                guard let self else { return }
                self.setBusy(false)
                if let latest = self.store.review(shown.id), latest.revision != shown.revision {
                    self.display(latest, keepingEdits: latest.isPending && latest.version == shown.version)
                }
                self.showError(failure, error.localizedDescription)
            }
        }
    }

    /// Another Device changed what the user is editing: show that version, or keep editing.
    private func offerNewVersion(_ latest: ReviewItem) {
        guard let window = view.window else { return }
        let alert = NSAlert()
        alert.messageText = L("This changed on another Device")
        alert.informativeText = L("Show Latest discards your changes and shows the current version to review.")
        alert.addButton(withTitle: L("Show Latest"))
        alert.addButton(withTitle: L("Cancel"))
        alert.beginSheetModal(for: window) { [weak self] response in
            guard let self, response == .alertFirstButtonReturn else { return }
            self.display(self.store.review(latest.id) ?? latest)
        }
    }

    private func showError(_ message: String, _ informative: String) {
        guard let window = view.window else { return }
        let alert = NSAlert()
        alert.messageText = message
        alert.informativeText = informative
        alert.beginSheetModal(for: window, completionHandler: nil)
    }
}
