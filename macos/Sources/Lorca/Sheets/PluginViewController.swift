import AppKit

/// One installed plugin on a Runner: its state, the sign-in for a remote server, its
/// variables (a secret is written, never read back), the skills it brought, and Remove. From a
/// DM's inspector it also shows the Always allowed rules for its tools.
final class PluginViewController: SheetViewController {
    private let store = AppStore.shared
    private let pluginID: String
    private let runner: Device
    private let bot: Bot?

    private let status = SectionView(title: "Status")
    private let signIn = SectionView(title: "Sign-in")
    private let variables = SectionView(title: "Setup")
    private let skills = SectionView(title: "Skills")
    private let saveButton = NSButton()
    private let removeButton = NSButton()
    private let note = Build.label(
        "", font: Theme.Font.caption, color: .secondaryLabelColor, lines: 0)

    private var detail: PluginDetail?
    private var fields: [(name: String, field: NSTextField)] = []

    init(pluginID: String, runner: Device, bot: Bot?) {
        self.pluginID = pluginID
        self.runner = runner
        self.bot = bot
        let plugin = runner.plugins.first { $0.id == pluginID }
        super.init(
            title: plugin?.name ?? pluginID,
            subtitle: [plugin?.description ?? "", "Installed on \(runner.name)."].filter {
                !$0.isEmpty
            }.joined(separator: " "),
            width: 520
        )
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()
        saveButton.title = "Save"
        saveButton.bezelStyle = .rounded
        saveButton.target = self
        saveButton.action = #selector(save)
        removeButton.title = "Remove…"
        removeButton.bezelStyle = .rounded
        removeButton.target = self
        removeButton.action = #selector(confirmRemove)
        let spacer = NSView()
        spacer.setContentHuggingPriority(.defaultLow, for: .horizontal)
        let actions = Build.stack(
            [saveButton, spacer, removeButton], orientation: .horizontal, spacing: 8)

        for section in [status, signIn, variables, skills] {
            contentStack.addArrangedSubview(section)
            section.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
        }
        contentStack.addArrangedSubview(note)
        contentStack.addArrangedSubview(actions)
        NSLayoutConstraint.activate([
            note.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            actions.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
        ])
        setButtons(confirm: "Done", cancel: nil)
        status.setRows([KeyValueRow(key: "State", value: "Loading…", tint: .secondaryLabelColor)])
        signIn.isHidden = true
        variables.isHidden = true
        skills.isHidden = true
        saveButton.isHidden = true
        note.stringValue =
            runner.isThisDevice
            ? "Keys and sign-ins stay on this device."
            : "Keys and sign-ins are sent sealed to \(runner.name) and stay there. A sign-in opens the browser on \(runner.name)."
        load()
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        store.observe(self) { [weak self] event in
            switch event {
            case .rosterChanged, .snapshotReplaced:
                // The Runner's state moved (a sign-in finished, a connection failed).
                self?.load()
            default:
                break
            }
        }
    }

    private func load() {
        Task { [weak self] in
            guard let self else { return }
            do {
                let detail = try await self.store.pluginDetail(self.pluginID, on: self.runner.id)
                self.detail = detail
                self.render(detail)
            } catch {
                self.status.setRows([
                    KeyValueRow(key: "State", value: error.localizedDescription, tint: .systemRed)
                ])
                self.fitSheetToContent()
            }
        }
    }

    private func render(_ detail: PluginDetail) {
        var statusRows: [NSView] = [
            KeyValueRow(key: "State", value: detail.status.detail, tint: detail.status.stateColor)
        ]
        let rules = store.autoReview.rules.filter { $0.tool?.hasPrefix("\(pluginID)/") == true }
        if !rules.isEmpty {
            let always = ActionRow(
                key: "Always allowed",
                value: rules.map { String(($0.tool ?? "").dropFirst(pluginID.count + 1)) }.joined(
                    separator: ", "), tint: .labelColor, actionTitle: "Reset")
            always.onAction = { [weak self] in
                guard let self else { return }
                var review = self.store.autoReview
                review.rules.removeAll { $0.tool?.hasPrefix("\(self.pluginID)/") == true }
                self.store.setAutoReview(review)
                self.render(detail)
            }
            statusRows.append(always)
        }
        if let homepage = detail.homepage, let url = URL(string: homepage) {
            let site = ActionRow(
                key: "Site", value: url.host ?? homepage, tint: .secondaryLabelColor,
                actionTitle: "Open")
            site.onAction = { NSWorkspace.shared.open(url) }
            statusRows.append(site)
        }
        status.setRows(statusRows)

        let oauthServers = detail.servers.filter(\.oauth)
        signIn.isHidden = oauthServers.isEmpty
        signIn.setRows(
            oauthServers.map { server in
                let row = ActionRow(
                    key: oauthServers.count > 1 ? server.name : "Account",
                    value: server.signedIn ? "Signed in" : "Not signed in",
                    tint: server.signedIn ? .systemGreen : .secondaryLabelColor,
                    actionTitle: server.signedIn ? "Sign in again" : "Sign in")
                row.onAction = { [weak self] in self?.connect() }
                return row
            })

        fields = []
        variables.isHidden = detail.variables.isEmpty
        saveButton.isHidden = detail.variables.isEmpty
        variables.setRows(
            detail.variables.map { variable in
                let field: NSTextField = variable.secret ? NSSecureTextField() : NSTextField()
                field.placeholderString =
                    variable.secret
                    ? (variable.isSet ? "Set · type to replace" : "Not set")
                    : (variable.isSet ? "" : "Not set")
                field.stringValue = variable.secret ? "" : (variable.value ?? "")
                field.font = .monospacedSystemFont(ofSize: 11, weight: .regular)
                field.controlSize = .small
                field.toolTip = variable.description
                fields.append((variable.name, field))
                let row = FieldRow(
                    key: variable.name + (variable.required ? " *" : ""), field: field)
                return row
            })

        skills.isHidden = detail.skills.isEmpty
        skills.setRows(detail.skills.map { KeyValueRow(key: $0.name, value: $0.description) })
        fitSheetToContent()
    }

    @objc private func save() {
        var values: [String: String] = [:]
        for (name, field) in fields
        where !field.stringValue.isEmpty || !(field is NSSecureTextField) {
            values[name] = field.stringValue
        }
        saveButton.isEnabled = false
        Task { [weak self] in
            guard let self else { return }
            defer { self.saveButton.isEnabled = true }
            do {
                _ = try await self.store.setPluginVariables(
                    self.pluginID, on: self.runner.id, variables: values)
                self.load()
            } catch {
                self.alert("Couldn't save", error.localizedDescription)
            }
        }
    }

    private func connect() {
        Task { [weak self] in
            guard let self else { return }
            do {
                let message = try await self.store.connectPlugin(self.pluginID, on: self.runner.id)
                self.status.setRows([
                    KeyValueRow(key: "State", value: message, tint: .controlAccentColor)
                ])
                self.fitSheetToContent()
            } catch {
                self.alert("Couldn't start the sign-in", error.localizedDescription)
            }
        }
    }

    @objc private func confirmRemove() {
        guard let window = view.window else { return }
        let name = runner.plugins.first { $0.id == pluginID }?.name ?? pluginID
        let alert = NSAlert()
        alert.messageText = "Remove \(name) from \(runner.name)?"
        alert.informativeText =
            "Every bot on \(runner.name) loses it, and its keys and sign-ins there are forgotten."
        alert.addButton(withTitle: "Remove")
        alert.addButton(withTitle: "Cancel")
        alert.alertStyle = .warning
        alert.beginSheetModal(for: window) { [weak self] response in
            guard let self, response == .alertFirstButtonReturn else { return }
            Task { [weak self] in
                guard let self else { return }
                do {
                    try await self.store.uninstallPlugin(self.pluginID, on: self.runner.id)
                    self.dismiss(nil)
                } catch {
                    self.alert("Couldn't remove it", error.localizedDescription)
                }
            }
        }
    }

    private func alert(_ title: String, _ text: String) {
        let alert = NSAlert()
        alert.messageText = title
        alert.informativeText = text
        if let window = view.window { alert.beginSheetModal(for: window) } else { alert.runModal() }
    }
}

/// Key on the left, a bordered field on the right, inside a section card.
final class FieldRow: NSView {
    init(key keyText: String, field: NSTextField) {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        let key = Build.label(
            keyText, font: .monospacedSystemFont(ofSize: 11, weight: .regular),
            color: .secondaryLabelColor)
        key.setContentCompressionResistancePriority(.required, for: .horizontal)
        field.translatesAutoresizingMaskIntoConstraints = false
        addSubview(key)
        addSubview(field)
        NSLayoutConstraint.activate([
            heightAnchor.constraint(greaterThanOrEqualToConstant: 34),
            key.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            key.centerYAnchor.constraint(equalTo: centerYAnchor),
            field.leadingAnchor.constraint(greaterThanOrEqualTo: key.trailingAnchor, constant: 10),
            field.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -10),
            field.centerYAnchor.constraint(equalTo: centerYAnchor),
            field.widthAnchor.constraint(equalToConstant: 230),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }
}
