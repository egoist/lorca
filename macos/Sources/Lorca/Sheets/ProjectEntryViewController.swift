import AppKit
import UniformTypeIdentifiers

/// One entry of a group's project context, opened from its row in the inspector or started
/// empty from the Project section's + menu: a title, the text, and the link it came from, or the
/// file it holds. Saving writes a new version that every bot in the group reads from its next
/// turn. A bot's suggestion is accepted by saving it, and an entry two Devices changed at once
/// keeps the version saved here. Remove… sits apart, at the leading edge.
final class ProjectEntryViewController: SheetViewController {
    private let store = AppStore.shared
    private let chatID: Chat.ID
    private let kind: ProjectEntry.Kind
    private var entry: ProjectEntry?
    private var otherVersions: [String]

    private let note = Build.label("", font: .systemFont(ofSize: 12), color: .systemOrange, lines: 0)
    private let titleField = NSTextField(string: "")
    private let textView = NSTextView()
    private let textScroll = NSScrollView()
    private let linkField = NSTextField(string: "")
    private let linkStatus = Build.label("", font: .systemFont(ofSize: 11), color: .secondaryLabelColor)
    private let checkButton = NSButton(title: L("Check Now"), target: nil, action: nil)
    private lazy var linkStatusRow = Build.stack([linkStatus, NSView(), checkButton], orientation: .horizontal, spacing: 8)
    private let problem = Build.label("", font: .systemFont(ofSize: 11), color: .systemOrange, lines: 0)
    private let removeButton = NSButton(title: "", target: nil, action: nil)
    private var saved = (title: "", text: "", link: "")
    private var busy = false
    private var shownLines: [Bool] = []

    /// An entry to open, or a new one of `kind`.
    init(chatID: Chat.ID, entry: ProjectEntry?, kind: ProjectEntry.Kind, otherVersions: [String] = []) {
        self.chatID = chatID
        self.entry = entry
        self.kind = entry?.kind ?? kind
        self.otherVersions = otherVersions
        super.init(
            title: entry == nil ? kind.newTitle : (entry?.kind ?? kind).title,
            subtitle: entry.map(Self.provenance) ?? L("Every bot in this group can read it."),
            width: 480)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    /// Who wrote the entry, or where it came from.
    private static func provenance(_ entry: ProjectEntry) -> String {
        let day = Format.formatter("MMMd").string(from: entry.updated)
        switch entry.source.kind {
        case "user": return L("Saved by you on %@", day)
        case "bot" where entry.isSuggestion: return L("Suggested by %@ on %@", entry.source.label, day)
        case "bot": return L("Added by %@ on %@", entry.source.label, day)
        case "url": return L("From %@", entry.host ?? entry.source.label)
        default: return L("From %@ on %@", entry.source.label, day)
        }
    }

    override func loadView() {
        super.loadView()

        titleField.placeholderString = L("Title")
        linkField.placeholderString = kind == .document ? "https://…" : L("Source link (optional)")
        for field in [titleField, linkField] {
            field.translatesAutoresizingMaskIntoConstraints = false
            field.delegate = self
        }

        textView.isRichText = false
        textView.font = .systemFont(ofSize: 13)
        textView.textColor = .labelColor
        textView.allowsUndo = true
        textView.isAutomaticQuoteSubstitutionEnabled = false
        textView.isAutomaticDashSubstitutionEnabled = false
        textView.textContainerInset = NSSize(width: 6, height: 8)
        textView.isVerticallyResizable = true
        textView.isHorizontallyResizable = false
        textView.autoresizingMask = [.width]
        textView.textContainer?.widthTracksTextView = true
        textView.delegate = self
        textScroll.documentView = textView
        textScroll.hasVerticalScroller = true
        textScroll.autohidesScrollers = true
        textScroll.borderType = .bezelBorder
        textScroll.translatesAutoresizingMaskIntoConstraints = false

        checkButton.bezelStyle = .rounded
        checkButton.controlSize = .small
        checkButton.font = .systemFont(ofSize: NSFont.systemFontSize(for: .small))
        checkButton.target = self
        checkButton.action = #selector(checkLink)
        linkStatus.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)

        // A link leads with its address; a note's source link follows the text it backs; a file
        // shows the file.
        var views: [NSView] = [note, titleField]
        switch kind {
        case .asset: views += (entry?.asset).map { [fileRow($0), textScroll] } ?? [textScroll]
        case .document: views += [linkField, linkStatusRow, textScroll]
        default: views += [textScroll, linkField, linkStatusRow]
        }
        views.append(problem)
        for view in views {
            contentStack.addArrangedSubview(view)
            view.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
        }
        textScroll.heightAnchor.constraint(equalToConstant: kind == .brief || kind == .decision || kind == .goal || kind == .constraint || kind == .fact ? 150 : 96).isActive = true

        // Removing sits apart from saving, red, at the leading edge.
        if entry != nil {
            removeButton.bezelStyle = .rounded
            removeButton.attributedTitle = NSAttributedString(
                string: L("Remove…"), attributes: [.foregroundColor: NSColor.systemRed, .font: NSFont.systemFont(ofSize: NSFont.systemFontSize)])
            removeButton.target = self
            removeButton.action = #selector(confirmRemove)
        }
        setButtons(confirm: confirmTitle, leading: entry == nil ? nil : removeButton)
        show(entry)
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        // A group deleted here or on another Device takes its context along.
        store.observe(self) { [weak self] event in
            guard let self, case .chatsChanged = event, self.store.chat(self.chatID) == nil else { return }
            self.dismiss(nil)
        }
    }

    override func viewDidAppear() {
        super.viewDidAppear()
        view.window?.makeFirstResponder(entry == nil ? titleField : textView)
    }

    /// A file entry's file: its name and size, and Open.
    private func fileRow(_ asset: ProjectEntry.Asset) -> NSView {
        let icon = NSImageView(image: NSWorkspace.shared.icon(for: UTType(filenameExtension: (asset.name as NSString).pathExtension) ?? .data))
        icon.translatesAutoresizingMaskIntoConstraints = false
        icon.widthAnchor.constraint(equalToConstant: 32).isActive = true
        icon.heightAnchor.constraint(equalToConstant: 32).isActive = true
        let name = Build.label(asset.name, font: .systemFont(ofSize: 13, weight: .medium))
        let size = Build.label(
            asset.size.map { ByteCountFormatter.string(fromByteCount: $0, countStyle: .file) } ?? "",
            font: .systemFont(ofSize: 11), color: .secondaryLabelColor)
        let open = NSButton(title: L("Open"), target: self, action: #selector(openFile))
        open.bezelStyle = .rounded
        open.setContentHuggingPriority(.required, for: .horizontal)
        return Build.stack([icon, Build.stack([name, size], spacing: 1), NSView(), open], orientation: .horizontal, spacing: 10)
    }

    /// Fills the form from `entry`, which becomes what the form compares the edits against.
    private func show(_ entry: ProjectEntry?) {
        self.entry = entry
        saved = (entry?.title ?? "", entry?.text ?? "", entry?.source.url ?? "")
        titleField.stringValue = saved.title
        textView.string = saved.text
        linkField.stringValue = saved.link
        update()
    }

    private var confirmTitle: String {
        guard let entry else { return L("Add") }
        if !otherVersions.isEmpty { return L("Keep This Version") }
        return entry.isSuggestion ? L("Accept") : L("Save")
    }

    private var link: String { linkField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines) }
    private var isEdited: Bool {
        titleField.stringValue != saved.title || textView.string != saved.text || link != saved.link
    }

    /// Why the entry can't be saved as it stands, in words for the form; nil when it can.
    private var whatStopsSaving: String? {
        if textView.string.utf8.count > 32_000 { return L("This is too long for one entry. Split it into a few.") }
        if titleField.stringValue.utf8.count > 240 { return L("The title is too long.") }
        if !link.isEmpty, URL(string: link)?.scheme != "https" || URL(string: link)?.host == nil {
            return L("Links start with https://.")
        }
        return nil
    }

    /// The buttons, the note, and the link's state, from what the form holds now.
    private func update() {
        guard isViewLoaded else { return }
        if !otherVersions.isEmpty {
            note.stringValue = L("Two Devices changed this at the same time. Keep This Version saves this one and replaces the other.")
        }
        note.isHidden = otherVersions.isEmpty
        let stops = whatStopsSaving
        problem.stringValue = stops ?? ""
        problem.isHidden = stops == nil
        let title = titleField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        let needsLink = kind == .document && link.isEmpty
        let changes = entry == nil || isEdited || entry?.isSuggestion == true || !otherVersions.isEmpty
        confirmButton.title = confirmTitle
        confirmButton.isEnabled = !busy && !title.isEmpty && !needsLink && stops == nil && changes
        removeButton.isEnabled = !busy

        // The link as the CLI last read it, for an entry that has one.
        if let entry, let url = entry.source.url, url == link {
            linkStatusRow.isHidden = false
            if entry.freshness == "unavailable" {
                linkStatus.stringValue = L("Couldn't open this link. Bots use the copy from before.")
                linkStatus.textColor = .systemOrange
                linkStatus.toolTip = entry.refreshError
            } else {
                linkStatus.stringValue = entry.checked.map { L("Checked %@", Format.formatter("MMMdjmm").string(from: $0)) } ?? L("Not checked yet")
                linkStatus.textColor = .secondaryLabelColor
                linkStatus.toolTip = nil
            }
            checkButton.isHidden = !entry.canCheckLink
            checkButton.isEnabled = !busy && !isEdited
        } else {
            linkStatusRow.isHidden = true
        }
        // The link's state sits close under it.
        if kind != .asset { contentStack.setCustomSpacing(linkStatusRow.isHidden ? contentStack.spacing : 6, after: linkField) }
        // The sheet only grows by itself; it shrinks back when a line goes away.
        let shown = [note, problem, linkStatusRow].map(\.isHidden)
        if shown != shownLines {
            shownLines = shown
            fitSheetToContent()
        }
    }

    override func confirmTapped() {
        save(replacing: entry)
    }

    private func save(replacing base: ProjectEntry?) {
        busy = true
        update()
        let (title, text, link) = (titleField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines), textView.string, link)
        Task { [weak self] in
            guard let self else { return }
            do {
                try await self.store.saveProjectEntry(
                    in: self.chatID, kind: self.kind, title: title, text: text, link: link.isEmpty ? nil : link,
                    replacing: base, alsoReplacing: base == nil ? [] : self.otherVersions)
                self.dismiss(nil)
            } catch {
                self.busy = false
                self.update()
                if let base, error.localizedDescription == staleEntry {
                    await self.resolveConflict(with: base)
                } else {
                    self.alert(L("Couldn't save this entry"), error.localizedDescription)
                }
            }
        }
    }

    /// Another Device changed or removed the entry while it was open here: show its version, or
    /// save these edits over it; a removed one can be added back.
    private func resolveConflict(with base: ProjectEntry) async {
        guard let window = view.window else { return }
        let newer = try? await store.currentProjectEntry(after: base.id, in: chatID)
        let alert = NSAlert()
        if let newer {
            alert.messageText = L("This entry changed on another Device")
            alert.informativeText = L("Reload shows the other version and discards your edits. Overwrite saves yours over it.")
            alert.addButton(withTitle: L("Reload"))
            alert.addButton(withTitle: L("Overwrite with mine"))
        } else {
            alert.messageText = L("This entry was removed on another Device")
            alert.informativeText = L("Add it back with your edits, or cancel to leave it removed.")
            alert.addButton(withTitle: L("Add It Back"))
        }
        alert.addButton(withTitle: L("Cancel"))
        let response = await alert.beginSheetModal(for: window)
        otherVersions = []
        switch (response, newer) {
        case (.alertFirstButtonReturn, let newer?): show(newer)
        case (.alertSecondButtonReturn, let newer?): save(replacing: newer)
        case (.alertFirstButtonReturn, nil): save(replacing: nil)
        default: break
        }
    }

    @objc private func checkLink() {
        guard let entry else { return }
        busy = true
        update()
        Task { [weak self] in
            guard let self else { return }
            do {
                if let fresh = try await self.store.checkProjectLink(entry.id, in: self.chatID) { self.show(fresh) }
            } catch {
                self.alert(L("Couldn't check this link"), error.localizedDescription)
            }
            self.busy = false
            self.update()
        }
    }

    @objc private func openFile() {
        guard let entry, let asset = entry.asset else { return }
        Task { [weak self] in
            guard let self else { return }
            do {
                NSWorkspace.shared.open(try await self.store.projectFile(entry.id, in: self.chatID))
            } catch {
                self.alert(L("Couldn't open “%@”", asset.name), L("The file hasn't reached this Device yet. Try again when the Device that added it is online."))
            }
        }
    }

    @objc private func confirmRemove() {
        guard let entry, let window = view.window else { return }
        let alert = NSAlert()
        alert.messageText = L("Remove “%@”?", entry.title)
        alert.informativeText = L("Bots in this group stop seeing it.")
        alert.addButton(withTitle: L("Remove"))
        alert.addButton(withTitle: L("Cancel"))
        alert.buttons.first?.hasDestructiveAction = true
        alert.beginSheetModal(for: window) { [weak self] response in
            guard let self, response == .alertFirstButtonReturn else { return }
            Task { [weak self] in
                guard let self else { return }
                do {
                    try await self.store.removeProjectEntry(entry, in: self.chatID)
                    self.dismiss(nil)
                } catch {
                    let changed = error.localizedDescription == staleEntry
                    self.alert(changed ? L("This entry changed on another Device") : L("Couldn't remove this entry"), changed ? L("Close it and open it again to see the other version.") : error.localizedDescription)
                }
            }
        }
    }

    private func alert(_ message: String, _ information: String) {
        guard let window = view.window else { return }
        let alert = NSAlert()
        alert.messageText = message
        alert.informativeText = information
        alert.beginSheetModal(for: window, completionHandler: nil)
    }
}

/// What the CLI answers when the entry being saved changed on another Device first.
private let staleEntry = "This entry changed on another Device."

extension ProjectEntryViewController: NSTextFieldDelegate, NSTextViewDelegate {
    func controlTextDidChange(_ obj: Notification) { update() }
    func textDidChange(_ notification: Notification) { update() }
}
