import AppKit

/// The account's email address, which every bot shares: getting one, copying, changing, and
/// giving it up. Listed while the relay offers email; the CLI asks the relay when it shows.
final class EmailSettingsViewController: SettingsPaneViewController {
    private let store = AppStore.shared
    private let section = SectionView(title: SettingsEntry.emailAddress.title)
    private let footnote = Build.label("", font: Theme.Font.caption, color: .tertiaryLabelColor, lines: 0)
    /// What the rows show, so a roster change that alters none of it keeps them (and a Copied
    /// confirmation) in place.
    private var shown: String?

    override func viewDidLoad() {
        super.viewDidLoad()
        title = L("Email")
        addSection(section)
        add(footnote)
        store.observe(self) { [weak self] event in
            switch event {
            case .mailChanged, .snapshotReplaced, .rosterChanged: self?.render()
            default: break
            }
        }
        render()
    }

    override func viewWillAppear() {
        super.viewWillAppear()
        store.refreshMail()
    }

    private func render() {
        let mail = store.mail
        let key = "\(String(describing: mail))|\(mail.flatMap(routingNote) ?? "")"
        guard key != shown else { return }
        shown = key
        guard let mail, mail.available else {
            // Only a page picked before the relay stopped offering email shows this.
            section.isHidden = true
            footnote.stringValue = mail == nil ? "" : L("Your relay doesn't offer email addresses.")
            return
        }
        section.isHidden = false
        guard let address = mail.address else {
            let get = ActionRow(key: SettingsEntry.emailAddress.row, value: "", tint: .secondaryLabelColor, actionTitle: L("Get an Address…"))
            get.onAction = { [weak self] in self?.presentSheet() }
            section.setRows([get])
            footnote.stringValue = L("Your bots can use an address of their own to sign up for services, write to people for you, and schedule time. Lorca keeps the mail it receives encrypted for your Runners.")
            return
        }

        let row = ActionRow(
            key: SettingsEntry.emailAddress.row, value: address.email, tint: .labelColor, actionTitle: L("Copy"), secondActionTitle: L("Change…"))
        row.onAction = { [weak self, weak row] in
            self?.copy(address.email)
            row?.showCopied()
        }
        row.onSecondAction = { [weak self] in self?.presentSheet() }
        row.menu = menu(for: address)
        var rows: [NSView] = [row]
        if address.state == .suspended {
            rows.append(KeyValueRow(key: L("Status"), value: L("Suspended"), tint: .systemRed))
        }
        let giveUp = ActionRow(key: L("Give Up"), value: "", tint: .secondaryLabelColor, actionTitle: L("Give Up Address…"))
        giveUp.onAction = { [weak self] in self?.confirmGiveUp(address) }
        rows.append(giveUp)
        section.setRows(rows)

        var notes: [String] = []
        if address.state == .suspended {
            notes.append(L("Mail to this address bounces for now, because mail sent from it kept bouncing."))
        }
        if let routing = routingNote(mail) { notes.append(routing) }
        footnote.stringValue = notes.joined(separator: " ")
    }

    /// Where mail goes: a bot's own address to that bot, the rest to the lead bot.
    private func routingNote(_ mail: MailStatus) -> String? {
        let lead = mail.leadBotId.flatMap { store.bot($0) }
        let example = store.bots.first { $0.id != lead?.id && mail.email(of: $0.id) != nil }
        switch (lead, example.flatMap { mail.email(of: $0.id) }) {
        case let (lead?, email?):
            return L("Each bot writes from an address of its own, such as %@, and mail to it goes to that bot. Other mail goes to %@.", email, lead.name)
        case let (lead?, nil):
            return L("Mail to this address goes to %@.", lead.name)
        default:
            return nil
        }
    }

    private func menu(for address: MailStatus.Address) -> NSMenu {
        let menu = NSMenu()
        menu.addItem(withTitle: L("Copy Address"), action: #selector(copyAddress), keyEquivalent: "").target = self
        menu.addItem(withTitle: L("Change Address…"), action: #selector(changeAddress), keyEquivalent: "").target = self
        menu.addItem(.separator())
        menu.addItem(withTitle: L("Give Up Address…"), action: #selector(giveUpAddress), keyEquivalent: "").target = self
        return menu
    }

    @objc private func copyAddress() {
        guard let email = store.mail?.address?.email else { return }
        copy(email)
    }

    @objc private func changeAddress() {
        presentSheet()
    }

    @objc private func giveUpAddress() {
        guard let address = store.mail?.address else { return }
        confirmGiveUp(address)
    }

    private func copy(_ email: String) {
        LinkBox.pasteboard.clearContents()
        LinkBox.pasteboard.setString(email, forType: .string)
    }

    private func presentSheet() {
        guard let mail = store.mail, mail.available else { return }
        presentAsSheet(EmailAddressViewController(current: mail.address, domain: mail.domain ?? ""))
    }

    private func confirmGiveUp(_ address: MailStatus.Address) {
        guard let window = view.window else { return }
        let alert = NSAlert()
        alert.messageText = L("Give up %@?", address.email)
        alert.informativeText = L("Mail to it will bounce, and nobody else can take the name.")
        alert.addButton(withTitle: L("Give Up"))
        alert.addButton(withTitle: L("Cancel"))
        alert.buttons.first?.hasDestructiveAction = true
        alert.beginSheetModal(for: window) { [weak self] response in
            guard response == .alertFirstButtonReturn, let self else { return }
            Task { @MainActor in
                do {
                    try await self.store.releaseMail()
                } catch {
                    let failed = NSAlert()
                    failed.messageText = L("Couldn't give up the address")
                    failed.informativeText = error.localizedDescription
                    failed.addButton(withTitle: L("OK"))
                    failed.beginSheetModal(for: window) { _ in }
                }
            }
        }
    }
}
