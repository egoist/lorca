import AppKit
import UniformTypeIdentifiers

extension UTType {
    static let lorcaTemplate = UTType(filenameExtension: "lorca-template") ?? .json
}

/// Shares a bot as a template: the user picks what goes in it besides the profile (routines,
/// plugins, memories), sees each as the template will hold it, and shares a link, which the CLI
/// puts on the relay encrypted with a key only the link carries, or saves a file. A bot shared
/// before updates its link, which keeps its address.
final class TemplateShareViewController: SheetViewController {
    private let bot: Bot
    private let reply: TemplateReply
    /// The link the bot was shared as, which Update Link replaces what is behind.
    private let link: SharedLink?
    private let list = TemplateItemList(maxHeight: 380)
    private let linkBox = LinkBox()
    private lazy var saveButton: NSButton = {
        let button = NSButton(title: L("Save as File…"), target: self, action: #selector(saveFile))
        button.bezelStyle = .rounded
        return button
    }()
    /// The link is out: the sheet shows it, and Done closes it.
    private var isShared = false
    private let status = Build.label("", font: .systemFont(ofSize: 11.5), color: .secondaryLabelColor, lines: 0)
    private var rows: [String: TemplateItemRow] = [:]
    /// What is picked, by kind: `skill_ids`, `memory_ids`, `routine_ids`, `requirement_ids`. The
    /// profile is what makes a template a bot, so it is always in the file.
    private var picked: [String: [String]] = [:]
    private var order: [String: [String]] = [:]
    private var isBusy = false
    private var isLoaded = false
    /// Select All in the header of each long list, by kind.
    private var selectAll: [String: NSButton] = [:]

    init(bot: Bot, reply: TemplateReply? = nil) {
        self.bot = bot
        self.reply = reply ?? { method, params in try await AppStore.shared.templateReply(method, params) }
        link = AppStore.shared.sharedLink(for: bot.id)
        super.init(title: L("Share “%@”", bot.name),
            subtitle: link == nil
                ? L("Others get a copy of what you pick. Keys, sign-ins, and chats stay.")
                : L("Update the link with what you pick now. Its address stays the same."),
            width: 480)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()
        for view in [linkBox, list, status] as [NSView] {
            contentStack.addArrangedSubview(view)
            view.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
        }
        linkBox.url = link?.url ?? ""
        linkBox.isHidden = link == nil
        contentStack.setCustomSpacing(16, after: linkBox)
        setButtons(confirm: link == nil ? L("Share Link") : L("Update Link"), leading: saveButton)
        confirmButton.isEnabled = false
        saveButton.isEnabled = false
        show(status: L("Loading…"))
        Task { [weak self] in
            guard let self else { return }
            do {
                let json = try await self.reply("templates.contents", ["bot_id": self.bot.id])
                let routines = AppStore.shared.routines(for: self.bot.id)
                self.show(TemplateContents(json: json) { id in routines.first { $0.id == id }?.scheduleText })
            } catch {
                self.show(status: error.localizedDescription, color: .systemRed)
            }
        }
    }

    private func show(_ contents: TemplateContents) {
        func rows(_ key: String, _ items: [TemplateItem], make: (TemplateItem) -> TemplateItemRow) -> [NSView] {
            order[key] = items.map(\.id)
            return items.map { item in
                let row = make(item)
                row.onToggle = { [weak self] in self?.toggle(item.id, in: key) }
                self.rows[key + ":" + item.id] = row
                return row
            }
        }
        let plugins = contents.plugins.map { TemplateItem(id: $0.id, title: $0.name, detail: "", flags: []) }
        // Memories come last: they are the longest list, and the most personal.
        let sections: [(key: String, title: String, rows: [NSView])] = [
            ("profile", L("Profile"), [TemplateItemRow.profile(contents.profile, isSelectable: false)]),
            ("skill_ids", L("Skills"), rows("skill_ids", contents.skills) { TemplateItemRow(item: $0, isSelectable: true) }),
            ("routine_ids", L("Routines"), rows("routine_ids", contents.routines) { TemplateItemRow(item: $0, isSelectable: true) }),
            ("requirement_ids", L("Plugins"), rows("requirement_ids", plugins) { .plugin(id: $0.id, name: $0.title, isSelectable: true) }),
            ("memory_ids", L("Memories"), rows("memory_ids", contents.memories) { TemplateItemRow(item: $0, isSelectable: true, titleLines: 3) }),
        ]
        // A long list picks all at once, whichever kind it is.
        var accessories: [String: NSView] = [:]
        for section in sections where section.key != "profile" && section.rows.count >= Self.longList {
            let button = NSButton(title: "", target: self, action: #selector(toggleAll(_:)))
            button.isBordered = false
            button.identifier = NSUserInterfaceItemIdentifier(section.key)
            selectAll[section.key] = button
            accessories[section.title] = button
        }
        list.setSections(sections.map { ($0.title, $0.rows) }, accessories: accessories)
        // An update starts from what the link holds, less what the bot no longer has.
        if let selection = link?.selection {
            for (key, ids) in [("skill_ids", selection.skillIds), ("routine_ids", selection.routineIds), ("requirement_ids", selection.requirementIds), ("memory_ids", selection.memoryIds)] {
                picked[key] = (order[key] ?? []).filter(ids.contains)
                for id in picked[key] ?? [] { self.rows[key + ":" + id]?.isSelected = true }
            }
        }
        isLoaded = true
        showPicked()
        show(status: "")
    }

    /// How many items a list has before its header offers Select All.
    static let longList = 6

    private func toggle(_ id: String, in key: String) {
        guard !isBusy else { return }
        var ids = picked[key] ?? []
        if let index = ids.firstIndex(of: id) { ids.remove(at: index) } else { ids.append(id) }
        // The file lists things in the bot's order, whatever order they were picked in.
        picked[key] = (order[key] ?? []).filter(ids.contains)
        rows[key + ":" + id]?.isSelected = picked[key]?.contains(id) == true
        showPicked()
    }

    @objc private func toggleAll(_ sender: NSButton) {
        guard !isBusy, let key = sender.identifier?.rawValue else { return }
        let all = order[key] ?? []
        picked[key] = picked[key] == all ? [] : all
        for id in all { rows[key + ":" + id]?.isSelected = picked[key] == all }
        showPicked()
    }

    private func showPicked() {
        for (key, button) in selectAll {
            button.attributedTitle = NSAttributedString(
                string: picked[key] == order[key] ? L("Deselect All") : L("Select All"),
                attributes: [.font: NSFont.systemFont(ofSize: 11), .foregroundColor: NSColor.secondaryLabelColor])
        }
        confirmButton.isEnabled = isLoaded && !isBusy
        saveButton.isEnabled = isLoaded && !isBusy
        if status.textColor == .systemRed { show(status: "") }
    }

    private var selection: [String: Any] {
        var selection: [String: Any] = ["profile": true]
        for key in ["skill_ids", "memory_ids", "routine_ids", "requirement_ids"] { selection[key] = picked[key] ?? [] }
        return selection
    }

    private func show(status text: String, color: NSColor = .secondaryLabelColor) {
        status.stringValue = text
        status.textColor = color
        status.isHidden = text.isEmpty
        fitSheetToContent()
    }

    /// Builds the template first, so the CLI's checks speak before anything leaves, then shares
    /// what was shown: the CLI refuses it if the contents changed since.
    override func confirmTapped() {
        if isShared {
            dismiss(nil)
            return
        }
        guard !isBusy else { return }
        let selection = selection
        setBusy(true)
        Task { [weak self] in
            guard let self else { return }
            do {
                let digest = try await self.preview(selection)
                var params: [String: Any] = ["bot_id": self.bot.id, "selection": selection, "expected_digest": digest, "reviewed": true]
                if let link = self.link { params["link_id"] = link.id }
                let reply = try await self.reply("templates.share", params)
                guard let url = (reply["link"] as? [String: Any])?["url"] as? String else { throw CLIClient.RequestError(message: L("Couldn't read the CLI's answer.")) }
                self.showShared(url)
            } catch {
                self.setBusy(false)
                self.show(status: error.localizedDescription, color: .systemRed)
            }
        }
    }

    /// The link, copied already, and what it does.
    private func showShared(_ url: String) {
        isShared = true
        list.isHidden = true
        status.isHidden = true
        linkBox.url = url
        linkBox.isHidden = false
        linkBox.copyLink()
        setSheetSubtitle(link == nil
            ? L("Anyone with this link can add their own copy of %@. Revoke it in Settings › Shared Links.", bot.name)
            : L("The link now holds what you picked. Anyone who opens it gets this version."))
        setButtons(confirm: L("Done"), cancel: nil)
        fitSheetToContent()
    }

    /// The same template as a file, wherever the user saves it.
    @objc private func saveFile() {
        guard !isBusy, let window = view.window else { return }
        let selection = selection
        setBusy(true)
        Task { [weak self] in
            guard let self else { return }
            do {
                let digest = try await self.preview(selection)
                let panel = NSSavePanel()
                panel.allowedContentTypes = [.lorcaTemplate]
                panel.nameFieldStringValue = "\(self.bot.name).lorca-template"
                guard await panel.beginSheetModal(for: window) == .OK, let url = panel.url else {
                    self.setBusy(false)
                    return
                }
                _ = try await self.reply("templates.export", ["bot_id": self.bot.id, "selection": selection, "path": url.path,
                    "expected_digest": digest, "reviewed": true, "overwrite": FileManager.default.fileExists(atPath: url.path)])
                self.dismiss(nil)
            } catch {
                self.setBusy(false)
                self.show(status: error.localizedDescription, color: .systemRed)
            }
        }
    }

    private func preview(_ selection: [String: Any]) async throws -> String {
        let preview = try await reply("templates.export.preview", ["bot_id": bot.id, "selection": selection])
        guard let digest = preview["digest"] as? String else { throw CLIClient.RequestError(message: L("Couldn't read the CLI's answer.")) }
        return digest
    }

    private func setBusy(_ busy: Bool) {
        isBusy = busy
        confirmButton.isEnabled = !busy && isLoaded
        saveButton.isEnabled = !busy && isLoaded
    }
}
