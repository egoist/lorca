import AppKit

/// The bots the account shares as links, from the roster, so every Device lists the same ones.
/// A row opens its bot's Share sheet, where Update Link keeps the address; its menu copies the
/// link or revokes it.
final class SharedLinksSettingsViewController: SettingsPaneViewController {
    private let store = AppStore.shared
    private let links = SectionView(title: L("Shared Links"))

    override func viewDidLoad() {
        super.viewDidLoad()
        title = L("Shared Links")
        addSection(links)
        addFootnote(L("Anyone with a link can add their own copy of the bot. Revoking a link stops it working; copies already added stay."))
        store.observe(self) { [weak self] event in
            switch event {
            case .rosterChanged, .snapshotReplaced: self?.render()
            default: break
            }
        }
        render()
    }

    private func render() {
        let rows: [NSView] = store.sharedLinks.reversed().map { link in
            let row = BotRow()
            let bot = store.bot(link.botId)
            // A deleted bot's link still works until it is revoked.
            row.configure(bot: bot ?? Bot(id: link.botId, name: link.name, description: "", symbolName: "link", accent: .indigo,
                runnerID: "", provider: .deepseek, createdAt: link.updated), detailText: L("Updated %@", Format.stamp(link.updated)))
            row.menu = menu(for: link)
            row.onClick = bot.map { bot in { [weak self] in self?.presentAsSheet(TemplateShareViewController(bot: bot)) } }
            row.toolTip = link.url
            return row
        }
        links.setRows(rows.isEmpty ? [NoteRow(text: L("No shared links yet. Share a bot from its chat's menu or File › Share as Template…"))] : rows)
    }

    private func menu(for link: SharedLink) -> NSMenu {
        let menu = NSMenu()
        let copy = NSMenuItem(title: L("Copy Link"), action: #selector(copyLink(_:)), keyEquivalent: "")
        let revoke = NSMenuItem(title: L("Revoke Link…"), action: #selector(revokeLink(_:)), keyEquivalent: "")
        for item in [copy, revoke] {
            item.target = self
            item.representedObject = link
        }
        menu.addItem(copy)
        menu.addItem(.separator())
        menu.addItem(revoke)
        return menu
    }

    @objc private func copyLink(_ sender: NSMenuItem) {
        guard let link = sender.representedObject as? SharedLink else { return }
        LinkBox.pasteboard.clearContents()
        LinkBox.pasteboard.setString(link.url, forType: .string)
    }

    @objc private func revokeLink(_ sender: NSMenuItem) {
        guard let link = sender.representedObject as? SharedLink, let window = view.window else { return }
        let alert = NSAlert()
        alert.messageText = L("Revoke the link to “%@”?", link.name)
        alert.informativeText = L("Whoever opens it sees that it no longer works. Bots already added from it stay.")
        alert.addButton(withTitle: L("Revoke"))
        alert.addButton(withTitle: L("Cancel"))
        alert.buttons.first?.hasDestructiveAction = true
        alert.beginSheetModal(for: window) { [weak self] response in
            guard response == .alertFirstButtonReturn, let self else { return }
            Task {
                do {
                    try await self.store.revokeLink(link.id)
                } catch {
                    let failed = NSAlert()
                    failed.messageText = L("Couldn't revoke the link")
                    failed.informativeText = error.localizedDescription
                    _ = await failed.beginSheetModal(for: window)
                }
            }
        }
    }
}
