import AppKit
import LorcaMarkdown

// MARK: - Segments

/// A table's cells, already styled, header row first.
struct TableContent {
    let cells: [[NSAttributedString]]
    let columns: Int
}

/// A message is text runs and, between them, the tables that get a grid of their own. A text
/// run carries the room a code box needs when it opens or closes the run.
enum MessageSegment {
    case text(NSAttributedString, topInset: CGFloat, bottomInset: CGFloat)
    case table(TableContent)
}

extension NSAttributedString.Key {
    /// Marks a fenced code block; the value is the x where its box starts.
    static let codeBlock = NSAttributedString.Key("lorca.codeBlock")
    /// Marks quoted text; the value is the x of its bar.
    static let quoteBar = NSAttributedString.Key("lorca.quoteBar")
}

/// Message Markdown, parsed by the linked `lorca-markdown` crate and styled here: fonts and
/// paragraph styles per block, custom attributes where the layout manager paints boxes and
/// bars, tables as grids. An unclosed marker mid-stream stays literal, as CommonMark reads it.
enum Markdown {
    /// Every block sits a blank line's worth from the next, tables included; only a code
    /// box, which carries its own padding, sits closer.
    static let paragraphGap: CGFloat = 24
    static let segmentSpacing: CGFloat = paragraphGap
    static let codeGap: CGFloat = 8
    static let itemGap: CGFloat = 2
    static let lineSpacing: CGFloat = 2.5
    static let codePaddingX: CGFloat = 10
    static let codePaddingY: CGFloat = 8
    static let codeCorner: CGFloat = 7
    static let listIndent: CGFloat = 18
    static let quoteIndent: CGFloat = 10
    static let quoteBar: CGFloat = 2
    static let cellPaddingX: CGFloat = 10
    static let cellPaddingY: CGFloat = 6
    static let columnFloor: CGFloat = 64

    static func segments(_ raw: String, textColor: NSColor) -> [MessageSegment] {
        MarkdownRenderer(textColor: textColor).render(parseMarkdown(text: raw))
    }

    /// Links on the accent-colored user bubble stay the bubble's text color.
    static func linkColor(for textColor: NSColor) -> NSColor {
        textColor == Theme.userBubbleText ? textColor : .linkColor
    }

    static var tableRule: NSColor { .separatorColor }
    static var tableHeaderRule: NSColor { .tertiaryLabelColor }

    /// The room after a block: a code box sits close to its neighbors, paragraphs a line apart.
    static func gap(after block: Block, before next: Block) -> CGFloat {
        if case .code = block { return codeGap }
        if case .code = next { return codeGap }
        return paragraphGap
    }
}

// MARK: - Renderer

final class MarkdownRenderer {
    private let textColor: NSColor
    private let linkColor: NSColor
    private var out = NSMutableAttributedString()
    private var topInset: CGFloat = 0
    private var bottomInset: CGFloat = 0
    private var segments: [MessageSegment] = []

    /// Where a block sits: its indents and the bar or marker it inherits.
    private struct Context {
        var indent: CGFloat = 0
        var quoteX: CGFloat? = nil
        var marker: String? = nil
        var markerX: CGFloat = 0
    }

    init(textColor: NSColor) {
        self.textColor = textColor
        linkColor = Markdown.linkColor(for: textColor)
    }

    func render(_ document: Document) -> [MessageSegment] {
        let blocks = document.blocks
        for (index, block) in blocks.enumerated() {
            if case let .table(alignments, header, rows) = block {
                flush()
                segments.append(.table(table(alignments: alignments, header: header, rows: rows)))
                continue
            }
            let next = index + 1 < blocks.count ? blocks[index + 1] : nil
            var after: CGFloat = 0
            if let next {
                if case .table = next { after = 0 } else { after = Markdown.gap(after: block, before: next) }
            }
            self.block(block, Context(), after: after)
        }
        flush()
        return segments
    }

    private func flush() {
        guard out.length > 0 else { return }
        segments.append(.text(out, topInset: topInset, bottomInset: bottomInset))
        out = NSMutableAttributedString()
        topInset = 0
        bottomInset = 0
    }

    private func table(alignments: [Align], header: [Cell], rows: [TableRow]) -> TableContent {
        let all = [header] + rows.map(\.cells)
        let columns = all.map(\.count).max() ?? 0
        let cells = all.enumerated().map { r, row in
            (0..<columns).map { c in
                cell(c < row.count ? row[c].spans : [], header: r == 0, align: c < alignments.count ? alignments[c] : .auto)
            }
        }
        return TableContent(cells: cells, columns: columns)
    }

    private func cell(_ spans: [Span], header: Bool, align: Align) -> NSAttributedString {
        let paragraph = NSMutableParagraphStyle()
        paragraph.lineSpacing = 2
        paragraph.lineBreakMode = .byWordWrapping
        switch align {
        case .center: paragraph.alignment = .center
        case .right: paragraph.alignment = .right
        case .left, .auto: paragraph.alignment = .natural
        }
        let size = Theme.Font.message.pointSize - 0.5
        let font: NSFont = header ? .systemFont(ofSize: size, weight: .semibold) : .systemFont(ofSize: size)
        let text = NSMutableAttributedString()
        for span in spans {
            text.append(NSAttributedString(string: span.text, attributes: inline(span, font: font, bold: header, paragraph: paragraph)))
        }
        if text.length == 0 {
            text.append(NSAttributedString(string: " ", attributes: [.font: font, .paragraphStyle: paragraph]))
        }
        return text
    }

    private func blocks(_ blocks: [Block], _ context: Context, after: CGFloat) {
        for (index, block) in blocks.enumerated() {
            var context = context
            if index > 0 { context.marker = nil }
            let last = index == blocks.count - 1
            self.block(block, context, after: last ? after : Markdown.gap(after: block, before: blocks[index + 1]))
        }
    }

    private func block(_ block: Block, _ context: Context, after: CGFloat) {
        switch block {
        case let .paragraph(spans):
            paragraph(spans, context, font: Theme.Font.message, after: after)
        case let .heading(level, spans):
            let size = level <= 2 ? Theme.Font.message.pointSize + 2 : Theme.Font.message.pointSize
            paragraph(spans, context, font: .systemFont(ofSize: size, weight: .bold), after: after, bold: true)
        case let .code(_, text):
            code(text, context, after: after)
        case let .listing(ordered, start, items):
            for (index, item) in items.enumerated() {
                var inner = context
                inner.marker = marker(ordered: ordered, number: Int(start) + index, checked: item.checked)
                inner.markerX = context.indent
                inner.indent = context.indent + Markdown.listIndent
                let last = index == items.count - 1
                if item.blocks.isEmpty {
                    paragraph([], inner, font: Theme.Font.message, after: last ? after : Markdown.itemGap)
                } else {
                    blocks(item.blocks, inner, after: last ? after : Markdown.itemGap)
                }
            }
        case let .quote(blocks):
            var inner = context
            inner.quoteX = context.indent
            inner.indent = context.indent + Markdown.quoteIndent
            self.blocks(blocks, inner, after: after)
        case let .table(_, header, rows):
            // A table inside a list or quote: its rows as lines, cells three spaces apart.
            paragraph(cells(header), context, font: Theme.Font.message, after: rows.isEmpty ? after : Markdown.itemGap, bold: true)
            for (index, row) in rows.enumerated() {
                paragraph(cells(row.cells), context, font: Theme.Font.message, after: index == rows.count - 1 ? after : Markdown.itemGap)
            }
        case .rule:
            let rule = Span(text: String(repeating: "\u{2500}", count: 24), bold: false, italic: false, code: false, strike: false, link: nil)
            paragraph([rule], context, font: Theme.Font.message, after: after, color: .tertiaryLabelColor)
        }
    }

    private func cells(_ cells: [Cell]) -> [Span] {
        var spans: [Span] = []
        for (index, cell) in cells.enumerated() {
            if index > 0 { spans.append(Span(text: "   ", bold: false, italic: false, code: false, strike: false, link: nil)) }
            spans.append(contentsOf: cell.spans)
        }
        return spans
    }

    private func marker(ordered: Bool, number: Int, checked: Bool?) -> String {
        if let checked { return checked ? "\u{2611}\t" : "\u{2610}\t" }
        return ordered ? "\(number).\t" : "\u{2022}\t"
    }

    private func paragraph(_ spans: [Span], _ context: Context, font: NSFont, after: CGFloat, bold: Bool = false, color: NSColor? = nil) {
        let paragraph = NSMutableParagraphStyle()
        paragraph.lineSpacing = Markdown.lineSpacing
        paragraph.headIndent = context.indent
        paragraph.firstLineHeadIndent = context.marker == nil ? context.indent : context.markerX
        paragraph.paragraphSpacing = after
        paragraph.lineBreakMode = .byWordWrapping
        if context.marker != nil {
            paragraph.tabStops = [NSTextTab(textAlignment: .left, location: context.indent, options: [:])]
            paragraph.defaultTabInterval = Markdown.listIndent
        }
        var base: [NSAttributedString.Key: Any] = [
            .font: font,
            .foregroundColor: color ?? textColor,
            .paragraphStyle: paragraph,
        ]
        if let x = context.quoteX { base[.quoteBar] = x }
        separator()
        let start = out.length
        if let marker = context.marker {
            out.append(NSAttributedString(string: marker, attributes: base))
        }
        for span in spans {
            var attributes = inline(span, font: font, bold: bold, paragraph: paragraph)
            if let color { attributes[.foregroundColor] = color }
            if let x = context.quoteX { attributes[.quoteBar] = x }
            out.append(NSAttributedString(string: span.text, attributes: attributes))
        }
        // A soft line break sits as close as a wrapped line does, and in a list item the line
        // after it starts under the item's text, not its marker.
        styleLines(from: start, like: paragraph) { line, first, last in
            if !first { line.firstLineHeadIndent = context.indent }
            if !last { line.paragraphSpacing = 0 }
        }
    }

    private func code(_ text: String, _ context: Context, after: CGFloat) {
        let paragraph = NSMutableParagraphStyle()
        paragraph.lineSpacing = 2
        paragraph.headIndent = context.indent + Markdown.codePaddingX
        paragraph.firstLineHeadIndent = paragraph.headIndent
        paragraph.tailIndent = -Markdown.codePaddingX
        paragraph.lineBreakMode = .byCharWrapping
        // The box is painted past the text on both sides; the spacing makes room for it. At the
        // very top or bottom of the run the text view's inset does that instead.
        if out.length == 0 {
            topInset = Markdown.codePaddingY
        } else {
            paragraph.paragraphSpacingBefore = Markdown.codePaddingY
        }
        if after == 0 {
            bottomInset = Markdown.codePaddingY
        } else {
            paragraph.paragraphSpacing = after + Markdown.codePaddingY
        }
        var attributes: [NSAttributedString.Key: Any] = [
            .font: Theme.Font.code,
            .foregroundColor: textColor.withAlphaComponent(0.92),
            .paragraphStyle: paragraph,
            .codeBlock: context.indent,
        ]
        if let x = context.quoteX { attributes[.quoteBar] = x }
        separator()
        let start = out.length
        if let marker = context.marker {
            out.append(NSAttributedString(string: marker, attributes: attributes))
        }
        out.append(NSAttributedString(string: text.isEmpty ? " " : text, attributes: attributes))
        // The room above the box goes to its first line and the room below to its last; the
        // lines between sit as close as wrapped ones.
        styleLines(from: start, like: paragraph) { line, first, last in
            if !first { line.paragraphSpacingBefore = 0 }
            if !last { line.paragraphSpacing = 0 }
        }
    }

    /// Gives each line of the block that starts at `start` its own copy of `style`, which
    /// `adjust` changes for the block's first and last lines. A line break inside a block (a
    /// soft break, a line of code) is a `\n`, and TextKit takes a `\n` to end a paragraph: with
    /// one style for the whole block, its spacing would open up between every line.
    private func styleLines(
        from start: Int, like style: NSParagraphStyle,
        _ adjust: (_ line: NSMutableParagraphStyle, _ first: Bool, _ last: Bool) -> Void
    ) {
        let text = out.mutableString
        let block = NSRange(location: start, length: text.length - start)
        guard text.rangeOfCharacter(from: .newlines, options: [], range: block).location != NSNotFound else { return }
        var location = start
        while location < text.length {
            var end = 0
            text.getParagraphStart(nil, end: &end, contentsEnd: nil, for: NSRange(location: location, length: 0))
            end = max(end, location + 1)
            let line = style.mutableCopy() as! NSMutableParagraphStyle
            adjust(line, location == start, end >= text.length)
            out.addAttribute(.paragraphStyle, value: line, range: NSRange(location: location, length: end - location))
            location = end
        }
    }

    /// Ends the previous paragraph. The newline takes the previous run's attributes so it
    /// never changes that line's height.
    private func separator() {
        guard out.length > 0 else { return }
        var attributes = out.attributes(at: out.length - 1, effectiveRange: nil)
        attributes.removeValue(forKey: .link)
        attributes.removeValue(forKey: .backgroundColor)
        attributes.removeValue(forKey: .strikethroughStyle)
        out.append(NSAttributedString(string: "\n", attributes: attributes))
    }

    private func inline(_ span: Span, font: NSFont, bold: Bool, paragraph: NSParagraphStyle) -> [NSAttributedString.Key: Any] {
        var attributes: [NSAttributedString.Key: Any] = [
            .font: inlineFont(font, span, bold: bold),
            .foregroundColor: textColor,
            .paragraphStyle: paragraph,
        ]
        if span.code { attributes[.backgroundColor] = Theme.codeBackground }
        if span.strike { attributes[.strikethroughStyle] = NSUnderlineStyle.single.rawValue }
        if let link = span.link, let url = URL(string: link) { attributes[.link] = url }
        return attributes
    }

    private func inlineFont(_ base: NSFont, _ span: Span, bold: Bool) -> NSFont {
        let bold = bold || span.bold
        var font: NSFont
        if span.code {
            font = bold ? .monospacedSystemFont(ofSize: Theme.Font.inlineCode.pointSize, weight: .semibold) : Theme.Font.inlineCode
        } else if bold {
            let weight: NSFont.Weight = base.fontDescriptor.symbolicTraits.contains(.bold) ? .bold : .semibold
            font = .systemFont(ofSize: base.pointSize, weight: weight)
        } else {
            font = base
        }
        if span.italic {
            font = NSFontManager.shared.convert(font, toHaveTrait: .italicFontMask)
        }
        return font
    }
}

// MARK: - Text view

/// A read-only, selectable text view on a TextKit 1 stack that paints code block boxes and
/// quote bars behind the text. Its frame is set by its parent; it never sizes itself.
final class MarkdownTextView: NSTextView {
    /// The run on show. Showing the same run again leaves the text, and the selection, alone.
    private weak var shown: NSAttributedString?
    private var linkBase: NSColor?

    init(textColor: NSColor) {
        let storage = NSTextStorage()
        let manager = NSLayoutManager()
        let container = NSTextContainer(size: NSSize(width: 0, height: CGFloat.greatestFiniteMagnitude))
        container.lineFragmentPadding = 0
        container.widthTracksTextView = true
        manager.addTextContainer(container)
        storage.addLayoutManager(manager)
        super.init(frame: .zero, textContainer: container)
        isEditable = false
        isSelectable = true
        isRichText = true
        drawsBackground = false
        textContainerInset = .zero
        isVerticallyResizable = false
        isHorizontallyResizable = false
        isAutomaticLinkDetectionEnabled = false
        colorLinks(for: textColor)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    /// Links take the text's color on the accent-colored user bubble, the link color elsewhere.
    /// A recycled cell can swap between the two.
    func colorLinks(for textColor: NSColor) {
        guard textColor != linkBase else { return }
        linkBase = textColor
        linkTextAttributes = [
            .foregroundColor: Markdown.linkColor(for: textColor),
            .underlineStyle: NSUnderlineStyle.single.rawValue,
            .cursor: NSCursor.pointingHand,
        ]
    }

    func show(_ text: NSAttributedString, topInset: CGFloat, bottomInset: CGFloat) {
        if text !== shown {
            textStorage?.setAttributedString(text)
            setSelectedRange(NSRange(location: 0, length: 0))
            shown = text
        }
        insets = (topInset, bottomInset)
    }

    /// Asymmetric vertical room for a code box at the top or bottom of the run.
    private var insets: (top: CGFloat, bottom: CGFloat) = (0, 0) {
        didSet { needsLayout = true }
    }

    /// Code boxes and quote bars, under the selection and the text. They are painted here
    /// rather than by the layout manager, which the text view clips to the text container: a
    /// box around the run's first line reaches above the container, into the top inset.
    override func drawBackground(in rect: NSRect) {
        super.drawBackground(in: rect)
        guard let storage = textStorage, storage.length > 0, let manager = layoutManager, let container = textContainer
        else { return }
        let origin = textContainerOrigin
        let all = NSRange(location: 0, length: storage.length)
        storage.enumerateAttribute(.codeBlock, in: all) { value, range, _ in
            guard let x = value as? CGFloat else { return }
            let glyphs = manager.glyphRange(forCharacterRange: range, actualCharacterRange: nil)
            var box = manager.boundingRect(forGlyphRange: glyphs, in: container)
            box.origin.x = x
            box.size.width = container.size.width - x
            box = box.insetBy(dx: 0, dy: -Markdown.codePaddingY).offsetBy(dx: origin.x, dy: origin.y)
            guard box.intersects(rect) else { return }
            Theme.codeBackground.setFill()
            NSBezierPath(roundedRect: box, xRadius: Markdown.codeCorner, yRadius: Markdown.codeCorner).fill()
        }
        storage.enumerateAttribute(.quoteBar, in: all) { value, range, _ in
            guard let x = value as? CGFloat else { return }
            let glyphs = manager.glyphRange(forCharacterRange: range, actualCharacterRange: nil)
            let lines = manager.boundingRect(forGlyphRange: glyphs, in: container)
            let bar = NSRect(x: origin.x + x, y: origin.y + lines.minY, width: Markdown.quoteBar, height: lines.height)
            guard bar.intersects(rect) else { return }
            NSColor.tertiaryLabelColor.setFill()
            NSBezierPath(roundedRect: bar, xRadius: Markdown.quoteBar / 2, yRadius: Markdown.quoteBar / 2).fill()
        }
    }

    override func layout() {
        super.layout()
        guard let container = textContainer else { return }
        container.size = NSSize(width: bounds.width, height: CGFloat.greatestFiniteMagnitude)
    }

    override var textContainerOrigin: NSPoint {
        NSPoint(x: 0, y: insets.top)
    }
}

// MARK: - Tables

/// A table laid out for a width: column widths measured from the cells and shrunk to fit when
/// they must, rows as tall as their tallest cell.
struct TableLayout {
    let content: TableContent
    let columns: [CGFloat]
    let rows: [CGFloat]

    var width: CGFloat { columns.reduce(0, +) }
    var height: CGFloat { rows.reduce(0, +) }

    @MainActor
    static func natural(_ content: TableContent) -> [CGFloat] {
        var natural = [CGFloat](repeating: 0, count: content.columns)
        for row in content.cells {
            for (c, cell) in row.enumerated() {
                natural[c] = max(natural[c], TextMeasure.textSize(of: cell, width: 100_000).width + Markdown.cellPaddingX * 2)
            }
        }
        return natural
    }

    @MainActor
    static func make(_ content: TableContent, maxWidth: CGFloat, natural known: [CGFloat]? = nil) -> TableLayout {
        let natural = known ?? self.natural(content)
        var widths = natural
        let total = natural.reduce(0, +)
        if total > maxWidth {
            // Take the excess from the wide columns first; no column drops below the floor.
            let flexible = natural.map { max(0, $0 - Markdown.columnFloor) }
            let give = flexible.reduce(0, +)
            let excess = min(total - maxWidth, give)
            if give > 0 {
                for c in widths.indices { widths[c] = natural[c] - flexible[c] / give * excess }
            }
        }
        widths = widths.map { ceil($0) }
        let heights = content.cells.map { row -> CGFloat in
            var height: CGFloat = 0
            for (c, cell) in row.enumerated() {
                let inner = max(1, widths[c] - Markdown.cellPaddingX * 2)
                height = max(height, TextMeasure.textSize(of: cell, width: inner).height + Markdown.cellPaddingY * 2)
            }
            return ceil(height)
        }
        return TableLayout(content: content, columns: widths, rows: heights)
    }
}

/// A table as a grid: a filled header row, hairline rules, rounded corners, selectable cells.
/// Wider than the bubble, it scrolls sideways.
final class MarkdownTableView: NSScrollView {
    private let grid = TableGridView()

    init() {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        drawsBackground = false
        hasVerticalScroller = false
        hasHorizontalScroller = false
        verticalScrollElasticity = .none
        automaticallyAdjustsContentInsets = false
        documentView = grid
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func show(_ layout: TableLayout, textColor: NSColor) {
        grid.show(layout, textColor: textColor)
    }
}

/// The grid's rules and header fill, with the cell text views on top.
final class TableGridView: NSView {
    private var cells: [MarkdownTextView] = []
    private var columns: [CGFloat] = []
    private var rows: [CGFloat] = []

    init() {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { true }

    func show(_ layout: TableLayout, textColor: NSColor) {
        let needed = layout.content.cells.reduce(0) { $0 + $1.count }
        while cells.count < needed {
            let cell = MarkdownTextView(textColor: textColor)
            addSubview(cell.framePositioned())
            cells.append(cell)
        }
        while cells.count > needed {
            cells.removeLast().removeFromSuperview()
        }
        columns = layout.columns
        rows = layout.rows
        var index = 0
        var y: CGFloat = 0
        for (r, row) in layout.content.cells.enumerated() {
            var x: CGFloat = 0
            for (c, text) in row.enumerated() {
                let cell = cells[index]
                index += 1
                cell.show(text, topInset: Markdown.cellPaddingY, bottomInset: Markdown.cellPaddingY)
                cell.frame = NSRect(x: x + Markdown.cellPaddingX, y: y, width: layout.columns[c] - Markdown.cellPaddingX * 2, height: layout.rows[r])
                x += layout.columns[c]
            }
            y += layout.rows[r]
        }
        frame = NSRect(x: 0, y: 0, width: layout.width, height: layout.height)
        needsDisplay = true
    }

    override func draw(_ dirtyRect: NSRect) {
        guard rows.count > 1 else { return }
        let hairline = 1 / max(1, window?.backingScaleFactor ?? 2)
        var y: CGFloat = 0
        for (index, height) in rows.dropLast().enumerated() {
            y += height
            let rule = NSRect(x: 0, y: y - hairline / 2, width: bounds.width, height: hairline)
            (index == 0 ? Markdown.tableHeaderRule : Markdown.tableRule).setFill()
            rule.fill()
        }
    }
}

// MARK: - Measurement

enum TextMeasure {
    /// `NSTextField` draws its text this far in from each side of its frame.
    static let labelInset: CGFloat = 2

    /// Frame size an `NSTextField` needs to show every line of `attributed` within `width`.
    /// `boundingRect` leaves out the field's inset and line metrics, and the field skips any
    /// line that does not fit its frame, so a `boundingRect` height can hide the last line.
    @MainActor
    static func labelSize(of attributed: NSAttributedString, width: CGFloat = 10_000) -> NSSize {
        guard width > 1, attributed.length > 0 else { return .zero }
        sizer.attributedStringValue = attributed
        let bounds = NSRect(x: 0, y: 0, width: width, height: .greatestFiniteMagnitude)
        let fitted = sizer.cell?.cellSize(forBounds: bounds) ?? .zero
        return NSSize(width: ceil(min(fitted.width, width)), height: ceil(fitted.height))
    }

    @MainActor private static let sizer: NSTextField = {
        let field = NSTextField(labelWithString: "")
        field.maximumNumberOfLines = 0
        field.lineBreakMode = .byWordWrapping
        return field
    }()

    static func width(of attributed: NSAttributedString) -> CGFloat {
        guard attributed.length > 0 else { return 0 }
        let rect = attributed.boundingRect(
            with: NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude),
            options: [.usesLineFragmentOrigin, .usesFontLeading]
        )
        return ceil(rect.width)
    }

    /// The size a `MarkdownTextView` uses for `attributed` within `width`: the same TextKit
    /// stack the view lays out with, so the two never disagree by a line. The width is the room
    /// the lines need to break as they do here: each line's text and its paragraph's indents,
    /// so a code line keeps the padding of its box and the width holds it on one line.
    @MainActor
    static func textSize(of attributed: NSAttributedString, width: CGFloat) -> NSSize {
        guard width > 1, attributed.length > 0 else { return .zero }
        let (storage, manager, container) = textSizer
        storage.setAttributedString(attributed)
        container.size = NSSize(width: width, height: CGFloat.greatestFiniteMagnitude)
        manager.ensureLayout(for: container)
        let used = manager.usedRect(for: container)
        let string = storage.mutableString
        var needed: CGFloat = 0
        manager.enumerateLineFragments(forGlyphRange: manager.glyphRange(for: container)) { _, line, _, glyphs, _ in
            let index = manager.characterIndexForGlyph(at: glyphs.location)
            let style = storage.attribute(.paragraphStyle, at: index, effectiveRange: nil) as? NSParagraphStyle ?? .default
            let startsParagraph = string.paragraphRange(for: NSRange(location: index, length: 0)).location == index
            let leading = startsParagraph ? style.firstLineHeadIndent : style.headIndent
            // A negative tail indent is measured from the trailing edge.
            let trailing = max(0, -style.tailIndent)
            needed = max(needed, leading + line.width + trailing)
        }
        return NSSize(width: ceil(min(needed, width)), height: ceil(used.height))
    }

    @MainActor private static let textSizer: (storage: NSTextStorage, manager: NSLayoutManager, container: NSTextContainer) = {
        let storage = NSTextStorage()
        let manager = NSLayoutManager()
        let container = NSTextContainer(size: NSSize(width: 0, height: CGFloat.greatestFiniteMagnitude))
        container.lineFragmentPadding = 0
        manager.addTextContainer(container)
        storage.addLayoutManager(manager)
        return (storage, manager, container)
    }()

    /// Width of the last wrapped line, used to park a timestamp on that line.
    @MainActor
    static func lastLineWidth(of attributed: NSAttributedString, width: CGFloat) -> CGFloat {
        guard width > 1, attributed.length > 0 else { return 0 }
        textSizer.storage.setAttributedString(attributed)
        textSizer.container.size = NSSize(width: width, height: CGFloat.greatestFiniteMagnitude)
        textSizer.manager.ensureLayout(for: textSizer.container)
        let glyphs = textSizer.manager.numberOfGlyphs
        guard glyphs > 0 else { return 0 }
        let rect = textSizer.manager.lineFragmentUsedRect(forGlyphAt: glyphs - 1, effectiveRange: nil)
        return ceil(rect.width)
    }
}

/// A message body laid out at one width: each segment's height and each table's grid, in order.
/// The measurement and the views that show it use the same numbers, so the text is laid out
/// once to measure a row rather than again for every cell that shows it.
struct SegmentLayout {
    var width: CGFloat = 0
    var heights: [CGFloat] = []
    var tables: [TableLayout] = []
    /// The segments and the spacing between them, rounded up.
    var height: CGFloat = 0

    init() {}

    /// `textSizes` are the text runs already measured at `width`, and `naturals` the tables'
    /// natural column widths, which no width changes.
    @MainActor
    init(_ segments: [MessageSegment], width: CGFloat, textSizes: [NSSize]? = nil, naturals: [[CGFloat]]? = nil) {
        self.width = width
        guard !segments.isEmpty else { return }
        var total: CGFloat = 0
        var texts = 0
        for (index, segment) in segments.enumerated() {
            let height: CGFloat
            switch segment {
            case let .text(attributed, top, bottom):
                let size = textSizes?[texts] ?? TextMeasure.textSize(of: attributed, width: width)
                texts += 1
                height = size.height + top + bottom
            case let .table(content):
                let table = TableLayout.make(content, maxWidth: width, natural: naturals?[tables.count])
                tables.append(table)
                height = table.height
            }
            heights.append(height)
            total += height
            if index < segments.count - 1 { total += Markdown.segmentSpacing }
        }
        self.height = ceil(total)
    }
}

struct RenderedMessage {
    let segments: [MessageSegment]

    init(_ raw: String, textColor: NSColor) {
        segments = Markdown.segments(raw, textColor: textColor)
    }

    var isEmpty: Bool { segments.isEmpty }

    /// The words as shown, without the Markdown: a table row by row, its cells between commas.
    var plainText: String {
        segments.map { segment in
            switch segment {
            case let .text(attributed, _, _):
                attributed.string
            case let .table(content):
                content.cells.map { row in row.map(\.string).joined(separator: ", ") }.joined(separator: "\n")
            }
        }
        .joined(separator: "\n")
        .trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// The body at the width it wants, from `minimum` up to `limit`: short text gets a bubble
    /// that hugs it, long text fills the limit. Text that fills it is measured once, at the limit.
    @MainActor
    func layout(fitting limit: CGFloat, minimum: CGFloat) -> SegmentLayout {
        var sizes: [NSSize] = []
        var naturals: [[CGFloat]] = []
        var widest: CGFloat = 0
        for segment in segments {
            switch segment {
            case let .text(attributed, _, _):
                let size = TextMeasure.textSize(of: attributed, width: limit)
                sizes.append(size)
                widest = max(widest, size.width)
            case let .table(content):
                let natural = TableLayout.natural(content)
                naturals.append(natural)
                widest = max(widest, natural.reduce(0, +))
            }
        }
        let width = max(minimum, min(widest, limit))
        return SegmentLayout(segments, width: width, textSizes: width == limit ? sizes : nil, naturals: naturals)
    }

    @MainActor
    func lastLineWidth(forWidth width: CGFloat) -> CGFloat {
        guard let last = segments.last else { return 0 }
        switch last {
        case let .text(attributed, _, _):
            return TextMeasure.lastLineWidth(of: attributed, width: width)
        case let .table(content):
            return min(width, TableLayout.make(content, maxWidth: width).width)
        }
    }
}
