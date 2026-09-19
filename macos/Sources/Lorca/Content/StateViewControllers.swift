import AppKit

/// Shown when the local CLI is not answering on 127.0.0.1.
final class OfflineViewController: NSViewController {
    var onRetry: (() -> Void)?

    private let command = "lorca serve"
    private let hint = Build.label("", font: Theme.Font.caption, color: .tertiaryLabelColor, lines: 0, alignment: .center)

    func refreshHint() {
        hint.stringValue = "\(AppStore.shared.offlineStatus) · 127.0.0.1:\(Preferences.cliPort)"
    }

    override func loadView() {
        let container = BackgroundView()
        container.fillColor = Theme.transcriptBackground
        container.cornerRadius = 0

        let icon = NSImageView()
        icon.image = NSImage(systemSymbolName: "bolt.horizontal.circle", accessibilityDescription: nil)
        icon.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 40, weight: .regular)
        icon.contentTintColor = .tertiaryLabelColor
        icon.translatesAutoresizingMaskIntoConstraints = false

        let title = Build.label(
            L("The Lorca CLI isn't answering"), font: .systemFont(ofSize: 18, weight: .semibold),
            alignment: .center)
        let body = Build.label(
            L("Your bots, keys and transcripts live in the CLI on this Mac. The app starts it on its own; you can also run it from a terminal, and this window reconnects either way."),
            font: .systemFont(ofSize: 12.5), color: .secondaryLabelColor, lines: 0, alignment: .center
        )

        let commandBox = BackgroundView()
        commandBox.cornerRadius = 8
        commandBox.fillColor = Theme.codeBackground
        let commandLabel = Build.label("$ \(command)", font: Theme.Font.code, color: .labelColor)
        commandLabel.isSelectable = true
        let copyButton = Build.imageButton(
            symbol: "doc.on.doc", pointSize: 12, tooltip: L("Copy command"), target: self,
            action: #selector(copyCommand))
        commandBox.addSubview(commandLabel)
        commandBox.addSubview(copyButton)

        let retry = NSButton(title: L("Retry Connection"), target: self, action: #selector(retry))
        retry.bezelStyle = .rounded
        retry.keyEquivalent = "\r"
        retry.controlSize = .large
        retry.translatesAutoresizingMaskIntoConstraints = false

        hint.font = Theme.Font.caption
        hint.textColor = .tertiaryLabelColor
        hint.alignment = .center
        hint.lineBreakMode = .byWordWrapping
        hint.maximumNumberOfLines = 0
        refreshHint()

        let column = Build.stack([icon, title, body, commandBox, retry, hint], spacing: 12)
        column.alignment = .centerX
        column.setCustomSpacing(18, after: icon)
        column.setCustomSpacing(20, after: body)
        column.setCustomSpacing(20, after: commandBox)

        container.addSubview(column)
        NSLayoutConstraint.activate([
            column.centerXAnchor.constraint(equalTo: container.centerXAnchor),
            column.centerYAnchor.constraint(equalTo: container.centerYAnchor),
            column.widthAnchor.constraint(equalToConstant: 420),
            body.widthAnchor.constraint(equalToConstant: 380),
            hint.widthAnchor.constraint(equalToConstant: 380),

            commandLabel.leadingAnchor.constraint(equalTo: commandBox.leadingAnchor, constant: 12),
            commandLabel.topAnchor.constraint(equalTo: commandBox.topAnchor, constant: 9),
            commandLabel.bottomAnchor.constraint(equalTo: commandBox.bottomAnchor, constant: -9),
            copyButton.leadingAnchor.constraint(equalTo: commandLabel.trailingAnchor, constant: 14),
            copyButton.trailingAnchor.constraint(equalTo: commandBox.trailingAnchor, constant: -10),
            copyButton.centerYAnchor.constraint(equalTo: commandBox.centerYAnchor),
        ])

        view = container
        AppStore.shared.observe(self) { [weak self] event in
            if case .connectionChanged = event { self?.refreshHint() }
        }
    }

    @objc private func copyCommand() {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(command, forType: .string)
    }

    @objc private func retry() {
        onRetry?()
    }
}

/// Shown when nothing is selected in the sidebar.
final class PlaceholderViewController: NSViewController {
    var onNewBot: (() -> Void)?

    override func loadView() {
        let container = BackgroundView()
        container.fillColor = Theme.transcriptBackground
        container.cornerRadius = 0

        let icon = NSImageView()
        icon.image = NSImage(systemSymbolName: "bubble.left.and.bubble.right", accessibilityDescription: nil)
        icon.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 34, weight: .regular)
        icon.contentTintColor = .tertiaryLabelColor
        icon.translatesAutoresizingMaskIntoConstraints = false

        let title = Build.label(
            L("No chat selected"), font: .systemFont(ofSize: 16, weight: .semibold), alignment: .center)
        let body = Build.label(
            L("Pick a conversation in the sidebar, or start a new one."),
            font: .systemFont(ofSize: 12.5), color: .secondaryLabelColor, alignment: .center)

        let button = NSButton(title: L("New Bot…"), target: self, action: #selector(newBot))
        button.bezelStyle = .rounded
        button.controlSize = .large

        let column = Build.stack([icon, title, body, button], spacing: 10)
        column.alignment = .centerX
        column.setCustomSpacing(16, after: icon)
        column.setCustomSpacing(18, after: body)

        container.addSubview(column)
        NSLayoutConstraint.activate([
            column.centerXAnchor.constraint(equalTo: container.centerXAnchor),
            column.centerYAnchor.constraint(equalTo: container.centerYAnchor),
        ])

        view = container
    }

    @objc private func newBot() {
        onNewBot?()
    }
}
