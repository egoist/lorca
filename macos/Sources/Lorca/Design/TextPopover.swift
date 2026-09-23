import AppKit

/// The full text behind a one-line marker, in a transient popover anchored to that marker. The
/// text renders like a message body (paragraphs and fenced code), scrolls past a screenful,
/// and sizes to its content up to a bubble's width.
enum TextPopover {
    static let maxWidth: CGFloat = 440
    static let minWidth: CGFloat = 160
    static let maxHeight: CGFloat = 360
    static let padding: CGFloat = 14

    @MainActor private static var current: NSPopover?

    @MainActor
    static func show(_ text: String, relativeTo rect: NSRect, of view: NSView, preferredEdge: NSRectEdge = .maxY) {
        current?.close()
        let rendered = RenderedMessage(text, textColor: .labelColor)
        guard !rendered.isEmpty else { return }

        let layout = rendered.layout(fitting: maxWidth, minimum: minWidth)
        let width = layout.width
        let height = layout.height

        let content = SegmentedTextView().framePositioned()
        content.configure(rendered.segments, layout: layout, textColor: .labelColor)
        content.frame = NSRect(x: padding, y: padding, width: width, height: height)

        // A padded document view, not scroll-view insets: insets shift the scroll range, not the text.
        let document = PaddedDocumentView(
            frame: NSRect(x: 0, y: 0, width: width + padding * 2, height: height + padding * 2))
        document.addSubview(content)

        let scroll = NSScrollView()
        scroll.drawsBackground = false
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.documentView = document

        let controller = NSViewController()
        controller.view = scroll
        controller.preferredContentSize = NSSize(
            width: width + padding * 2, height: min(maxHeight, height + padding * 2))

        let popover = NSPopover()
        popover.behavior = .transient
        popover.contentViewController = controller
        popover.show(relativeTo: rect, of: view, preferredEdge: preferredEdge)
        current = popover
    }

    /// Flipped so the text starts at the top of the scroll range.
    private final class PaddedDocumentView: NSView {
        override var isFlipped: Bool { true }
    }

    /// The first non-empty line of `text`, with fence markers skipped: what a one-line preview shows.
    static func firstLine(of text: String) -> String {
        text.split(whereSeparator: \.isNewline)
            .lazy
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .first { !$0.isEmpty && !$0.hasPrefix("```") } ?? ""
    }
}
