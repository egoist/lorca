import AppKit
import UniformTypeIdentifiers

extension UTType {
    static let lorcaTemplate = UTType(filenameExtension: "lorca-template") ?? .json
}

/// Exports a bot as a template file: the user picks its profile, memories, routines, and the
/// plugins it needs, sees each as the file will hold it, and saves the file wherever they like.
/// The CLI writes the file; saving publishes nothing.
final class TemplateExportViewController: SheetViewController {
    private let bot: Bot
    private let reply: TemplateReply
    private let list = TemplateItemList(maxHeight: 380)
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
        super.init(title: L("Export “%@”", bot.name),
            subtitle: L("Pick what goes in the template. Keys, sign-ins, and chats never do."), width: 480)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()
        for view in [list, status] as [NSView] {
            contentStack.addArrangedSubview(view)
            view.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
        }
        setButtons(confirm: L("Export…"))
        confirmButton.isEnabled = false
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

    /// Builds the file's contents first, so the CLI's checks speak before the Save panel opens,
    /// then writes what was shown: the CLI refuses the save if the contents changed since.
    override func confirmTapped() {
        guard !isBusy, let window = view.window else { return }
        let selection = selection
        setBusy(true)
        Task { [weak self] in
            guard let self else { return }
            do {
                let preview = try await self.reply("templates.export.preview", ["bot_id": self.bot.id, "selection": selection])
                guard let digest = preview["digest"] as? String else { throw CLIClient.RequestError(message: L("Couldn't read the CLI's answer.")) }
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

    private func setBusy(_ busy: Bool) {
        isBusy = busy
        confirmButton.isEnabled = !busy && isLoaded
    }
}
