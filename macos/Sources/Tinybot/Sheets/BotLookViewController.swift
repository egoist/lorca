import AppKit
import UniformTypeIdentifiers

/// How a bot looks: a symbol on an accent gradient, or an image of the user's own. The image
/// wins while it is set; the symbol and accent stay underneath for when it is removed and for
/// Devices that have not fetched it yet.
final class BotLookViewController: SheetViewController {
    /// The symbols offered, the same list the phone app shows.
    static let symbols: [String] = [
        "sparkles", "wand.and.stars", "hammer.fill", "book.fill",
        "paintbrush.fill", "chart.bar.fill", "terminal.fill", "globe",
        "brain.head.profile", "magnifyingglass", "envelope.fill", "calendar",
        "flask.fill", "bolt.fill", "leaf.fill", "shield.fill",
        "binoculars.fill", "chevron.left.forwardslash.chevron.right", "pencil.and.scribble", "bolt.horizontal.fill",
        "flame.fill",
    ]

    /// Longest side of a stored profile image. Small enough to sync in a moment, large enough
    /// for the biggest avatar any screen draws.
    static let imageSide: CGFloat = 512

    private enum ImageChange {
        case keep
        case remove
        case set(URL, NSImage)
    }

    private let store = AppStore.shared
    private let botID: Bot.ID
    private var symbolName: String
    private var accent: Accent
    private var imageChange: ImageChange = .keep

    private let preview = AvatarView(diameter: 72)
    private let symbolGrid = NSGridView()
    private let accentRow = Build.stack([], orientation: .horizontal, spacing: 8)
    private let chooseButton = NSButton()
    private let removeButton = NSButton()
    private let caption = Build.label("", font: Theme.Font.caption, color: .tertiaryLabelColor, lines: 0)
    private var symbolTiles: [LookTile] = []
    private var accentTiles: [LookTile] = []

    init(botID: Bot.ID) {
        self.botID = botID
        let bot = AppStore.shared.bot(botID)
        symbolName = bot?.symbolName ?? "sparkles"
        accent = bot?.accent ?? .indigo
        super.init(
            title: "Look",
            subtitle: "Pick a symbol and a color, or use an image of your own. Paired Devices see the same look.",
            width: 400
        )
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    private var bot: Bot? { store.bot(botID) }

    /// Whether the saved look, with the pending change applied, has an image.
    private var hasImage: Bool {
        switch imageChange {
        case .keep: bot?.avatar != nil
        case .remove: false
        case .set: true
        }
    }

    override func loadView() {
        super.loadView()

        let previewRow = NSView()
        previewRow.translatesAutoresizingMaskIntoConstraints = false
        previewRow.addSubview(preview)
        NSLayoutConstraint.activate([
            preview.centerXAnchor.constraint(equalTo: previewRow.centerXAnchor),
            preview.topAnchor.constraint(equalTo: previewRow.topAnchor),
            preview.bottomAnchor.constraint(equalTo: previewRow.bottomAnchor),
        ])

        buildSymbolGrid()
        buildAccentRow()

        // The grid keeps its own width inside a full-width row; stretched to the sheet, a grid
        // spreads its columns out.
        let symbolRow = NSView()
        symbolRow.translatesAutoresizingMaskIntoConstraints = false
        symbolRow.addSubview(symbolGrid)
        NSLayoutConstraint.activate([
            symbolGrid.leadingAnchor.constraint(equalTo: symbolRow.leadingAnchor),
            symbolGrid.trailingAnchor.constraint(lessThanOrEqualTo: symbolRow.trailingAnchor),
            symbolGrid.topAnchor.constraint(equalTo: symbolRow.topAnchor),
            symbolGrid.bottomAnchor.constraint(equalTo: symbolRow.bottomAnchor),
        ])

        chooseButton.title = "Choose Image…"
        chooseButton.bezelStyle = .rounded
        chooseButton.controlSize = .regular
        chooseButton.target = self
        chooseButton.action = #selector(chooseImage)
        chooseButton.translatesAutoresizingMaskIntoConstraints = false

        removeButton.title = "Remove Image"
        removeButton.bezelStyle = .rounded
        removeButton.controlSize = .regular
        removeButton.target = self
        removeButton.action = #selector(removeImage)
        removeButton.translatesAutoresizingMaskIntoConstraints = false

        let imageRow = Build.stack([chooseButton, removeButton], orientation: .horizontal, spacing: 8)

        let rows: [NSView] = [
            previewRow,
            heading("Symbol"),
            symbolRow,
            heading("Color"),
            accentRow,
            heading("Image"),
            imageRow,
            caption,
        ]
        for row in rows {
            contentStack.addArrangedSubview(row)
            row.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
        }
        contentStack.setCustomSpacing(18, after: previewRow)
        contentStack.setCustomSpacing(6, after: rows[1])
        contentStack.setCustomSpacing(16, after: symbolRow)
        contentStack.setCustomSpacing(6, after: rows[3])
        contentStack.setCustomSpacing(16, after: accentRow)
        contentStack.setCustomSpacing(6, after: rows[5])

        setButtons(confirm: "Save")
        refresh()
    }

    private func heading(_ text: String) -> NSView {
        Build.label(text.uppercased(), font: .systemFont(ofSize: 10, weight: .semibold), color: .tertiaryLabelColor)
    }

    private func buildSymbolGrid() {
        let perRow = 8
        symbolGrid.translatesAutoresizingMaskIntoConstraints = false
        symbolGrid.rowSpacing = 6
        symbolGrid.columnSpacing = 6
        symbolGrid.xPlacement = .leading
        symbolGrid.yPlacement = .center
        for name in Self.symbols {
            let tile = LookTile(size: NSSize(width: 38, height: 34), cornerRadius: 8)
            tile.symbolName = name
            tile.symbolPointSize = 15
            tile.toolTip = name
            tile.onClick = { [weak self] in
                self?.symbolName = name
                self?.refresh()
            }
            symbolTiles.append(tile)
        }
        // Whole rows at a time: a grid grows its columns from the longest row it is given.
        for start in stride(from: 0, to: symbolTiles.count, by: perRow) {
            symbolGrid.addRow(with: Array(symbolTiles[start..<min(start + perRow, symbolTiles.count)]))
        }
    }

    private func buildAccentRow() {
        for accent in Accent.allCases {
            let tile = LookTile(size: NSSize(width: 26, height: 26), cornerRadius: 13)
            tile.fill = accent.color
            tile.toolTip = accent.rawValue.capitalized
            tile.onClick = { [weak self] in
                self?.accent = accent
                self?.refresh()
            }
            accentTiles.append(tile)
            accentRow.addArrangedSubview(tile)
        }
    }

    private func refresh() {
        switch imageChange {
        case let .set(_, image):
            preview.content = .image(image)
        case .remove:
            preview.content = .bot(symbolName: symbolName, accent: accent)
        case .keep:
            if let bot, let image = store.avatarImage(for: bot) {
                preview.content = .image(image)
            } else {
                preview.content = .bot(symbolName: symbolName, accent: accent)
            }
        }

        for (index, tile) in symbolTiles.enumerated() {
            let selected = Self.symbols[index] == symbolName
            tile.fill = selected ? accent.color : NSColor.labelColor.withAlphaComponent(0.06)
            tile.symbolColor = selected ? .white : .labelColor
        }
        for (index, tile) in accentTiles.enumerated() {
            tile.borderWidth = Accent.allCases[index] == accent ? 2.5 : 0
        }

        removeButton.isHidden = !hasImage
        caption.stringValue = hasImage
            ? "The image shows in place of the symbol and color. It is resized to \(Int(Self.imageSide)) px and shared encrypted, like an attachment."
            : "Images are resized to \(Int(Self.imageSide)) px and shared encrypted, like an attachment."
        fitSheetToContent()
    }

    @objc private func chooseImage() {
        guard let window = view.window else { return }
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.image]
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        panel.message = "Choose an image for this bot."
        panel.beginSheetModal(for: window) { [weak self] response in
            guard let self, response == .OK, let url = panel.url else { return }
            guard let prepared = Self.prepare(imageAt: url) else {
                let alert = NSAlert()
                alert.messageText = "That file could not be read as an image."
                alert.runModal()
                return
            }
            imageChange = .set(prepared.url, prepared.image)
            refresh()
        }
    }

    @objc private func removeImage() {
        imageChange = .remove
        refresh()
    }

    override func confirmTapped() {
        guard let bot else {
            dismiss(nil)
            return
        }
        if symbolName != bot.symbolName || accent != bot.accent {
            store.setBotLook(botID, symbolName: symbolName, accent: accent)
        }
        switch imageChange {
        case .keep:
            break
        case .remove:
            store.setBotAvatar(botID, fileURL: nil)
        case let .set(url, _):
            store.setBotAvatar(botID, fileURL: url)
        }
        dismiss(nil)
    }

    /// A square, center-cropped PNG of at most `imageSide` px, written to a temporary file the
    /// CLI copies into its store. The bytes that leave the Mac are these, not the original.
    static func prepare(imageAt url: URL) -> (url: URL, image: NSImage)? {
        guard let source = NSImage(contentsOf: url), source.isValid else { return nil }
        guard let representation = source.representations.max(by: { $0.pixelsWide < $1.pixelsWide }) else { return nil }
        let pixelWidth = CGFloat(representation.pixelsWide)
        let pixelHeight = CGFloat(representation.pixelsHigh)
        guard pixelWidth > 0, pixelHeight > 0 else { return nil }

        let side = Int(min(imageSide, min(pixelWidth, pixelHeight)))
        guard
            let bitmap = NSBitmapImageRep(
                bitmapDataPlanes: nil, pixelsWide: side, pixelsHigh: side, bitsPerSample: 8, samplesPerPixel: 4,
                hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)
        else { return nil }
        bitmap.size = NSSize(width: side, height: side)

        NSGraphicsContext.saveGraphicsState()
        guard let context = NSGraphicsContext(bitmapImageRep: bitmap) else { return nil }
        NSGraphicsContext.current = context
        context.imageInterpolation = .high
        // Aspect-fill the square from the middle of the picture.
        let scale = CGFloat(side) / min(pixelWidth, pixelHeight)
        let drawn = NSSize(width: pixelWidth * scale, height: pixelHeight * scale)
        let origin = NSPoint(x: (CGFloat(side) - drawn.width) / 2, y: (CGFloat(side) - drawn.height) / 2)
        source.draw(in: NSRect(origin: origin, size: drawn), from: .zero, operation: .copy, fraction: 1)
        NSGraphicsContext.restoreGraphicsState()

        guard let data = bitmap.representation(using: .png, properties: [:]) else { return nil }
        let target = FileManager.default.temporaryDirectory
            .appendingPathComponent("tinybot-avatar-\(UUID().uuidString.lowercased().prefix(8)).png")
        do {
            try data.write(to: target, options: .atomic)
        } catch {
            return nil
        }
        let image = NSImage(size: bitmap.size)
        image.addRepresentation(bitmap)
        return (target, image)
    }
}

/// A clickable rounded tile the sheet draws itself. Its frame is exactly its size, where an
/// NSButton's frame carries its bezel's alignment insets even with no border, which ate the
/// grid's row gap.
final class LookTile: NSView {
    private let size: NSSize
    private let cornerRadius: CGFloat
    var onClick: (() -> Void)?

    var fill: NSColor = .clear { didSet { needsDisplay = true } }
    var borderWidth: CGFloat = 0 { didSet { needsDisplay = true } }
    var symbolName: String? { didSet { needsDisplay = true } }
    var symbolPointSize: CGFloat = 15 { didSet { needsDisplay = true } }
    var symbolColor: NSColor = .labelColor { didSet { needsDisplay = true } }

    init(size: NSSize, cornerRadius: CGFloat) {
        self.size = size
        self.cornerRadius = cornerRadius
        super.init(frame: NSRect(origin: .zero, size: size))
        translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            widthAnchor.constraint(equalToConstant: size.width),
            heightAnchor.constraint(equalToConstant: size.height),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var intrinsicContentSize: NSSize { size }
    override var allowsVibrancy: Bool { false }

    override func resetCursorRects() {
        super.resetCursorRects()
        addCursorRect(bounds, cursor: .pointingHand)
    }

    override func mouseDown(with event: NSEvent) {}

    override func mouseUp(with event: NSEvent) {
        if bounds.contains(convert(event.locationInWindow, from: nil)) { onClick?() }
    }

    override func draw(_ dirtyRect: NSRect) {
        let path = NSBezierPath(roundedRect: bounds, xRadius: cornerRadius, yRadius: cornerRadius)
        fill.setFill()
        path.fill()
        if borderWidth > 0 {
            let inset = NSBezierPath(
                roundedRect: bounds.insetBy(dx: borderWidth / 2, dy: borderWidth / 2),
                xRadius: max(0, cornerRadius - borderWidth / 2), yRadius: max(0, cornerRadius - borderWidth / 2))
            inset.lineWidth = borderWidth
            NSColor.labelColor.setStroke()
            inset.stroke()
        }
        if let symbolName, let image = Glyph.symbol(symbolName, pointSize: symbolPointSize, weight: .semibold, color: symbolColor) {
            let imageSize = image.size
            image.draw(in: NSRect(x: bounds.midX - imageSize.width / 2, y: bounds.midY - imageSize.height / 2, width: imageSize.width, height: imageSize.height))
        }
    }
}
