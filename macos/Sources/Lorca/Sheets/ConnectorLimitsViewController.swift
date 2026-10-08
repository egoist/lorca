import AppKit

/// How often, and how many at once, all bots on a Runner may call one plugin account. An
/// account of a service with several accounts can also set the limit they share.
final class ConnectorLimitsViewController: SheetViewController {
    private let store = AppStore.shared
    private let pluginID: String
    private let name: String
    private let runnerID: Device.ID
    private let scope = NSPopUpButton()
    private let calls = NSTextField()
    private let window = NSTextField()
    private let concurrency = NSTextField()
    private let note = Build.label("", font: .systemFont(ofSize: 12), color: .secondaryLabelColor, lines: 0)
    private let errorLabel = Build.label("", font: .systemFont(ofSize: 12), color: .systemRed, lines: 0)
    /// Its first row, Applies to, shows only for an account whose service has others.
    private var grid: NSGridView?
    private var loads = 0
    var onSaved: (() -> Void)?

    init(pluginID: String, name: String, runner: Device) {
        self.pluginID = pluginID
        self.name = name
        runnerID = runner.id
        super.init(title: L("Call Limit"), subtitle: L("All bots on %@ share this limit when they use %@.", runner.name, name), width: 420)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()
        scope.addItems(withTitles: [L("This account"), L("All %@ accounts", name)])
        scope.target = self
        scope.action = #selector(load)
        for (field, label) in [(calls, L("Calls")), (window, L("Seconds")), (concurrency, L("At once"))] {
            field.alignment = .right
            field.font = .systemFont(ofSize: 12)
            field.setAccessibilityLabel(label)
            field.translatesAutoresizingMaskIntoConstraints = false
            field.widthAnchor.constraint(equalToConstant: 64).isActive = true
        }
        func label(_ text: String) -> NSTextField { Build.label(text, font: .systemFont(ofSize: 12), color: .secondaryLabelColor) }
        let rate = Build.stack([calls, label(L("every", context: "calls every n seconds")), window, label(L("seconds"))], orientation: .horizontal, spacing: 6)
        let grid = NSGridView(views: [
            [label(L("Applies to")), scope],
            [label(L("Calls")), rate],
            [label(L("At once")), concurrency],
        ])
        grid.rowSpacing = 8
        grid.columnSpacing = 8
        grid.rowAlignment = .firstBaseline
        grid.column(at: 0).width = 76
        grid.row(at: 0).isHidden = true
        grid.translatesAutoresizingMaskIntoConstraints = false
        self.grid = grid
        contentStack.addArrangedSubview(grid)
        contentStack.addArrangedSubview(note)
        contentStack.addArrangedSubview(errorLabel)
        for view in [grid, note, errorLabel] { view.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true }
        note.isHidden = true
        errorLabel.isHidden = true
        setButtons(confirm: L("Save"))
        load()
    }

    private var service: Bool { scope.indexOfSelectedItem == 1 }

    /// Reads the selected limit from the Runner; Save waits for it.
    @objc private func load() {
        loads += 1
        let load = loads
        confirmButton.isEnabled = false
        Task { [weak self] in
            guard let self else { return }
            do {
                let limits = try await store.callLimits(pluginID, on: runnerID, service: service)
                guard load == loads else { return }
                calls.stringValue = String(limits.maxCalls)
                window.stringValue = String(limits.windowSecs)
                concurrency.stringValue = String(limits.maxConcurrency)
                grid?.row(at: 0).isHidden = !limits.sharesService
                if let retryAt = limits.retryAt, retryAt > Date() {
                    note.stringValue = L("%@ asked Lorca to slow down. Calls wait until %@.", name, Format.time(retryAt))
                    note.isHidden = false
                } else {
                    note.isHidden = true
                }
                errorLabel.isHidden = true
                confirmButton.isEnabled = true
            } catch {
                guard load == loads else { return }
                errorLabel.stringValue = error.localizedDescription
                errorLabel.isHidden = false
            }
            fitSheetToContent()
        }
    }

    override func confirmTapped() {
        guard let maxCalls = Int(calls.stringValue), (0...10_000).contains(maxCalls),
            let windowSecs = Int(window.stringValue), (1...86_400).contains(windowSecs),
            let maxConcurrency = Int(concurrency.stringValue), (0...256).contains(maxConcurrency)
        else {
            errorLabel.stringValue = L("Use whole numbers: up to 10,000 calls every 1 to 86,400 seconds, and up to 256 at once.")
            errorLabel.isHidden = false
            fitSheetToContent()
            return
        }
        let limits = CallLimits(maxCalls: maxCalls, windowSecs: windowSecs, maxConcurrency: maxConcurrency, retryAt: nil, sharesService: false)
        confirmButton.isEnabled = false
        Task { [weak self] in
            guard let self else { return }
            do {
                try await store.setCallLimits(pluginID, on: runnerID, service: service, limits: limits)
                onSaved?()
                dismiss(nil)
            } catch {
                errorLabel.stringValue = error.localizedDescription
                errorLabel.isHidden = false
                confirmButton.isEnabled = true
                fitSheetToContent()
            }
        }
    }
}
