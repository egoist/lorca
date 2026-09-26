import AppKit

/// The same native Codex controls in onboarding, New Bot, and the inspector.
final class CodexSettingsView: NSStackView {
    private static var catalogs: [String: (Date, CodexCatalog)] = [:]
    private let runnerID: Device.ID
    private let botID: Bot.ID?
    private var catalog: CodexCatalog?
    private var error: String?
    private var loading = false
    private(set) var selection: CodexSelection
    var onChange: ((CodexSelection) -> Void)?

    init(runnerID: Device.ID, botID: Bot.ID? = nil, selection: CodexSelection) {
        self.runnerID = runnerID
        self.botID = botID
        self.selection = selection
        super.init(frame: .zero)
        orientation = .vertical
        alignment = .leading
        spacing = 10
        translatesAutoresizingMaskIntoConstraints = false
        if let (date, value) = Self.catalogs[cacheKey], Date().timeIntervalSince(date) < 60 { catalog = value }
        render()
        if catalog == nil { fetch() }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    private var cacheKey: String { "\(runnerID)/\(botID ?? "new")" }

    private func fetch() {
        guard !loading else { return }
        loading = true
        error = nil
        render()
        Task { [weak self] in
            guard let self else { return }
            do {
                let value = try await AppStore.shared.codexModels(runnerID: runnerID, botID: botID)
                Self.catalogs[cacheKey] = (Date(), value)
                catalog = value
            } catch { self.error = error.localizedDescription }
            loading = false
            render()
        }
    }

    private func changed() {
        render()
        onChange?(selection)
    }

    private func add(_ view: NSView) {
        addArrangedSubview(view)
        view.widthAnchor.constraint(equalTo: widthAnchor).isActive = true
    }

    private func render() {
        for view in arrangedSubviews { removeArrangedSubview(view); view.removeFromSuperview() }
        if let catalog {
            var ids: [String?] = [nil] + catalog.models.map { Optional($0.id) }
            var labels = [catalog.defaultModel.map { L("Default (%@)", $0) } ?? L("Default")] + catalog.models.map(\.label)
            if let model = selection.model, !ids.contains(model) { ids.append(model); labels.append(model) }
            let models = PopUpRow(key: L("Model"), items: labels, selected: ids.firstIndex(of: selection.model) ?? 0)
            models.onChange = { [weak self] index in
                guard let self, ids.indices.contains(index) else { return }
                selection.model = ids[index]
                selection.thinking = nil
                if selection.options.speed == .fast && catalog.model(selection.model)?.fastTier == nil { selection.options.speed = .default }
                changed()
            }
            add(models)

            var levels: [String?] = [nil] + (catalog.model(selection.model)?.levels ?? []).map { Optional($0) }
            if let level = selection.thinking, !levels.contains(level) { levels.append(level) }
            let defaultLabel = catalog.thinking(selection.model).map { L("Default (%@)", $0) } ?? L("Default")
            let thinking = PopUpRow(key: L("Thinking"), items: [defaultLabel] + levels.dropFirst().map { $0 ?? "" },
                                    selected: levels.firstIndex(of: selection.thinking) ?? 0)
            thinking.onChange = { [weak self] index in
                guard let self, levels.indices.contains(index) else { return }
                selection.thinking = levels[index]
                changed()
            }
            add(thinking)
        }
        let defaultSpeed = catalog?.usesFastByDefault(selection.model) == true ? L("Default (%@)", "Fast") : L("Default")
        var speeds: [CodexOptions.Speed] = [.default, .standard]
        var speedLabels = [defaultSpeed, L("Standard")]
        if catalog?.model(selection.model)?.fastTier != nil || selection.options.speed == .fast {
            speeds.append(.fast); speedLabels.append(L("Fast"))
        }
        let speed = PopUpRow(key: L("Speed"), items: speedLabels, selected: speeds.firstIndex(of: selection.options.speed) ?? 0)
        speed.onChange = { [weak self] index in
            guard let self, speeds.indices.contains(index) else { return }
            selection.options.speed = speeds[index]
            changed()
        }
        add(speed)
        let approvals = PopUpRow(key: L("Approvals"), items: [L("Approve for me"), L("Ask me")], selected: selection.options.approvals == .autoReview ? 0 : 1)
        approvals.onChange = { [weak self] index in
            guard let self else { return }
            selection.options.approvals = index == 0 ? .autoReview : .user
            changed()
        }
        add(approvals)
        add(NoteRow(text: L("Fast uses more quota. Approve for me lets Codex review native approval requests. Changes apply to the next turn.")))
        let status = ActionRow(key: L("Codex"), value: error ?? (loading ? L("Loading models…") : ""),
                               tint: error == nil ? .secondaryLabelColor : .systemRed, actionTitle: L("Reload models"))
        status.onAction = { [weak self] in self?.fetch() }
        add(status)
    }
}
