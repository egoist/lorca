import AppKit

/// One of a bot's browser profiles, as its Runner reports it. Each keeps its own sign-ins.
struct BrowserProfile: Decodable, Equatable {
    /// The Browser plugin, whose sheet lists a bot's profiles.
    static let pluginID = "playwright"

    enum State: String, Decodable {
        case stopped
        case bot
        case takingOver = "taking_over"
        case human
    }

    let id: String
    let name: String
    let state: State
    /// Goes up with every change of control; Return to Bot sends the one this Mac last saw.
    let revision: UInt64
    /// The user is recording a workflow in it for the bot to learn, with the browser in hand.
    let recording: Bool

    private enum CodingKeys: String, CodingKey { case id, name, state, revision, recording }

    init(id: String, name: String, state: State, revision: UInt64, recording: Bool = false) {
        self.id = id
        self.name = name
        self.state = state
        self.revision = revision
        self.recording = recording
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        id = try values.decode(String.self, forKey: .id)
        name = try values.decode(String.self, forKey: .name)
        state = try values.decode(State.self, forKey: .state)
        revision = try values.decode(UInt64.self, forKey: .revision)
        // A Runner from before recordings leaves it out.
        recording = try values.decodeIfPresent(Bool.self, forKey: .recording) ?? false
    }
}

extension AppStore {
    private struct ProfileList: Decodable { let sessions: [BrowserProfile] }

    /// The bot's profiles, oldest first, from its Runner: this Mac's CLI, or a sealed request.
    func browserProfiles(of botID: Bot.ID) async throws -> [BrowserProfile] {
        if isMock { return MockBrowser.profiles[botID] ?? [] }
        return try await client.request("browser.sessions", ["bot_id": botID], as: ProfileList.self).sessions
    }

    /// `browser.create` (`name`), `browser.open`, `browser.takeover`, `browser.resume`
    /// (`revision`), `browser.stop`, `browser.delete`, `browser.screenshot` (`chat_id`), or
    /// `browser.record`.
    func browserProfileAction(_ method: String, botID: Bot.ID, params: [String: Any]) async throws {
        if isMock {
            MockBrowser.apply(method, botID: botID, params: params)
            return
        }
        _ = try await client.request(method, params.merging(["bot_id": botID]) { _, new in new })
    }

    private struct StoppedRecording: Decodable {
        let messageId: String?
        enum CodingKeys: String, CodingKey { case messageId = "message_id" }
    }

    /// Stops the profile's recording and sends it to the bot in the chat with the user's words.
    /// False when the user did nothing in the browser, so nothing was sent.
    func stopBrowserRecording(botID: Bot.ID, profileID: String, chatID: Chat.ID, text: String) async throws -> Bool {
        if isMock {
            MockBrowser.apply("browser.stop_recording", botID: botID, params: ["session_id": profileID])
            return true
        }
        let params: [String: Any] = ["bot_id": botID, "session_id": profileID, "chat_id": chatID, "text": text]
        return try await client.request("browser.stop_recording", params, as: StoppedRecording.self).messageId != nil
    }
}

/// The demo's profiles: in memory, with no browser behind them.
@MainActor
private enum MockBrowser {
    static var profiles: [Bot.ID: [BrowserProfile]] = [:]

    static func apply(_ method: String, botID: Bot.ID, params: [String: Any]) {
        var list = profiles[botID] ?? []
        let id = params["session_id"] as? String
        func set(_ state: BrowserProfile.State, recording: Bool = false) {
            list = list.map { $0.id == id ? BrowserProfile(id: $0.id, name: $0.name, state: state, revision: $0.revision + 1, recording: recording) : $0 }
        }
        switch method {
        case "browser.create":
            list.append(BrowserProfile(id: UUID().uuidString, name: params["name"] as? String ?? "", state: .stopped, revision: 1))
        case "browser.open", "browser.takeover", "browser.stop_recording": set(.human)
        case "browser.record": set(.human, recording: true)
        case "browser.resume": set(.bot)
        case "browser.stop": set(.stopped)
        case "browser.delete": list.removeAll { $0.id == id }
        default: break
        }
        profiles[botID] = list
    }
}

/// The Browser plugin sheet's Profiles section for one bot: a row per profile with how it
/// stands, + to add one, and on a click or a right-click the profile's menu: open it, take it
/// over or hand it back, record a workflow for the bot to learn, take a screenshot into the
/// chat, close it, or delete it.
@MainActor
final class BrowserProfilesSection: NSObject {
    let section = SectionView(title: L("Profiles"))
    /// After the rows change, so the sheet fits them.
    var onChange: (() -> Void)?

    private let store = AppStore.shared
    private let bot: Bot
    private let runner: Device
    private let chatID: Chat.ID?
    private weak var presenter: NSViewController?
    private var profiles: [BrowserProfile]?
    private var loadError: String?
    /// What a row says while its action runs on the Runner ("Opening…").
    private var pending: [String: String] = [:]
    private var poll: Task<Void, Never>?
    private var loads = 0
    private var rows: [String: StatusRow] = [:]
    private var note: (text: String, row: NoteRow)?
    /// What the rows last showed, so a poll that finds nothing new leaves the sheet alone.
    private var shown: (profiles: [BrowserProfile]?, error: String?, pending: [String: String])?

    init(bot: Bot, runner: Device, chatID: Chat.ID?, presenter: NSViewController) {
        self.bot = bot
        self.runner = runner
        self.chatID = chatID
        self.presenter = presenter
        super.init()
        section.setHeaderAccessory(HoverButton(symbol: "plus", pointSize: 11, tooltip: L("Add Profile"), target: self, action: #selector(addProfile)))
        section.isHidden = true
    }

    /// Asks the Runner while the sheet is up: on this Mac every two seconds, through the relay
    /// every five, since another Device can change the browser too.
    func start() {
        poll?.cancel()
        let interval: UInt64 = runner.isThisDevice ? 2 : 5
        poll = Task { [weak self] in
            while !Task.isCancelled {
                await self?.refresh()
                try? await Task.sleep(nanoseconds: interval * 1_000_000_000)
            }
        }
    }

    func stop() {
        poll?.cancel()
        poll = nil
    }

    private func refresh() async {
        loads += 1
        let load = loads
        do {
            let profiles = try await store.browserProfiles(of: bot.id)
            guard load == loads else { return }
            self.profiles = profiles
            loadError = nil
        } catch {
            guard load == loads else { return }
            loadError = error.localizedDescription
        }
        render()
    }

    private func render() {
        guard profiles != nil || loadError != nil else { return }
        if let shown, shown.profiles == profiles, shown.error == loadError, shown.pending == pending { return }
        shown = (profiles, loadError, pending)
        var views: [NSView] = (profiles ?? []).map { profile in
            let row = rows[profile.id] ?? makeRow(for: profile.id)
            let (state, color) = stateText(of: profile)
            row.configure(symbol: "person.crop.circle", title: profile.name, subtitle: "", state: state, stateColor: color)
            row.toolTip = explanation(of: profile)
            row.menu = menu(for: profile)
            return row
        }
        rows = rows.filter { id, _ in profiles?.contains { $0.id == id } == true }
        if let text = loadError ?? (views.isEmpty ? L("A profile keeps sign-ins for %@'s browser. Add one, then open it on %@ to sign in.", bot.name, runner.name) : nil) {
            if note?.text != text { note = (text, NoteRow(text: text)) }
            views.append(note!.row)
        }
        section.setRows(views)
        section.isHidden = false
        onChange?()
    }

    private func makeRow(for id: String) -> StatusRow {
        let row = StatusRow()
        row.identifier = NSUserInterfaceItemIdentifier(id)
        row.addGestureRecognizer(NSClickGestureRecognizer(target: self, action: #selector(rowClicked(_:))))
        rows[id] = row
        return row
    }

    /// One or two words; orange while the bot waits on the user to hand the browser back, red
    /// while it records.
    private func stateText(of profile: BrowserProfile) -> (String, NSColor) {
        if let pending = pending[profile.id] { return (pending, .secondaryLabelColor) }
        if profile.recording { return (L("Recording"), .systemRed) }
        switch profile.state {
        case .stopped: return (L("Closed", context: "browser"), .secondaryLabelColor)
        case .bot: return (L("Open", context: "browser"), .secondaryLabelColor)
        case .takingOver: return (L("Taking over…"), .secondaryLabelColor)
        case .human: return (L("You have control"), .systemOrange)
        }
    }

    /// What the state means for the bot, as the row's tooltip.
    private func explanation(of profile: BrowserProfile) -> String {
        if profile.recording {
            return L("Do the task in this browser on %@, then choose Stop Recording. Passwords aren't recorded.", runner.name)
        }
        switch profile.state {
        case .stopped: return L("Open it on %@ to sign in. %@ can open it too.", runner.name, bot.name)
        case .bot: return L("%@'s browser calls use this profile.", bot.name)
        case .takingOver: return L("Waiting for %@'s current step in the browser to finish.", bot.name)
        case .human: return L("%@'s browser calls wait until you return the browser.", bot.name)
        }
    }

    private func menu(for profile: BrowserProfile) -> NSMenu {
        let menu = NSMenu()
        menu.autoenablesItems = false
        func add(_ title: String, _ action: Selector, enabled: Bool = true) {
            let item = NSMenuItem(title: title, action: action, keyEquivalent: "")
            item.target = self
            item.representedObject = profile.id
            item.isEnabled = enabled && pending[profile.id] == nil
            menu.addItem(item)
        }
        switch profile.state {
        case _ where profile.recording:
            add(L("Stop Recording"), #selector(stopRecording(_:)), enabled: chatID != nil)
        case .stopped:
            // A window opens only on the Runner's own screen, and a recording is made there.
            if runner.isThisDevice {
                add(L("Open Browser"), #selector(openBrowser(_:)))
            } else {
                add(L("Open on %@", runner.name), #selector(openBrowser(_:)), enabled: false)
            }
            if chatID != nil {
                if runner.isThisDevice {
                    add(L("Record", context: "browser"), #selector(record(_:)))
                } else {
                    add(L("Record on %@", runner.name), #selector(record(_:)), enabled: false)
                }
            }
        case .bot:
            add(L("Take Over"), #selector(takeOver(_:)))
            if chatID != nil { add(L("Record", context: "browser"), #selector(record(_:))) }
        case .takingOver:
            break
        case .human:
            add(L("Return to Bot"), #selector(returnToBot(_:)))
            if chatID != nil { add(L("Record", context: "browser"), #selector(record(_:))) }
        }
        if profile.state == .bot || profile.state == .human, chatID != nil {
            add(L("Take Screenshot"), #selector(takeScreenshot(_:)))
        }
        if profile.state != .stopped || pending[profile.id] != nil {
            let close = NSMenuItem(title: L("Close Browser"), action: #selector(closeBrowser(_:)), keyEquivalent: "")
            close.target = self
            close.representedObject = profile.id
            // Close works while an open or a takeover waits.
            menu.addItem(close)
        }
        if !menu.items.isEmpty { menu.addItem(.separator()) }
        add(L("Delete…"), #selector(confirmDelete(_:)))
        return menu
    }

    @objc private func rowClicked(_ sender: NSClickGestureRecognizer) {
        guard let row = sender.view, let menu = row.menu, let event = NSApp.currentEvent else { return }
        NSMenu.popUpContextMenu(menu, with: event, for: row)
    }

    private func profile(_ sender: NSMenuItem) -> BrowserProfile? {
        profiles?.first { $0.id == sender.representedObject as? String }
    }

    @objc private func openBrowser(_ sender: NSMenuItem) {
        guard let profile = profile(sender) else { return }
        perform("browser.open", on: profile, while: L("Opening…"), failure: L("Couldn't open the browser"))
    }

    @objc private func takeOver(_ sender: NSMenuItem) {
        guard let profile = profile(sender) else { return }
        perform("browser.takeover", on: profile, while: L("Taking over…"), failure: L("Couldn't take over the browser"))
    }

    @objc private func returnToBot(_ sender: NSMenuItem) {
        guard let profile = profile(sender) else { return }
        perform("browser.resume", on: profile, params: ["revision": profile.revision], while: L("Returning…"), failure: L("Couldn't hand the browser back"))
    }

    @objc private func record(_ sender: NSMenuItem) {
        guard let profile = profile(sender) else { return }
        perform("browser.record", on: profile, while: L("Starting…"), failure: L("Couldn't start recording"))
    }

    /// Sends the recording to the bot in the chat, which then shows: the bot answers there.
    @objc private func stopRecording(_ sender: NSMenuItem) {
        guard let profile = profile(sender), let chatID else { return }
        pending[profile.id] = L("Stopping…")
        render()
        loads += 1
        let text = L("I recorded this in the %@ browser. Make it a skill you can repeat, and ask me about anything the recording doesn't show.", profile.name)
        Task { [weak self] in
            guard let self else { return }
            do {
                let sent = try await self.store.stopBrowserRecording(botID: self.bot.id, profileID: profile.id, chatID: chatID, text: text)
                self.pending[profile.id] = nil
                if sent {
                    self.presenter?.dismiss(nil)
                    return
                }
                self.alert(L("Nothing was recorded"), L("Do the task in the browser while it records, then stop."))
            } catch {
                self.pending[profile.id] = nil
                self.alert(L("Couldn't stop recording"), error.localizedDescription)
            }
            await self.refresh()
        }
    }

    @objc private func takeScreenshot(_ sender: NSMenuItem) {
        guard let profile = profile(sender), let chatID else { return }
        perform("browser.screenshot", on: profile, params: ["chat_id": chatID], while: nil, failure: L("Couldn't take a screenshot"))
    }

    @objc private func closeBrowser(_ sender: NSMenuItem) {
        guard let profile = profile(sender) else { return }
        perform("browser.stop", on: profile, while: L("Closing…"), failure: L("Couldn't close the browser"))
    }

    @objc private func confirmDelete(_ sender: NSMenuItem) {
        guard let profile = profile(sender), let window = presenter?.view.window else { return }
        let alert = NSAlert()
        alert.messageText = L("Delete the “%@” profile?", profile.name)
        alert.informativeText = L("Its browser closes, and the sites it signed in to are signed out for %@.", bot.name)
        alert.addButton(withTitle: L("Delete"))
        alert.addButton(withTitle: L("Cancel"))
        alert.buttons[0].hasDestructiveAction = true
        alert.alertStyle = .warning
        alert.beginSheetModal(for: window) { [weak self] response in
            guard response == .alertFirstButtonReturn else { return }
            self?.perform("browser.delete", on: profile, while: L("Deleting…"), failure: L("Couldn't delete the profile"))
        }
    }

    @objc private func addProfile() {
        guard let window = presenter?.view.window else { return }
        let alert = NSAlert()
        alert.messageText = L("New Profile")
        alert.informativeText = L("%@ keeps the sign-ins you make in it.", bot.name)
        alert.addButton(withTitle: L("Add"))
        alert.addButton(withTitle: L("Cancel"))
        let field = NSTextField(frame: NSRect(x: 0, y: 0, width: 260, height: 24))
        field.placeholderString = L("Work", context: "browser profile")
        alert.accessoryView = field
        alert.beginSheetModal(for: window) { [weak self] response in
            guard let self, response == .alertFirstButtonReturn else { return }
            let name = field.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !name.isEmpty else { return }
            Task { [weak self] in
                guard let self else { return }
                do {
                    try await self.store.browserProfileAction("browser.create", botID: self.bot.id, params: ["name": name])
                } catch {
                    self.alert(L("Couldn't add the profile"), error.localizedDescription)
                }
                await self.refresh()
            }
        }
        // The accessory view only becomes first responder once the sheet exists.
        DispatchQueue.main.async { alert.window.makeFirstResponder(field) }
    }

    private func perform(_ method: String, on profile: BrowserProfile, params: [String: Any] = [:], while label: String?, failure: String) {
        if let label { pending[profile.id] = label }
        render()
        // A read that started before this action could describe the browser as it was.
        loads += 1
        Task { [weak self] in
            guard let self else { return }
            do {
                try await self.store.browserProfileAction(method, botID: self.bot.id, params: params.merging(["session_id": profile.id]) { _, new in new })
            } catch {
                self.alert(failure, error.localizedDescription)
            }
            self.pending[profile.id] = nil
            await self.refresh()
        }
    }

    private func alert(_ title: String, _ text: String) {
        guard let window = presenter?.view.window else { return }
        let alert = NSAlert()
        alert.messageText = title
        alert.informativeText = text
        alert.beginSheetModal(for: window)
    }
}
