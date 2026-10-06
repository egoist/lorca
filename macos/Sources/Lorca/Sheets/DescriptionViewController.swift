import AppKit

/// A description edited in a sheet of its own, opened from a compact row in the inspector: a
/// bot's (what it does and how it should work) or a group's (what it is for). Saving publishes
/// it, so the next turn and every paired Device see it.
final class DescriptionViewController: SheetViewController {
    private let initialText: String
    private let save: (String) -> Void
    private let scrollView = NSScrollView()
    private let textView = NSTextView()

    init(subtitle: String, text: String, save: @escaping (String) -> Void) {
        initialText = text
        self.save = save
        super.init(title: L("Description"), subtitle: subtitle, width: 520)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    /// The bot's full behavioral description.
    static func bot(_ botID: Bot.ID) -> DescriptionViewController {
        let store = AppStore.shared
        return DescriptionViewController(subtitle: L("What it does and how it should work"), text: store.bot(botID)?.description ?? "") { description in
            guard let bot = store.bot(botID), description != bot.description else { return }
            store.updateBot(bot.id, name: bot.name, description: description)
        }
    }

    /// What a group is for, which every bot in it reads.
    static func group(_ chatID: Chat.ID) -> DescriptionViewController {
        let store = AppStore.shared
        return DescriptionViewController(subtitle: L("What this group is for. Every bot in it reads this."), text: store.chat(chatID)?.groupDescription ?? "") { description in
            store.setDescription(description, of: chatID)
        }
    }

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
        textView.string = initialText

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
        save(textView.string.trimmingCharacters(in: .whitespacesAndNewlines))
        dismiss(nil)
    }
}
