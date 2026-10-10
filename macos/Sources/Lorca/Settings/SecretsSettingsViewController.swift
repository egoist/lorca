import AppKit

/// The secrets the picked Runner keeps for its bots: what each is, whose it is, and where it goes,
/// never its value. A row opens it to type a new value or delete it; its menu does either. The
/// list is the Runner's answer, through the relay when it is another Device, asked when the pane
/// shows a Runner and after each change made here.
final class SecretsSettingsViewController: DevicePaneViewController {
    private let section = SectionView(title: L("Secrets"))
    private var secrets: [SavedSecret]?
    private var failure: String?
    /// The Runner the list is of, and the latest ask, so an older answer never replaces it.
    private var listedRunner: Device.ID?
    private var asks = 0

    override func viewDidLoad() {
        title = L("Secrets")
        addSection(section)
        addFootnote(L("A bot asks in the chat when it needs a password or a key. What you save stays encrypted on its Runner, and the bot uses it by name without ever seeing it."))
        super.viewDidLoad()
    }

    override func viewWillAppear() {
        super.viewWillAppear()
        if let device, device.isRunner { load(device) }
    }

    override func reload() {
        section.title = device.map { L("Secrets on %@", $0.name) } ?? L("Secrets")
        if let rows = placeholderRows(for: device) {
            section.setRows(rows)
            return
        }
        guard let device else { return }
        if listedRunner != device.id {
            listedRunner = device.id
            secrets = nil
            failure = nil
            load(device)
        }
        render()
    }

    private func load(_ device: Device) {
        asks += 1
        let ask = asks
        Task { [weak self] in
            guard let self else { return }
            do {
                let secrets = try await store.secrets(on: device.id)
                guard ask == asks else { return }
                self.secrets = secrets
                failure = nil
            } catch {
                guard ask == asks else { return }
                failure = error.localizedDescription
            }
            render()
        }
    }

    private func render() {
        guard let device, device.isRunner else { return }
        guard let secrets else {
            section.setRows([failure.map { NoteRow(text: $0) } ?? KeyValueRow(key: L("Loading…"), value: "")])
            return
        }
        let rows: [NSView] = secrets.map { secret in
            let row = BotRow()
            let bot = store.bot(secret.botID)
            let owner = bot?.name ?? L("A deleted bot")
            row.configure(
                bot: bot ?? Bot(id: secret.botID, name: owner, description: "", symbolName: "lock", accent: .indigo, runnerID: device.id, provider: .deepseek, createdAt: secret.updatedAt),
                title: secret.label, detailText: "\(owner) · \(Self.place(of: secret))")
            row.onClick = { [weak self] in self?.open(secret) }
            row.menu = menu(for: secret)
            return row
        }
        section.setRows(rows.isEmpty ? [NoteRow(text: L("No secrets on %@ yet.", device.name))] : rows)
    }

    /// Where a secret goes: the site it is typed into, or the bot's commands.
    static func place(of secret: SavedSecret) -> String {
        switch secret.use {
        case .browser: secret.site ?? L("Browser")
        case .command, .plugin: L("Commands")
        }
    }

    private func menu(for secret: SavedSecret) -> NSMenu {
        let menu = NSMenu()
        let replace = NSMenuItem(title: L("Replace…"), action: #selector(replaceFromMenu(_:)), keyEquivalent: "")
        let delete = NSMenuItem(title: L("Delete…"), action: #selector(deleteFromMenu(_:)), keyEquivalent: "")
        for item in [replace, delete] {
            item.target = self
            item.representedObject = secret
        }
        menu.addItem(replace)
        menu.addItem(.separator())
        menu.addItem(delete)
        return menu
    }

    @objc private func replaceFromMenu(_ sender: NSMenuItem) {
        guard let secret = sender.representedObject as? SavedSecret else { return }
        open(secret)
    }

    @objc private func deleteFromMenu(_ sender: NSMenuItem) {
        guard let secret = sender.representedObject as? SavedSecret, let device, let window = view.window else { return }
        SecretViewController.confirmDelete(secret, botName: store.bot(secret.botID)?.name, on: device, in: window) { [weak self] in
            self?.load(device)
        }
    }

    private func open(_ secret: SavedSecret) {
        guard let device else { return }
        let sheet = SecretViewController(secret: secret, botName: store.bot(secret.botID)?.name, runner: device)
        sheet.onChange = { [weak self] in self?.load(device) }
        presentAsSheet(sheet)
    }
}
