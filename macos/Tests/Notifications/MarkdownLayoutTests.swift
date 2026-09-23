import AppKit
import XCTest
@testable import Lorca

@MainActor
final class MarkdownLayoutTests: XCTestCase {
    private func run(_ raw: String) throws -> NSAttributedString {
        guard case let .text(text, _, _)? = Markdown.segments(raw, textColor: .labelColor).first else {
            throw XCTSkip("expected a text run first")
        }
        return text
    }

    private func style(in text: NSAttributedString, at needle: String) -> NSParagraphStyle? {
        let range = (text.string as NSString).range(of: needle)
        guard range.location != NSNotFound else { return nil }
        return text.attribute(.paragraphStyle, at: range.location, effectiveRange: nil) as? NSParagraphStyle
    }

    func testACodeBlockKeepsItsRoomAboveTheFirstLineAndBelowTheLast() throws {
        let text = try run("Here is the fix:\n\n```swift\nlet a = 1\nlet b = 2\nlet c = 3\n```\n\nThat should do it.")
        let first = try XCTUnwrap(style(in: text, at: "let a"))
        let middle = try XCTUnwrap(style(in: text, at: "let b"))
        let last = try XCTUnwrap(style(in: text, at: "let c"))
        XCTAssertEqual(first.paragraphSpacingBefore, Markdown.codePaddingY)
        XCTAssertEqual(first.paragraphSpacing, 0)
        XCTAssertEqual(middle.paragraphSpacingBefore, 0)
        XCTAssertEqual(middle.paragraphSpacing, 0)
        XCTAssertEqual(last.paragraphSpacingBefore, 0)
        XCTAssertEqual(last.paragraphSpacing, Markdown.codeGap + Markdown.codePaddingY)
    }

    func testASoftBreakIsALineBreakNotAParagraphGap() throws {
        let text = try run("Line one\nline two\n\nNext paragraph.")
        XCTAssertEqual(style(in: text, at: "Line one")?.paragraphSpacing, 0)
        XCTAssertEqual(style(in: text, at: "line two")?.paragraphSpacing, Markdown.paragraphGap)
    }

    func testAListItemsSecondLineStartsUnderItsText() throws {
        let text = try run("- first item\n  continued here\n- second item")
        let first = try XCTUnwrap(style(in: text, at: "first item"))
        let continued = try XCTUnwrap(style(in: text, at: "continued"))
        XCTAssertEqual(first.firstLineHeadIndent, 0)
        XCTAssertEqual(continued.firstLineHeadIndent, first.headIndent)
        XCTAssertEqual(continued.paragraphSpacing, Markdown.itemGap)
    }

    /// The width measured for a message is one its lines fit in as they are; a code line keeps
    /// the padding of its box.
    func testCodeFitsTheWidthMeasuredForIt() throws {
        for raw in ["```\nonly code\nsecond line\n```", "- item\n\n  ```\n  code one\n  ```", "> just a quote"] {
            let text = try run(raw)
            let wide = TextMeasure.textSize(of: text, width: 400)
            let fitted = TextMeasure.textSize(of: text, width: wide.width)
            XCTAssertEqual(fitted.height, wide.height, raw)
        }
    }
}
