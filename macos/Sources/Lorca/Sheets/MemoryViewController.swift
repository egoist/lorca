import AppKit

/// Edits a bot's MEMORY.md, the notes that open every turn. A save carries the hash of the text
/// that was opened, so it is refused when the bot wrote in between; the user then reloads the
/// bot's version or overwrites it knowingly.
final class MemoryViewController: SheetViewController {
    private let store = AppStore.shared
    private let bot: Bot
    private var memory: BotMemory

    private let scrollView = NSScrollView()
    private let textView = NSTextView()
    private let gauge = Build.label("", font: Theme.Font.caption, color: .secondaryLabelColor, lines: 0)

    var onSaved: (() -> Void)?

    init(bot: Bot, memory: BotMemory) {
        self.bot = bot
        self.memory = memory
        super.init(
            title: L("%@'s memory", bot.name),
            subtitle: L(
                "MEMORY.md opens at the start of every turn: the first %d lines or %@, whichever cuts first. Longer notes belong in memory/<topic>.md files the bot reads on demand.",
                memory.maxLines, Format.kilobytes(memory.maxBytes)),
            width: 560
        )
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()

        textView.isRichText = false
        textView.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
        textView.textColor = .labelColor
        textView.isAutomaticQuoteSubstitutionEnabled = false
        textView.isAutomaticDashSubstitutionEnabled = false
        textView.isAutomaticTextReplacementEnabled = false
        textView.allowsUndo = true
        textView.textContainerInset = NSSize(width: 6, height: 8)
        textView.isVerticallyResizable = true
        textView.isHorizontallyResizable = false
        textView.autoresizingMask = [.width]
        textView.textContainer?.widthTracksTextView = true
        textView.delegate = self
        textView.string = memory.text

        scrollView.documentView = textView
        scrollView.hasVerticalScroller = true
        scrollView.borderType = .bezelBorder
        scrollView.translatesAutoresizingMaskIntoConstraints = false

        contentStack.addArrangedSubview(scrollView)
        contentStack.addArrangedSubview(gauge)
        NSLayoutConstraint.activate([
            scrollView.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            scrollView.heightAnchor.constraint(equalToConstant: 340),
            gauge.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
        ])

        setButtons(confirm: L("Save"))
        updateGauge()
    }

    override func viewDidAppear() {
        super.viewDidAppear()
        view.window?.makeFirstResponder(textView)
    }

    /// Lines and bytes against the load budget; amber from 80%, red once anything stops loading.
    private func updateGauge() {
        let text = textView.string
        let lines = text.isEmpty ? 0 : text.split(separator: "\n", omittingEmptySubsequences: false).count
        let bytes = text.utf8.count
        let overLines = lines > memory.maxLines
        let overBytes = bytes > memory.maxBytes
        var status =
            lines == 1
            ? L("%d line of %d · %@ of %@", lines, memory.maxLines, Format.kilobytes(bytes), Format.kilobytes(memory.maxBytes))
            : L("%d lines of %d · %@ of %@", lines, memory.maxLines, Format.kilobytes(bytes), Format.kilobytes(memory.maxBytes))
        if overLines || overBytes {
            let hidden = overLines ? lines - memory.maxLines : 0
            status += " · " + (hidden > 0
                ? (hidden == 1
                    ? L("%d line past the budget will not load", hidden)
                    : L("%d lines past the budget will not load", hidden))
                : L("the end will not load"))
            gauge.textColor = .systemRed
        } else if Double(lines) >= Double(memory.maxLines) * 0.8 || Double(bytes) >= Double(memory.maxBytes) * 0.8 {
            gauge.textColor = .systemOrange
        } else {
            gauge.textColor = .secondaryLabelColor
        }
        gauge.stringValue = status
    }

    override func confirmTapped() {
        save(expectedHash: memory.hash)
    }

    private func save(expectedHash: String?) {
        let text = textView.string
        confirmButton.isEnabled = false
        Task { [weak self] in
            guard let self else { return }
            do {
                _ = try await self.store.writeBotMemory(self.bot.id, text: text, expectedHash: expectedHash)
                self.onSaved?()
                self.dismiss(nil)
            } catch {
                self.confirmButton.isEnabled = true
                if error.localizedDescription.contains("changed since") {
                    self.resolveConflict()
                } else {
                    let alert = NSAlert()
                    alert.messageText = L("Couldn't save the memory")
                    alert.informativeText = error.localizedDescription
                    alert.beginSheetModal(for: self.view.window!, completionHandler: nil)
                }
            }
        }
    }

    /// The bot wrote while the user was editing: show the bot's version, or write over it.
    private func resolveConflict() {
        let alert = NSAlert()
        alert.messageText = L("%@ changed this file while you were editing", bot.name)
        alert.informativeText = L("Reload shows %@'s version and discards your draft. Overwrite saves yours over it.", bot.name)
        alert.addButton(withTitle: L("Reload"))
        alert.addButton(withTitle: L("Overwrite with mine"))
        alert.addButton(withTitle: L("Cancel"))
        guard let window = view.window else { return }
        alert.beginSheetModal(for: window) { [weak self] response in
            guard let self else { return }
            switch response {
            case .alertFirstButtonReturn:
                Task { [weak self] in
                    guard let self, let fresh = try? await self.store.botMemory(self.bot.id) else { return }
                    self.memory = fresh
                    self.textView.string = fresh.text
                    self.updateGauge()
                }
            case .alertSecondButtonReturn:
                self.save(expectedHash: nil)
            default:
                break
            }
        }
    }
}

extension MemoryViewController: NSTextViewDelegate {
    func textDidChange(_ notification: Notification) {
        updateGauge()
    }
}
