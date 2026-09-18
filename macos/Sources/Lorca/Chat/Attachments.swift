import AppKit
import ImageIO
import UniformTypeIdentifiers

/// A file picked, dropped, or pasted into the composer, before the CLI stores it. The id is
/// minted here so the bubble the app shows right away and the CLI's copy agree.
struct OutgoingAttachment: Hashable {
    var attachment: Attachment
    var url: URL

    static let maxBytes = 20 * 1024 * 1024
    static let maxCount = 10

    enum Problem: LocalizedError {
        case notAFile(String)
        case tooLarge(String)

        var errorDescription: String? {
            switch self {
            case let .notAFile(name): "\(name) is not a file."
            case let .tooLarge(name): "\(name) is larger than \(OutgoingAttachment.maxBytes / 1024 / 1024) MB."
            }
        }
    }

    static func make(url: URL) throws -> OutgoingAttachment {
        let values = try url.resourceValues(forKeys: [.isRegularFileKey, .fileSizeKey, .contentTypeKey])
        guard values.isRegularFile == true else { throw Problem.notAFile(url.lastPathComponent) }
        let size = values.fileSize ?? 0
        guard size <= maxBytes else { throw Problem.tooLarge(url.lastPathComponent) }
        let mime = values.contentType?.preferredMIMEType ?? "application/octet-stream"

        var width: Int?
        var height: Int?
        if mime.hasPrefix("image/"),
            let source = CGImageSourceCreateWithURL(url as CFURL, nil),
            let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any]
        {
            width = properties[kCGImagePropertyPixelWidth] as? Int
            height = properties[kCGImagePropertyPixelHeight] as? Int
            // A rotated photo (EXIF orientation 5–8) is shown upright, so its box is too.
            if let orientation = properties[kCGImagePropertyOrientation] as? Int, orientation >= 5 {
                swap(&width, &height)
            }
        }

        let id = "att-" + (0..<6).map { _ in String(format: "%02x", Int.random(in: 0...255)) }.joined()
        let attachment = Attachment(id: id, name: url.lastPathComponent, mime: mime, size: size, width: width, height: height)
        return OutgoingAttachment(attachment: attachment, url: url)
    }

    /// Pasted image data becomes a PNG on disk the CLI can read.
    static func make(pastedPNG data: Data) throws -> OutgoingAttachment {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("lorca-paste", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let formatter = DateFormatter()
        formatter.dateFormat = "yyyy-MM-dd 'at' HH.mm.ss"
        let url = directory.appendingPathComponent("Pasted image \(formatter.string(from: Date())).png")
        try data.write(to: url)
        return try make(url: url)
    }
}

/// Downsampled images for bubbles and composer chips, decoded once per file.
@MainActor
enum Thumbnails {
    private static var cache: [URL: NSImage] = [:]

    static func image(at url: URL, maxPixels: Int) -> NSImage? {
        if let cached = cache[url] { return cached }
        guard let source = CGImageSourceCreateWithURL(url as CFURL, nil) else { return nil }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: maxPixels,
        ]
        guard let cgImage = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else { return nil }
        let image = NSImage(cgImage: cgImage, size: NSSize(width: cgImage.width, height: cgImage.height))
        cache[url] = image
        return image
    }
}

/// One attachment in a bubble: a thumbnail, or a card with the file's name and size.
final class AttachmentTile: NSView {
    private let image = NSImageView()
    private let card = BackgroundView()
    private let icon = NSImageView()
    private let name = Build.label("", font: .systemFont(ofSize: 12.5, weight: .medium))
    private let detail = Build.label("", font: Theme.Font.caption)
    private var url: URL?

    override var isFlipped: Bool { true }

    init() {
        super.init(frame: .zero)
        wantsLayer = true
        layer?.cornerRadius = 10
        layer?.masksToBounds = true
        image.imageScaling = .scaleProportionallyUpOrDown
        image.imageAlignment = .alignCenter
        card.cornerRadius = 10
        addSubview(image.framePositioned())
        addSubview(card.framePositioned())
        card.addSubview(icon.framePositioned())
        card.addSubview(name.framePositioned())
        card.addSubview(detail.framePositioned())
        addGestureRecognizer(NSClickGestureRecognizer(target: self, action: #selector(open)))
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func configure(_ attachment: Attachment, url: URL?, onUserBubble: Bool) {
        self.url = url
        toolTip = url == nil ? "\(attachment.name) · fetching…" : attachment.name
        let foreground: NSColor = onUserBubble ? Theme.userBubbleText : .labelColor
        let fill = onUserBubble ? NSColor.white.withAlphaComponent(0.16) : NSColor.labelColor.withAlphaComponent(0.06)
        if attachment.isImage {
            card.isHidden = true
            image.isHidden = false
            image.image = url.flatMap { Thumbnails.image(at: $0, maxPixels: Int(AttachmentLayout.imageMax * 2)) }
            // A quiet box until the bytes are here.
            layer?.backgroundColor = image.image == nil ? fill.cgColor : nil
        } else {
            card.isHidden = false
            image.isHidden = true
            image.image = nil
            layer?.backgroundColor = nil
            card.fillColor = fill
            icon.image = Glyph.symbol("doc.fill", pointSize: 18, color: foreground)
            name.stringValue = attachment.name
            name.textColor = foreground
            detail.stringValue = Attachment.sizeText(attachment.size)
            detail.textColor = foreground.withAlphaComponent(0.7)
        }
        setAccessibilityLabel(attachment.name)
    }

    override func layout() {
        super.layout()
        image.frame = bounds
        card.frame = bounds
        icon.frame = NSRect(x: 10, y: (bounds.height - 22) / 2, width: 22, height: 22)
        name.frame = NSRect(x: 40, y: 7, width: max(0, bounds.width - 48), height: 17)
        detail.frame = NSRect(x: 40, y: 25, width: max(0, bounds.width - 48), height: 14)
    }

    @objc private func open() {
        if let url { NSWorkspace.shared.open(url) }
    }
}

/// The attachments block inside a bubble, framed by `AttachmentLayout`.
final class AttachmentsView: NSView {
    struct Item {
        var attachment: Attachment
        var url: URL?
        var frame: NSRect
    }

    private var items: [Item] = []
    private var tiles: [AttachmentTile] = []

    override var isFlipped: Bool { true }

    init() {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func configure(_ items: [Item], onUserBubble: Bool) {
        self.items = items
        while tiles.count < items.count {
            let tile = AttachmentTile()
            addSubview(tile.framePositioned())
            tiles.append(tile)
        }
        while tiles.count > items.count {
            tiles.removeLast().removeFromSuperview()
        }
        for (index, item) in items.enumerated() {
            tiles[index].configure(item.attachment, url: item.url, onUserBubble: onUserBubble)
        }
        needsLayout = true
    }

    override func layout() {
        super.layout()
        for (index, item) in items.enumerated() {
            tiles[index].frame = item.frame
        }
    }
}

/// Chips for the files waiting in the composer: a thumbnail or a name card, each with a
/// remove button at its corner. Wraps into rows when the field is narrow.
final class ComposerAttachmentStrip: NSView {
    var onRemove: ((Int) -> Void)?

    private var attachments: [OutgoingAttachment] = []
    private var chips: [Chip] = []

    static let chipHeight: CGFloat = 56
    static let fileChipWidth: CGFloat = 176
    static let gap: CGFloat = 8
    /// Room for the remove button that hangs off a chip's corner.
    static let overhang: CGFloat = 6

    override var isFlipped: Bool { true }

    init() {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func configure(_ attachments: [OutgoingAttachment]) {
        self.attachments = attachments
        while chips.count < attachments.count {
            let chip = Chip()
            let index = chips.count
            chip.onRemove = { [weak self] in self?.onRemove?(index) }
            addSubview(chip.framePositioned())
            chips.append(chip)
        }
        while chips.count > attachments.count {
            chips.removeLast().removeFromSuperview()
        }
        for (index, attachment) in attachments.enumerated() {
            chips[index].configure(attachment)
            chips[index].onRemove = { [weak self] in self?.onRemove?(index) }
        }
        needsLayout = true
    }

    private func chipWidth(_ attachment: OutgoingAttachment) -> CGFloat {
        attachment.attachment.isImage ? Self.chipHeight : Self.fileChipWidth
    }

    /// Frames for the chips within `width`, wrapping, in flipped coordinates.
    private func frames(width: CGFloat) -> [NSRect] {
        var frames: [NSRect] = []
        var x: CGFloat = 0
        var y = Self.overhang
        for attachment in attachments {
            let chipWidth = min(chipWidth(attachment), width)
            if x > 0, x + chipWidth > width {
                x = 0
                y += Self.chipHeight + Self.gap
            }
            frames.append(NSRect(x: x, y: y, width: chipWidth, height: Self.chipHeight))
            x += chipWidth + Self.gap + Self.overhang
        }
        return frames
    }

    func heightThatFits(width: CGFloat) -> CGFloat {
        guard !attachments.isEmpty else { return 0 }
        return (frames(width: width).last?.maxY ?? 0)
    }

    override func layout() {
        super.layout()
        for (chip, frame) in zip(chips, frames(width: bounds.width)) {
            chip.frame = frame
        }
    }

    private final class Chip: NSView {
        var onRemove: (() -> Void)?
        private let thumbnail = NSImageView()
        private let card = BackgroundView()
        private let icon = NSImageView()
        private let name = Build.label("", font: .systemFont(ofSize: 12, weight: .medium))
        private let detail = Build.label("", font: Theme.Font.caption, color: .secondaryLabelColor)
        private let remove = NSButton()

        override var isFlipped: Bool { true }

        init() {
            super.init(frame: .zero)
            wantsLayer = true
            thumbnail.imageScaling = .scaleProportionallyUpOrDown
            thumbnail.wantsLayer = true
            thumbnail.layer?.cornerRadius = 10
            thumbnail.layer?.masksToBounds = true
            card.cornerRadius = 10
            card.fillColor = Theme.composerControl
            remove.image = Glyph.symbol("xmark", pointSize: 8, weight: .bold, color: .white)
            remove.bezelStyle = .circular
            remove.isBordered = false
            remove.wantsLayer = true
            remove.layer?.backgroundColor = NSColor.black.withAlphaComponent(0.7).cgColor
            remove.layer?.cornerRadius = 9
            remove.target = self
            remove.action = #selector(removeTapped)
            remove.toolTip = "Remove"
            addSubview(thumbnail.framePositioned())
            addSubview(card.framePositioned())
            card.addSubview(icon.framePositioned())
            card.addSubview(name.framePositioned())
            card.addSubview(detail.framePositioned())
            addSubview(remove.framePositioned())
        }

        @available(*, unavailable)
        required init?(coder: NSCoder) { fatalError() }

        func configure(_ outgoing: OutgoingAttachment) {
            toolTip = outgoing.attachment.name
            if outgoing.attachment.isImage {
                card.isHidden = true
                thumbnail.isHidden = false
                thumbnail.image = Thumbnails.image(at: outgoing.url, maxPixels: Int(ComposerAttachmentStrip.chipHeight * 2))
            } else {
                card.isHidden = false
                thumbnail.isHidden = true
                icon.image = Glyph.symbol("doc.fill", pointSize: 16, color: .secondaryLabelColor)
                name.stringValue = outgoing.attachment.name
                detail.stringValue = Attachment.sizeText(outgoing.attachment.size)
            }
        }

        override func layout() {
            super.layout()
            thumbnail.frame = bounds
            card.frame = bounds
            icon.frame = NSRect(x: 10, y: (bounds.height - 20) / 2, width: 20, height: 20)
            name.frame = NSRect(x: 36, y: 12, width: max(0, bounds.width - 52), height: 16)
            detail.frame = NSRect(x: 36, y: 29, width: max(0, bounds.width - 52), height: 14)
            remove.frame = NSRect(x: bounds.width - 12, y: -6, width: 18, height: 18)
        }

        // The remove button hangs past the chip's edge.
        override func hitTest(_ point: NSPoint) -> NSView? {
            let local = convert(point, from: superview)
            if remove.frame.contains(local) { return remove }
            return super.hitTest(point)
        }

        @objc private func removeTapped() {
            onRemove?()
        }
    }
}
