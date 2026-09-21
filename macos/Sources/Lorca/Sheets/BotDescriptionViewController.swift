import AppKit

/// The bot's full behavioral description, opened from the compact Profile row in the inspector.
/// Saving publishes the changed profile, so the bot's next turn and every paired Device see it.
final class BotDescriptionViewController: SheetViewController {
    private let store = AppStore.shared
    private let botID: Bot.ID
    private let scrollView = NSScrollView()
    private let textView = NSTextView()

    init(botID: Bot.ID) {
        self.botID = botID
        super.init(title: L("Description"), subtitle: L("What it does and how it should work"), width: 520)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()

        textView.isRichText = false
        textView.font = .systemFont(ofSize: 13)
        textView.textColor = .labelColor
        textView.allowsUndo = true
        textView.textContainerInset = NSSize(width: 8, height: 8)
        textView.isVerticallyResizable = true
        textView.isHorizontallyResizable = false
        textView.autoresizingMask = [.width]
        textView.textContainer?.widthTracksTextView = true
        textView.string = store.bot(botID)?.description ?? ""

        scrollView.documentView = textView
        scrollView.hasVerticalScroller = true
        scrollView.borderType = .bezelBorder
        scrollView.translatesAutoresizingMaskIntoConstraints = false

        contentStack.addArrangedSubview(scrollView)
        NSLayoutConstraint.activate([
            scrollView.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            scrollView.heightAnchor.constraint(equalToConstant: 280),
        ])

        setButtons(confirm: L("Save"))
    }

    override func viewDidAppear() {
        super.viewDidAppear()
        view.window?.makeFirstResponder(textView)
    }

    override func confirmTapped() {
        guard let bot = store.bot(botID) else {
            dismiss(nil)
            return
        }
        let description = textView.string.trimmingCharacters(in: .whitespacesAndNewlines)
        if description != bot.description {
            store.updateBot(bot.id, name: bot.name, label: bot.label, description: description)
        }
        dismiss(nil)
    }
}
