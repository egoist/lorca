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
        let segments = Markdown.segments(text, textColor: .labelColor)
        guard !segments.isEmpty else { return }

        let natural = segments.map { segment -> CGFloat in
            switch segment {
            case let .text(attributed): TextMeasure.width(of: attributed)
            case let .code(attributed, _): TextMeasure.width(of: attributed) + Markdown.codePaddingX * 2
            }
        }.max() ?? 0
        let width = min(maxWidth, max(minWidth, ceil(natural)))
        let height = segments.enumerated().reduce(CGFloat(0)) { sum, item in
            let gap: CGFloat = item.offset < segments.count - 1 ? Markdown.segmentSpacing : 0
            switch item.element {
            case let .text(attributed):
                return sum + TextMeasure.labelSize(of: attributed, width: width).height + gap
            case let .code(attributed, _):
                return sum + TextMeasure.labelSize(of: attributed, width: width - Markdown.codePaddingX * 2).height
                    + Markdown.codePaddingY * 2 + gap
            }
        }

        let content = SegmentedTextView().framePositioned()
        content.configure(segments)
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
