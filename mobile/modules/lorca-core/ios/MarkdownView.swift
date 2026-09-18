import ExpoModulesCore
import UIKit

/// What the JS side sets: the Markdown, the width the bubble allows, and the message palette.
struct MarkdownStyle {
  var markdown = ""
  var maxWidth: CGFloat = 0
  var fontSize: CGFloat = 16.5
  var codeFontSize: CGFloat = 14
  var color: UIColor = .label
  var linkColor: UIColor = .link
  var codeBackground: UIColor = UIColor.black.withAlphaComponent(0.06)
  var quoteColor: UIColor = .tertiaryLabel
  var border: UIColor = .separator
  var tint: UIColor = .systemBlue
}

enum MarkdownLayout {
  static let gap: CGFloat = 8
  static let itemGap: CGFloat = 3
  static let rowGap: CGFloat = 2
  static let codePadding: CGFloat = 8
  static let codeInset: CGFloat = 10
  static let codeCorner: CGFloat = 8
  static let listIndent: CGFloat = 22
  static let quoteIndent: CGFloat = 12
  static let quoteBar: CGFloat = 2
  static let cellPaddingX: CGFloat = 10
  static let cellPaddingY: CGFloat = 7
  static let columnFloor: CGFloat = 72
}

extension NSAttributedString.Key {
  /// Marks a fenced code block; the value is the x where its box starts.
  static let codeBlock = NSAttributedString.Key("lorca.codeBlock")
  /// Marks quoted text; the value is the x of its bar.
  static let quoteBar = NSAttributedString.Key("lorca.quoteBar")
}

/// A TextKit 1 layout manager that paints code block boxes and quote bars behind the text.
final class MarkdownLayoutManager: NSLayoutManager {
  var codeBackground: UIColor = .clear
  var quoteColor: UIColor = .clear

  override func drawBackground(forGlyphRange glyphsToShow: NSRange, at origin: CGPoint) {
    super.drawBackground(forGlyphRange: glyphsToShow, at: origin)
    guard let storage = textStorage, let container = textContainers.first else { return }
    let chars = characterRange(forGlyphRange: glyphsToShow, actualGlyphRange: nil)
    storage.enumerateAttribute(.codeBlock, in: chars) { value, range, _ in
      guard let x = value as? CGFloat else { return }
      let glyphs = glyphRange(forCharacterRange: range, actualCharacterRange: nil)
      var rect = boundingRect(forGlyphRange: glyphs, in: container)
      rect.origin.x = x
      rect.size.width = container.size.width - x
      rect = rect.insetBy(dx: 0, dy: -MarkdownLayout.codePadding).offsetBy(dx: origin.x, dy: origin.y)
      codeBackground.setFill()
      UIBezierPath(roundedRect: rect, cornerRadius: MarkdownLayout.codeCorner).fill()
    }
    storage.enumerateAttribute(.quoteBar, in: chars) { value, range, _ in
      guard let x = value as? CGFloat else { return }
      let glyphs = glyphRange(forCharacterRange: range, actualCharacterRange: nil)
      let rect = boundingRect(forGlyphRange: glyphs, in: container)
      let bar = CGRect(x: origin.x + x, y: origin.y + rect.minY, width: MarkdownLayout.quoteBar, height: rect.height)
      quoteColor.setFill()
      UIBezierPath(roundedRect: bar, cornerRadius: MarkdownLayout.quoteBar / 2).fill()
    }
  }
}

/// A run of text plus the vertical room a code block needs when it opens or closes the run.
struct RenderedText {
  let text: NSAttributedString
  let topInset: CGFloat
  let bottomInset: CGFloat
}

struct TableSpec {
  let alignments: [Align]
  let header: [Cell]
  let rows: [TableRow]
}

/// A message is text runs and, between them, the tables that get a grid of their own.
enum Segment {
  case text(RenderedText)
  case table(TableSpec)
}

/// Turns the core's document into attributed text: fonts and paragraph styles per block,
/// custom attributes where the layout manager paints. Top-level tables become segments; a
/// table inside a list or quote is written as lines of cells.
final class MarkdownRenderer {
  private let style: MarkdownStyle
  private var out = NSMutableAttributedString()
  private var topInset: CGFloat = 0
  private var bottomInset: CGFloat = 0
  private var segments: [Segment] = []

  /// Where a block sits: its indents and the bar or marker it inherits.
  private struct Context {
    var indent: CGFloat = 0
    var quoteX: CGFloat? = nil
    var marker: String? = nil
    var markerX: CGFloat = 0
  }

  init(style: MarkdownStyle) {
    self.style = style
  }

  func render(_ document: Document) -> [Segment] {
    let blocks = document.blocks
    for (index, block) in blocks.enumerated() {
      if case .table(let alignments, let header, let rows) = block {
        flush()
        segments.append(.table(TableSpec(alignments: alignments, header: header, rows: rows)))
        continue
      }
      let next = index + 1 < blocks.count ? blocks[index + 1] : nil
      var breaks = next == nil
      if let next, case .table = next { breaks = true }
      self.block(block, Context(), after: breaks ? 0 : MarkdownLayout.gap)
    }
    flush()
    return segments
  }

  private func flush() {
    guard out.length > 0 else { return }
    segments.append(.text(RenderedText(text: out, topInset: topInset, bottomInset: bottomInset)))
    out = NSMutableAttributedString()
    topInset = 0
    bottomInset = 0
  }

  /// One table cell, for the grid.
  func cell(_ spans: [Span], header: Bool, align: Align) -> NSAttributedString {
    let paragraph = NSMutableParagraphStyle()
    paragraph.minimumLineHeight = style.fontSize * 1.3
    paragraph.maximumLineHeight = style.fontSize * 1.3
    paragraph.lineBreakMode = .byWordWrapping
    switch align {
    case .center: paragraph.alignment = .center
    case .right: paragraph.alignment = .right
    case .left, .auto: paragraph.alignment = .natural
    }
    let font: UIFont = header ? .systemFont(ofSize: style.fontSize - 1, weight: .semibold) : .systemFont(ofSize: style.fontSize - 1)
    let text = NSMutableAttributedString()
    for span in spans {
      var attributes: [NSAttributedString.Key: Any] = [
        .font: inlineFont(font, span, bold: header),
        .foregroundColor: style.color,
        .paragraphStyle: paragraph,
      ]
      if span.code { attributes[.backgroundColor] = style.codeBackground }
      if span.strike { attributes[.strikethroughStyle] = NSUnderlineStyle.single.rawValue }
      if let link = span.link, let url = URL(string: link) { attributes[.link] = url }
      text.append(NSAttributedString(string: span.text, attributes: attributes))
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
      self.block(block, context, after: last ? after : MarkdownLayout.gap)
    }
  }

  private func block(_ block: Block, _ context: Context, after: CGFloat) {
    switch block {
    case .paragraph(let spans):
      paragraph(spans, context, font: body(), lineHeight: style.fontSize * 1.35, after: after)
    case .heading(let level, let spans):
      let size = level <= 2 ? style.fontSize + 2 : style.fontSize
      paragraph(spans, context, font: .systemFont(ofSize: size, weight: .bold), lineHeight: (style.fontSize + 2) * 1.3, after: after, bold: true)
    case .code(_, let text):
      code(text, context, after: after)
    case .listing(let ordered, let start, let items):
      for (index, item) in items.enumerated() {
        var inner = context
        inner.marker = marker(ordered: ordered, number: Int(start) + index, checked: item.checked)
        inner.markerX = context.indent
        inner.indent = context.indent + MarkdownLayout.listIndent
        let last = index == items.count - 1
        if item.blocks.isEmpty {
          paragraph([], inner, font: body(), lineHeight: style.fontSize * 1.35, after: last ? after : MarkdownLayout.itemGap)
        } else {
          blocks(item.blocks, inner, after: last ? after : MarkdownLayout.itemGap)
        }
      }
    case .quote(let blocks):
      var inner = context
      inner.quoteX = context.indent
      inner.indent = context.indent + MarkdownLayout.quoteIndent
      self.blocks(blocks, inner, after: after)
    case .table(_, let header, let rows):
      let last = rows.isEmpty
      paragraph(cells(header), context, font: body(), lineHeight: style.fontSize * 1.35, after: last ? after : MarkdownLayout.rowGap, bold: true)
      for (index, row) in rows.enumerated() {
        let last = index == rows.count - 1
        paragraph(cells(row.cells), context, font: body(), lineHeight: style.fontSize * 1.35, after: last ? after : MarkdownLayout.rowGap)
      }
    case .rule:
      let rule = Span(text: String(repeating: "\u{2500}", count: 24), bold: false, italic: false, code: false, strike: false, link: nil)
      paragraph([rule], context, font: body(), lineHeight: style.fontSize * 1.35, after: after, color: style.quoteColor)
    }
  }

  /// One row of a nested table: its cells on a line, three spaces apart.
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

  private func paragraph(_ spans: [Span], _ context: Context, font: UIFont, lineHeight: CGFloat, after: CGFloat, bold: Bool = false, color: UIColor? = nil) {
    let paragraph = NSMutableParagraphStyle()
    paragraph.minimumLineHeight = lineHeight
    paragraph.maximumLineHeight = lineHeight
    paragraph.headIndent = context.indent
    paragraph.firstLineHeadIndent = context.marker == nil ? context.indent : context.markerX
    paragraph.paragraphSpacing = after
    paragraph.lineBreakMode = .byWordWrapping
    if context.marker != nil {
      paragraph.tabStops = [NSTextTab(textAlignment: .left, location: context.indent, options: [:])]
      paragraph.defaultTabInterval = MarkdownLayout.listIndent
    }
    var base: [NSAttributedString.Key: Any] = [
      .font: font,
      .foregroundColor: color ?? style.color,
      .paragraphStyle: paragraph,
      .baselineOffset: max(0, (lineHeight - font.lineHeight) / 2),
    ]
    if let x = context.quoteX { base[.quoteBar] = x }
    separator()
    if let marker = context.marker {
      out.append(NSAttributedString(string: marker, attributes: base))
    }
    for span in spans {
      var attributes = base
      attributes[.font] = inlineFont(font, span, bold: bold)
      if span.code { attributes[.backgroundColor] = style.codeBackground }
      if span.strike { attributes[.strikethroughStyle] = NSUnderlineStyle.single.rawValue }
      if let link = span.link, let url = URL(string: link) { attributes[.link] = url }
      out.append(NSAttributedString(string: span.text, attributes: attributes))
    }
  }

  private func code(_ text: String, _ context: Context, after: CGFloat) {
    let font = mono(style.codeFontSize)
    let lineHeight = style.codeFontSize * 1.4
    let paragraph = NSMutableParagraphStyle()
    paragraph.minimumLineHeight = lineHeight
    paragraph.maximumLineHeight = lineHeight
    paragraph.headIndent = context.indent + MarkdownLayout.codeInset
    paragraph.firstLineHeadIndent = paragraph.headIndent
    paragraph.tailIndent = -MarkdownLayout.codeInset
    paragraph.lineBreakMode = .byCharWrapping
    // The box is painted past the text on both sides; the spacing makes room for it. At the
    // very top or bottom of the run the text view's inset does that instead.
    if out.length == 0 {
      topInset = MarkdownLayout.codePadding
    } else {
      paragraph.paragraphSpacingBefore = MarkdownLayout.codePadding
    }
    if after == 0 {
      bottomInset = MarkdownLayout.codePadding
    } else {
      paragraph.paragraphSpacing = after + MarkdownLayout.codePadding
    }
    var attributes: [NSAttributedString.Key: Any] = [
      .font: font,
      .foregroundColor: style.color,
      .paragraphStyle: paragraph,
      .baselineOffset: max(0, (lineHeight - font.lineHeight) / 2),
      .codeBlock: context.indent,
    ]
    if let x = context.quoteX { attributes[.quoteBar] = x }
    separator()
    if let marker = context.marker {
      out.append(NSAttributedString(string: marker, attributes: attributes))
    }
    out.append(NSAttributedString(string: text.isEmpty ? " " : text, attributes: attributes))
  }

  /// Ends the previous paragraph. The newline takes the previous run's attributes so it never
  /// changes that line's height.
  private func separator() {
    guard out.length > 0 else { return }
    var attributes = out.attributes(at: out.length - 1, effectiveRange: nil)
    attributes.removeValue(forKey: .link)
    attributes.removeValue(forKey: .backgroundColor)
    attributes.removeValue(forKey: .strikethroughStyle)
    out.append(NSAttributedString(string: "\n", attributes: attributes))
  }

  private func body() -> UIFont {
    .systemFont(ofSize: style.fontSize)
  }

  private func mono(_ size: CGFloat) -> UIFont {
    UIFont(name: "Menlo", size: size) ?? .monospacedSystemFont(ofSize: size, weight: .regular)
  }

  private func inlineFont(_ base: UIFont, _ span: Span, bold: Bool) -> UIFont {
    let bold = bold || span.bold
    var font: UIFont
    if span.code {
      font = mono(base.pointSize - 1.5)
    } else if bold {
      font = .systemFont(ofSize: base.pointSize, weight: base.fontDescriptor.symbolicTraits.contains(.traitBold) ? .bold : .semibold)
    } else {
      font = base
    }
    var traits = font.fontDescriptor.symbolicTraits
    if span.italic { traits.insert(.traitItalic) }
    if bold, span.code { traits.insert(.traitBold) }
    if traits != font.fontDescriptor.symbolicTraits, let descriptor = font.fontDescriptor.withSymbolicTraits(traits) {
      font = UIFont(descriptor: descriptor, size: font.pointSize)
    }
    return font
  }
}

/// Text and table sizes from standalone TextKit stacks: no views, so the JS thread can ask for
/// a message's size before the view exists and the view lays out to the same numbers.
enum MarkdownMeasure {
  static func text(_ text: NSAttributedString, insets: UIEdgeInsets, maxWidth: CGFloat) -> CGSize {
    let storage = NSTextStorage(attributedString: text)
    let manager = NSLayoutManager()
    let container = NSTextContainer(size: CGSize(width: max(1, maxWidth - insets.left - insets.right), height: CGFloat.greatestFiniteMagnitude))
    container.lineFragmentPadding = 0
    manager.addTextContainer(container)
    storage.addLayoutManager(manager)
    manager.ensureLayout(for: container)
    let used = manager.usedRect(for: container)
    return CGSize(width: min(ceil(used.width) + insets.left + insets.right, maxWidth), height: ceil(used.height) + insets.top + insets.bottom)
  }

  /// The size a whole message takes within `maxWidth`: its segments stacked with the block gap.
  static func message(_ style: MarkdownStyle) -> CGSize {
    let renderer = MarkdownRenderer(style: style)
    let width = max(style.maxWidth, 1)
    var total = CGSize.zero
    for (index, segment) in renderer.render(parseMarkdown(text: style.markdown)).enumerated() {
      let size: CGSize
      switch segment {
      case .text(let rendered):
        size = text(rendered.text, insets: UIEdgeInsets(top: rendered.topInset, left: 0, bottom: rendered.bottomInset, right: 0), maxWidth: width)
      case .table(let spec):
        size = TableLayout(cells: TableLayout.cells(spec, renderer: renderer), maxWidth: width).size
      }
      if index > 0 { total.height += MarkdownLayout.gap }
      total.height += size.height
      total.width = max(total.width, size.width)
    }
    return total
  }
}

/// A table's columns measured from their cells and shrunk to fit when they must, rows as tall
/// as their tallest cell.
struct TableLayout {
  static let cellInsets = UIEdgeInsets(top: MarkdownLayout.cellPaddingY, left: MarkdownLayout.cellPaddingX, bottom: MarkdownLayout.cellPaddingY, right: MarkdownLayout.cellPaddingX)

  var widths: [CGFloat] = []
  var heights: [CGFloat] = []
  /// The size the table takes in the message; the grid itself may be wider.
  var size = CGSize.zero

  /// The header and rows as attributed cells, every row padded to the widest one.
  static func cells(_ spec: TableSpec, renderer: MarkdownRenderer) -> [[NSAttributedString]] {
    let rows = [spec.header] + spec.rows.map(\.cells)
    let columns = rows.map(\.count).max() ?? 0
    return rows.enumerated().map { r, row in
      (0..<columns).map { c in
        let align = c < spec.alignments.count ? spec.alignments[c] : .auto
        return renderer.cell(c < row.count ? row[c].spans : [], header: r == 0, align: align)
      }
    }
  }

  init(cells: [[NSAttributedString]], maxWidth: CGFloat) {
    let columns = cells.first?.count ?? 0
    guard columns > 0 else { return }
    let unbounded: CGFloat = 100_000
    var natural = [CGFloat](repeating: 0, count: columns)
    for row in cells {
      for (c, cell) in row.enumerated() {
        natural[c] = max(natural[c], MarkdownMeasure.text(cell, insets: Self.cellInsets, maxWidth: unbounded).width)
      }
    }
    widths = natural
    let total = natural.reduce(0, +)
    if total > maxWidth {
      // Take the excess from the wide columns first; no column drops below the floor.
      let floor = MarkdownLayout.columnFloor
      let flexible = natural.map { max(0, $0 - floor) }
      let give = flexible.reduce(0, +)
      let excess = min(total - maxWidth, give)
      if give > 0 {
        for c in 0..<columns { widths[c] = natural[c] - flexible[c] / give * excess }
      }
    }
    widths = widths.map { ceil($0) }
    for row in cells {
      var height: CGFloat = 0
      for (c, cell) in row.enumerated() {
        height = max(height, MarkdownMeasure.text(cell, insets: Self.cellInsets, maxWidth: widths[c]).height)
      }
      heights.append(ceil(height))
    }
    size = CGSize(width: min(widths.reduce(0, +), maxWidth), height: heights.reduce(0, +))
  }
}

/// A read-only, selectable UITextView on a TextKit 1 stack, so the layout manager can paint
/// code boxes and quote bars.
final class SelectableTextView: UITextView {
  private let manager = MarkdownLayoutManager()
  private let storage = NSTextStorage()
  private let container = NSTextContainer(size: CGSize(width: 0, height: CGFloat.greatestFiniteMagnitude))

  init(style: MarkdownStyle) {
    container.lineFragmentPadding = 0
    container.widthTracksTextView = true
    manager.addTextContainer(container)
    storage.addLayoutManager(manager)
    super.init(frame: .zero, textContainer: container)
    manager.codeBackground = style.codeBackground
    manager.quoteColor = style.quoteColor
    isEditable = false
    isSelectable = true
    isScrollEnabled = false
    backgroundColor = .clear
    textContainerInset = .zero
    dataDetectorTypes = [.link]
    adjustsFontForContentSizeCategory = false
    tintColor = style.tint
    linkTextAttributes = [.foregroundColor: style.linkColor, .underlineStyle: NSUnderlineStyle.single.rawValue]
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) {
    fatalError("init(coder:) has not been implemented")
  }
}

/// A table as a grid: a filled header row, hairline rules, rounded corners. Wider than the
/// bubble, it scrolls sideways.
final class MarkdownTableView: UIScrollView {
  private let grid: TableGridView
  private let texts: [[NSAttributedString]]
  private let cells: [[SelectableTextView]]

  init(spec: TableSpec, style: MarkdownStyle, renderer: MarkdownRenderer) {
    texts = TableLayout.cells(spec, renderer: renderer)
    cells = texts.map { row in
      row.map { text in
        let view = SelectableTextView(style: style)
        view.textContainerInset = TableLayout.cellInsets
        view.attributedText = text
        return view
      }
    }
    grid = TableGridView(style: style)
    super.init(frame: .zero)
    showsHorizontalScrollIndicator = false
    showsVerticalScrollIndicator = false
    alwaysBounceHorizontal = false
    backgroundColor = .clear
    addSubview(grid)
    for row in cells { for cell in row { grid.addSubview(cell) } }
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) {
    fatalError("init(coder:) has not been implemented")
  }

  /// Lays the grid out for the width and returns the size the table takes in the message.
  func measure(_ maxWidth: CGFloat) -> CGSize {
    let layout = TableLayout(cells: texts, maxWidth: maxWidth)
    var y: CGFloat = 0
    for (r, row) in cells.enumerated() {
      var x: CGFloat = 0
      for (c, cell) in row.enumerated() {
        cell.frame = CGRect(x: x, y: y, width: layout.widths[c], height: layout.heights[r])
        x += layout.widths[c]
      }
      y += layout.heights[r]
    }
    grid.columns = layout.widths
    grid.rows = layout.heights
    grid.frame = CGRect(x: 0, y: 0, width: layout.widths.reduce(0, +), height: y)
    grid.setNeedsDisplay()
    contentSize = grid.frame.size
    return layout.size
  }
}

/// The grid's rules and header fill, drawn behind the cells.
final class TableGridView: UIView {
  var columns: [CGFloat] = []
  var rows: [CGFloat] = []
  private let style: MarkdownStyle

  init(style: MarkdownStyle) {
    self.style = style
    super.init(frame: .zero)
    backgroundColor = .clear
    isOpaque = false
    contentMode = .redraw
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) {
    fatalError("init(coder:) has not been implemented")
  }

  override func draw(_ rect: CGRect) {
    guard rows.count > 1 else { return }
    let hairline = 1 / max(1, traitCollection.displayScale)
    var y: CGFloat = 0
    for (index, height) in rows.dropLast().enumerated() {
      y += height
      (index == 0 ? style.quoteColor : style.border).setFill()
      UIRectFill(CGRect(x: 0, y: y - hairline / 2, width: bounds.width, height: hairline))
    }
  }
}

/// A message body as native views: the core parses the Markdown, this renders the text runs
/// into selectable text views and the tables into grids, stacked with the block gap. The JS
/// side sizes it from `MarkdownMeasure.message`, in the same commit as the text, so a one-line
/// message keeps a narrow bubble and a recycled row never shows its last message's width.
final class MarkdownView: ExpoView {
  var style = MarkdownStyle()
  private var segments: [(view: UIView, size: CGSize)] = []

  required init(appContext: AppContext? = nil) {
    super.init(appContext: appContext)
  }

  override func layoutSubviews() {
    super.layoutSubviews()
    var y: CGFloat = 0
    for (index, segment) in segments.enumerated() {
      if index > 0 { y += MarkdownLayout.gap }
      let width = segment.view is SelectableTextView ? bounds.width : min(bounds.width, segment.size.width)
      segment.view.frame = CGRect(x: 0, y: y, width: width, height: segment.size.height)
      y += segment.size.height
    }
  }

  func render() {
    for segment in segments { segment.view.removeFromSuperview() }
    segments = []
    let renderer = MarkdownRenderer(style: style)
    let width = max(style.maxWidth, 1)
    for segment in renderer.render(parseMarkdown(text: style.markdown)) {
      let view: UIView
      let size: CGSize
      switch segment {
      case .text(let rendered):
        let text = SelectableTextView(style: style)
        text.textContainerInset = UIEdgeInsets(top: rendered.topInset, left: 0, bottom: rendered.bottomInset, right: 0)
        text.attributedText = rendered.text
        size = MarkdownMeasure.text(rendered.text, insets: text.textContainerInset, maxWidth: width)
        view = text
      case .table(let spec):
        let table = MarkdownTableView(spec: spec, style: style, renderer: renderer)
        size = table.measure(width)
        view = table
      }
      addSubview(view)
      segments.append((view, size))
    }
    setNeedsLayout()
  }
}
