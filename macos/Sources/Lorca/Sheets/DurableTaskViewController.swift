import AppKit

/// Task editing uses the revision that was opened. Errors keep the form; Reload fetches the
/// authority's current revision. Retried delivery keeps the same request id and payload.
final class DurableTaskViewController: SheetViewController {
    private let store = AppStore.shared
    private let chatID: Chat.ID
    private var record: DurableTask?
    private let goal = NSTextField()
    private let owner = NSPopUpButton()
    private let state = NSPopUpButton()
    private let criteria = NSTextView()
    private let nextAction = NSTextView()
    private let reason = NSTextView()
    private let result = NSTextView()
    private let links = NSTextView()
    private let dependencyMenu = NSPopUpButton()
    private let dependencyRows = Build.stack([], spacing: 4)
    private let chatMenu = NSPopUpButton()
    private let chatRows = Build.stack([], spacing: 4)
    private let evidenceMenu = NSPopUpButton()
    private let evidenceRows = Build.stack([], spacing: 4)
    private let evidenceURL = NSTextField()
    private let errorLabel = Build.label("", font: .systemFont(ofSize: 12), color: .systemRed, lines: 0)
    private let metadata = Build.label("", font: .systemFont(ofSize: 11), color: .secondaryLabelColor, lines: 0)
    private let runButton = NSButton()
    private let reloadButton = NSButton()
    private var members: [Bot] = []
    private var dependencies: [String] = []
    private var linkedChats: [String] = []
    private var evidence: [DurableTask.Evidence] = []
    private var requestID = UUID().uuidString.lowercased()
    private var requestBody: Data?
    private var isSaving = false
    private var runRequestID: String?
    private let creationID = "task-\(UUID().uuidString.lowercased())"

    init(chatID: Chat.ID, task: DurableTask?) {
        self.chatID = chatID
        record = task
        super.init(title: task == nil ? L("New task") : L("Task"), subtitle: L("Keep ownership, progress, and evidence across turns."), width: 590)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()
        let form = Build.stack([], spacing: 12)
        form.alignment = .leading
        form.edgeInsets = NSEdgeInsets(top: 4, left: 4, bottom: 10, right: 4)
        let scroll = NSScrollView()
        let document = FlippedView()
        document.addSubview(form)
        document.translatesAutoresizingMaskIntoConstraints = false
        form.translatesAutoresizingMaskIntoConstraints = false
        scroll.documentView = document
        scroll.hasVerticalScroller = true
        scroll.drawsBackground = false
        scroll.translatesAutoresizingMaskIntoConstraints = false
        contentStack.addArrangedSubview(scroll)
        contentStack.addArrangedSubview(errorLabel)
        NSLayoutConstraint.activate([
            scroll.widthAnchor.constraint(equalTo: contentStack.widthAnchor), scroll.heightAnchor.constraint(equalToConstant: 510),
            document.widthAnchor.constraint(equalTo: scroll.widthAnchor),
            form.topAnchor.constraint(equalTo: document.topAnchor), form.leadingAnchor.constraint(equalTo: document.leadingAnchor),
            form.trailingAnchor.constraint(equalTo: document.trailingAnchor), form.bottomAnchor.constraint(equalTo: document.bottomAnchor),
            errorLabel.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
        ])
        func add(_ title: String, _ control: NSView) {
            let label = Build.label(title, font: .systemFont(ofSize: 12, weight: .semibold))
            let row = Build.stack([label, control], spacing: 4)
            form.addArrangedSubview(row)
            row.widthAnchor.constraint(equalTo: form.widthAnchor, constant: -8).isActive = true
            control.widthAnchor.constraint(equalTo: row.widthAnchor).isActive = true
            control.setAccessibilityLabel(title)
        }
        func editor(_ view: NSTextView, height: CGFloat) -> NSView {
            view.isRichText = false; view.font = .systemFont(ofSize: 12)
            view.textColor = .labelColor; view.allowsUndo = true
            view.textContainerInset = NSSize(width: 6, height: 6)
            view.isVerticallyResizable = true; view.isHorizontallyResizable = false
            view.autoresizingMask = [.width]; view.textContainer?.widthTracksTextView = true
            let scroll = NSScrollView()
            scroll.documentView = view; scroll.hasVerticalScroller = true; scroll.borderType = .bezelBorder
            scroll.heightAnchor.constraint(equalToConstant: height).isActive = true
            return scroll
        }
        goal.placeholderString = L("What should be achieved?")
        add(L("Goal"), goal); add(L("Owning bot"), owner); add(L("State"), state)
        add(L("Linked chats"), chatRows); add(L("Add linked chat"), chatMenu)
        chatMenu.target = self; chatMenu.action = #selector(addLinkedChat)
        add(L("Acceptance criteria · one per line"), editor(criteria, height: 70))
        add(L("Next action"), editor(nextAction, height: 55))
        add(L("Reason · required when blocked or cancelled"), editor(reason, height: 55))
        add(L("Result · required for completion"), editor(result, height: 90))
        add(L("Dependencies"), dependencyRows); add(L("Add dependency"), dependencyMenu)
        dependencyMenu.target = self; dependencyMenu.action = #selector(addDependency)
        add(L("External links · one HTTPS URL per line"), editor(links, height: 55))
        add(L("Supporting evidence"), evidenceRows); add(L("Add a chat message as evidence"), evidenceMenu)
        evidenceMenu.target = self; evidenceMenu.action = #selector(addMessageEvidence)
        evidenceURL.placeholderString = L("HTTPS link to supporting evidence")
        let addLink = NSButton(title: L("Add link"), target: self, action: #selector(addLinkEvidence))
        addLink.bezelStyle = .rounded
        add(L("Add link evidence"), Build.stack([evidenceURL, addLink], orientation: .horizontal, spacing: 8))
        form.addArrangedSubview(metadata)
        metadata.widthAnchor.constraint(equalTo: form.widthAnchor, constant: -8).isActive = true
        state.addItems(withTitles: DurableTask.State.allCases.map(\.title))
        reloadButton.title = L("Reload"); reloadButton.bezelStyle = .rounded
        reloadButton.target = self; reloadButton.action = #selector(reloadTask)
        runButton.title = L("Start saved task"); runButton.bezelStyle = .rounded
        runButton.target = self; runButton.action = #selector(runTask)
        let actions = Build.stack([reloadButton, runButton], orientation: .horizontal, spacing: 8)
        contentStack.addArrangedSubview(actions)
        setButtons(confirm: L("Save"))
        populate()
    }

    private func populate() {
        runRequestID = nil
        linkedChats = record?.chatIds ?? [chatID]
        members = []
        refreshOwners()
        goal.stringValue = record?.goal ?? ""
        criteria.string = record?.acceptanceCriteria.joined(separator: "\n") ?? ""
        nextAction.string = record?.nextAction ?? ""
        reason.string = record?.reason ?? ""
        result.string = record?.result ?? ""
        links.string = record?.links.map(\.url).joined(separator: "\n") ?? ""
        state.selectItem(at: DurableTask.State.allCases.firstIndex(of: record?.state ?? .queued) ?? 0)
        state.isEnabled = record != nil
        dependencies = record?.dependencies ?? []; evidence = record?.evidence ?? []
        if let record {
            metadata.stringValue = "\(record.id)\n\(L("Assigned Runner: %@", store.device(record.runnerId)?.name ?? L("Runner")))\n\(L("Saved on %@", store.device(record.authorityRunnerId)?.name ?? L("Runner")))"
        } else { metadata.stringValue = "" }
        reloadButton.isEnabled = record != nil
        runButton.isEnabled = record?.state.canRun == true && record?.activeRun == nil
        owner.isEnabled = record?.activeRun == nil
        rebuildReferences()
        refreshEvidenceMenu()
    }

    private func refreshOwners() {
        let selected = members.indices.contains(owner.indexOfSelectedItem) ? members[owner.indexOfSelectedItem].id : record?.ownerBotId
        members = store.bots.filter { bot in linkedChats.contains { store.chat($0)?.botIDs.contains(bot.id) == true } }
        owner.removeAllItems()
        owner.addItems(withTitles: members.map { "\($0.name) · \(store.device($0.runnerID)?.name ?? L("Runner"))" })
        owner.selectItem(at: members.firstIndex { $0.id == selected } ?? 0)
        confirmButton.isEnabled = !members.isEmpty && !linkedChats.isEmpty
    }

    private func refreshEvidenceMenu() {
        evidenceMenu.removeAllItems()
        evidenceMenu.addItem(withTitle: L("Choose a message…"))
        for id in linkedChats {
            for message in store.chat(id)?.messages.filter({ $0.canBeQuoted }) ?? [] {
                let label = String(message.text.replacingOccurrences(of: "\n", with: " ").prefix(100))
                evidenceMenu.addItem(withTitle: label.isEmpty ? L("Attachment") : label)
                evidenceMenu.lastItem?.representedObject = DurableTask.Evidence(kind: "message", label: label.isEmpty ? L("Attachment") : label, chatId: id, messageId: message.id)
            }
        }
    }

    private func rebuildReferences() {
        chatRows.arrangedSubviews.forEach { chatRows.removeArrangedSubview($0); $0.removeFromSuperview() }
        dependencyRows.arrangedSubviews.forEach { dependencyRows.removeArrangedSubview($0); $0.removeFromSuperview() }
        evidenceRows.arrangedSubviews.forEach { evidenceRows.removeArrangedSubview($0); $0.removeFromSuperview() }
        for id in linkedChats {
            let row = ActionRow(key: chatTitle(id), value: "", tint: .secondaryLabelColor, actionTitle: record?.activeRun == nil ? L("Remove") : nil)
            row.onAction = { [weak self] in
                guard let self else { return }
                linkedChats.removeAll { $0 == id }; refreshOwners(); rebuildReferences(); refreshEvidenceMenu()
            }
            chatRows.addArrangedSubview(row)
            row.widthAnchor.constraint(equalTo: chatRows.widthAnchor).isActive = true
        }
        chatMenu.removeAllItems(); chatMenu.addItem(withTitle: L("Choose a chat…"))
        for chat in store.chats where !linkedChats.contains(chat.id) {
            chatMenu.addItem(withTitle: chatTitle(chat.id)); chatMenu.lastItem?.representedObject = chat.id
        }
        chatMenu.isEnabled = record?.activeRun == nil
        for id in dependencies {
            let label = store.durableTasks.first { $0.id == id }?.goal ?? id
            let row = ActionRow(key: label, value: "", tint: .secondaryLabelColor, actionTitle: L("Remove"))
            row.onAction = { [weak self] in self?.dependencies.removeAll { $0 == id }; self?.rebuildReferences() }
            dependencyRows.addArrangedSubview(row)
            row.widthAnchor.constraint(equalTo: dependencyRows.widthAnchor).isActive = true
        }
        for item in evidence {
            let row = ActionRow(key: item.label, value: item.url ?? "", tint: .secondaryLabelColor, actionTitle: L("Remove"))
            row.toolTip = item.url ?? item.messageId
            row.onAction = { [weak self] in self?.evidence.removeAll { $0 == item }; self?.rebuildReferences() }
            evidenceRows.addArrangedSubview(row)
            row.widthAnchor.constraint(equalTo: evidenceRows.widthAnchor).isActive = true
        }
        dependencyMenu.removeAllItems(); dependencyMenu.addItem(withTitle: L("Choose a task…"))
        for task in store.durableTasks where task.id != record?.id && !dependencies.contains(task.id) {
            dependencyMenu.addItem(withTitle: task.goal); dependencyMenu.lastItem?.representedObject = task.id
        }
    }

    @objc private func addDependency() {
        if let id = dependencyMenu.selectedItem?.representedObject as? String { dependencies.append(id); rebuildReferences() }
    }
    private func chatTitle(_ id: Chat.ID) -> String {
        guard let chat = store.chat(id) else { return id }
        return chat.customTitle ?? store.bots(in: chat).map(\.name).joined(separator: ", ")
    }
    @objc private func addLinkedChat() {
        if let id = chatMenu.selectedItem?.representedObject as? String {
            linkedChats.append(id); refreshOwners(); rebuildReferences(); refreshEvidenceMenu()
        }
    }
    @objc private func addMessageEvidence() {
        if let item = evidenceMenu.selectedItem?.representedObject as? DurableTask.Evidence, !evidence.contains(item) {
            evidence.append(item); rebuildReferences()
        }
        evidenceMenu.selectItem(at: 0)
    }
    @objc private func addLinkEvidence() {
        let value = evidenceURL.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let url = URL(string: value), url.scheme == "https", url.host != nil else {
            errorLabel.stringValue = L("Enter an HTTPS link."); return
        }
        let item = DurableTask.Evidence(kind: "url", label: value, url: value)
        if !evidence.contains(item) { evidence.append(item) }
        evidenceURL.stringValue = ""; errorLabel.stringValue = ""; rebuildReferences()
    }

    private func lines(_ text: String) -> [String] {
        text.components(separatedBy: .newlines).map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }.filter { !$0.isEmpty }
    }

    override func confirmTapped() {
        guard !isSaving, members.indices.contains(owner.indexOfSelectedItem) else { return }
        let bot = members[owner.indexOfSelectedItem]
        var params: [String: Any] = [:]
        func changed(_ field: String, _ value: String, _ previous: String?) { if value != previous { params[field] = value } }
        changed("goal", goal.stringValue, record?.goal)
        changed("next_action", nextAction.string, record?.nextAction)
        changed("owner_bot_id", bot.id, record?.ownerBotId)
        if let record, record.runnerId != bot.runnerID { params["runner_id"] = bot.runnerID }
        let acceptance = lines(criteria.string)
        if acceptance != record?.acceptanceCriteria { params["acceptance_criteria"] = acceptance }
        if dependencies != record?.dependencies { params["dependencies"] = dependencies }
        if linkedChats != record?.chatIds { params["chat_ids"] = linkedChats }
        let urls = lines(links.string)
        if urls != record?.links.map(\.url) { params["links"] = urls.map { ["label": $0, "url": $0] } }
        if let record {
            params["id"] = record.id; params["expected_revision"] = record.revision
            let selectedState = DurableTask.State.allCases[state.indexOfSelectedItem]
            if selectedState != record.state { params["state"] = selectedState.rawValue }
            if reason.string != (record.reason ?? "") { params["reason"] = reason.string.isEmpty ? NSNull() : reason.string as Any }
            if result.string != (record.result ?? "") { params["result"] = result.string.isEmpty ? NSNull() : result.string as Any }
            if evidence != record.evidence { params["evidence"] = evidence.map(\.params) }
        } else {
            params["id"] = creationID; params["chat_ids"] = linkedChats
        }
        do {
            let body = try JSONSerialization.data(withJSONObject: params, options: [.sortedKeys])
            if requestBody != body { requestBody = body; requestID = UUID().uuidString.lowercased() }
        } catch { errorLabel.stringValue = error.localizedDescription; return }
        params["request_id"] = requestID
        isSaving = true; confirmButton.isEnabled = false; runButton.isEnabled = false
        errorLabel.stringValue = ""
        Task { [weak self] in
            guard let self else { return }
            do { _ = try await store.taskRequest(record == nil ? "tasks.create" : "tasks.update", params: params); dismiss(nil) }
            catch { errorLabel.stringValue = error.localizedDescription }
            isSaving = false; confirmButton.isEnabled = !members.isEmpty && !linkedChats.isEmpty
            runButton.isEnabled = record?.state.canRun == true && record?.activeRun == nil
        }
    }

    @objc private func reloadTask() {
        guard let record, !isSaving else { return }
        isSaving = true
        Task { [weak self] in
            guard let self else { return }
            do { self.record = try await store.taskRequest("tasks.get", params: ["id": record.id, "refresh": true]); populate(); errorLabel.stringValue = "" }
            catch { errorLabel.stringValue = error.localizedDescription }
            isSaving = false
        }
    }

    @objc private func runTask() {
        guard let record, !isSaving else { return }
        isSaving = true; runButton.isEnabled = false
        if runRequestID == nil { runRequestID = UUID().uuidString.lowercased() }
        let runChat = store.chat(chatID)?.botIDs.contains(record.ownerBotId) == true ? chatID : record.chatIds.first { store.chat($0)?.botIDs.contains(record.ownerBotId) == true }
        guard let runChat else { isSaving = false; errorLabel.stringValue = L("The task owner needs a linked chat to run in."); return }
        let params: [String: Any] = ["id": record.id, "expected_revision": record.revision, "request_id": runRequestID!, "chat_id": runChat]
        Task { [weak self] in
            guard let self else { return }
            do { _ = try await store.taskRequest("tasks.run", params: params); dismiss(nil) }
            catch { errorLabel.stringValue = error.localizedDescription; runButton.isEnabled = true }
            isSaving = false
        }
    }
}
