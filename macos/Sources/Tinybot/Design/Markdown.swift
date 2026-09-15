import AppKit

enum MessageSegment {
    case text(NSAttributedString)
    case code(NSAttributedString, language: String?)
}

/// Enough Markdown for what a model actually emits mid-stream: fenced blocks,
/// inline code, bold, and dashed lists. Unclosed markers stay literal so a
/// half-streamed token never flickers into formatting.
enum Markdown {
    static let segmentSpacing: CGFloat = 8
    static let codePaddingX: CGFloat = 10
    static let codePaddingY: CGFloat = 8

    static func segments(_ raw: String, textColor: NSColor) -> [MessageSegment] {
        var segments: [MessageSegment] = []
        var textLines: [String] = []
        var codeLines: [String] = []
        var language: String?
        var inFence = false

        func flushText() {
            let joined = textLines.joined(separator: "\n").trimmingCharacters(in: .newlines)
            textLines.removeAll()
            guard !joined.isEmpty else { return }
            segments.append(.text(paragraphs(joined, color: textColor)))
        }

        func flushCode() {
            let joined = codeLines.joined(separator: "\n")
            codeLines.removeAll()
            guard !joined.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
            segments.append(.code(code(joined, color: textColor), language: language))
        }

        for line in raw.components(separatedBy: "\n") {
            if line.hasPrefix("```") {
                if inFence {
                    flushCode()
                    inFence = false
                    language = nil
                } else {
                    flushText()
                    inFence = true
                    let tag = String(line.dropFirst(3)).trimmingCharacters(in: .whitespaces)
                    language = tag.isEmpty ? nil : tag
                }
                continue
            }
            if inFence {
                codeLines.append(line)
            } else {
                textLines.append(line)
            }
        }

        if inFence {
            flushCode()
        } else {
            flushText()
        }

        return segments
    }

    // MARK: - Blocks

    private static func paragraphs(_ text: String, color: NSColor) -> NSAttributedString {
        let result = NSMutableAttributedString()
        let lines = text.components(separatedBy: "\n")

        for (index, line) in lines.enumerated() {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            let isBullet = trimmed.hasPrefix("- ") || trimmed.hasPrefix("* ")
            let content = isBullet ? String(trimmed.dropFirst(2)) : line

            if isBullet {
                result.append(
                    NSAttributedString(
                        string: "•\t",
                        attributes: [.font: Theme.Font.message, .foregroundColor: color]
                    ))
            }

            let body = inline(content, color: color)
            result.append(body)

            let style = NSMutableParagraphStyle()
            style.lineSpacing = 2.5
            style.paragraphSpacing = trimmed.isEmpty ? 0 : 6
            if isBullet {
                style.headIndent = 15
                style.tabStops = [NSTextTab(textAlignment: .left, location: 15)]
                style.paragraphSpacing = 2
            }

            let paragraphRange = NSRange(
                location: result.length - body.length - (isBullet ? 2 : 0),
                length: body.length + (isBullet ? 2 : 0)
            )
            result.addAttribute(.paragraphStyle, value: style, range: paragraphRange)

            if index < lines.count - 1 {
                result.append(NSAttributedString(string: "\n", attributes: [.paragraphStyle: style]))
            }
        }

        return result
    }

    private static func code(_ text: String, color: NSColor) -> NSAttributedString {
        let style = NSMutableParagraphStyle()
        style.lineSpacing = 2
        return NSAttributedString(
            string: text,
            attributes: [
                .font: Theme.Font.code,
                .foregroundColor: color.withAlphaComponent(0.92),
                .paragraphStyle: style,
            ]
        )
    }

    // MARK: - Inline

    private static func inline(_ text: String, color: NSColor) -> NSAttributedString {
        let result = NSMutableAttributedString()
        let characters = Array(text)
        var index = 0
        var plain = ""

        func flushPlain() {
            guard !plain.isEmpty else { return }
            result.append(
                NSAttributedString(
                    string: plain,
                    attributes: [.font: Theme.Font.message, .foregroundColor: color]
                ))
            plain = ""
        }

        while index < characters.count {
            let character = characters[index]

            if character == "*", index + 1 < characters.count, characters[index + 1] == "*",
                let close = find("**", in: characters, from: index + 2)
            {
                flushPlain()
                let inner = String(characters[(index + 2)..<close])
                result.append(
                    NSAttributedString(
                        string: inner,
                        attributes: [.font: Theme.Font.messageBold, .foregroundColor: color]
                    ))
                index = close + 2
                continue
            }

            if character == "`", let close = find("`", in: characters, from: index + 1) {
                flushPlain()
                let inner = String(characters[(index + 1)..<close])
                result.append(
                    NSAttributedString(
                        string: inner,
                        attributes: [
                            .font: Theme.Font.inlineCode,
                            .foregroundColor: color,
                            .backgroundColor: Theme.codeBackground,
                        ]
                    ))
                index = close + 1
                continue
            }

            plain.append(character)
            index += 1
        }

        flushPlain()
        return result
    }

    private static func find(_ marker: String, in characters: [Character], from start: Int) -> Int? {
        let markers = Array(marker)
        guard !markers.isEmpty, start < characters.count else { return nil }
        var index = start
        while index + markers.count <= characters.count {
            if Array(characters[index..<(index + markers.count)]) == markers { return index }
            index += 1
        }
        return nil
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

    /// Width of the last wrapped line, used to park a timestamp on that line.
    static func lastLineWidth(of attributed: NSAttributedString, width: CGFloat) -> CGFloat {
        guard width > 1, attributed.length > 0 else { return 0 }
        let storage = NSTextStorage(attributedString: attributed)
        let manager = NSLayoutManager()
        let container = NSTextContainer(size: NSSize(width: width, height: CGFloat.greatestFiniteMagnitude))
        container.lineFragmentPadding = 0
        manager.addTextContainer(container)
        storage.addLayoutManager(manager)
        manager.ensureLayout(for: container)
        let glyphs = manager.numberOfGlyphs
        guard glyphs > 0 else { return 0 }
        let rect = manager.lineFragmentUsedRect(forGlyphAt: glyphs - 1, effectiveRange: nil)
        return ceil(rect.width)
    }
}

struct RenderedMessage {
    let segments: [MessageSegment]

    init(_ raw: String, textColor: NSColor) {
        segments = Markdown.segments(raw, textColor: textColor)
    }

    var isEmpty: Bool { segments.isEmpty }

    @MainActor
    func height(forWidth width: CGFloat) -> CGFloat {
        guard !segments.isEmpty else { return 0 }
        var total: CGFloat = 0
        for (index, segment) in segments.enumerated() {
            switch segment {
            case let .text(attributed):
                total += TextMeasure.labelSize(of: attributed, width: width).height
            case let .code(attributed, _):
                let inner = width - Markdown.codePaddingX * 2
                let height = TextMeasure.labelSize(of: attributed, width: inner).height
                total += height + Markdown.codePaddingY * 2
            }
            if index < segments.count - 1 { total += Markdown.segmentSpacing }
        }
        return ceil(total)
    }

    /// Natural width so short replies get a bubble that hugs the text.
    @MainActor
    func preferredWidth(max limit: CGFloat) -> CGFloat {
        var widest: CGFloat = 0
        for segment in segments {
            switch segment {
            case let .text(attributed):
                widest = max(widest, TextMeasure.labelSize(of: attributed).width)
            case let .code(attributed, _):
                let width = TextMeasure.labelSize(of: attributed).width + Markdown.codePaddingX * 2
                widest = max(widest, width)
            }
        }
        return min(widest, limit)
    }

    func lastLineWidth(forWidth width: CGFloat) -> CGFloat {
        guard let last = segments.last else { return 0 }
        switch last {
        case let .text(attributed):
            return TextMeasure.lastLineWidth(of: attributed, width: width)
        case .code:
            return width
        }
    }
}
