import AppKit

enum ChatMetrics {
    static let horizontalInset: CGFloat = 22
    static let avatarSize: CGFloat = 26
    static let avatarGutter: CGFloat = 10
    static let headerLineHeight: CGFloat = 16
    static let headerToBody: CGFloat = 2
    static let timeLineHeight: CGFloat = 15
    static let timeGap: CGFloat = 8
    static let groupTopPadding: CGFloat = 14
    static let tightTopPadding: CGFloat = 4
    static let bubblePadX: CGFloat = 14
    static let bubblePadY: CGFloat = 10
    static let maxBubbleWidth: CGFloat = 580
    static let userLeftGutter: CGFloat = 72
    static let dayRowHeight: CGFloat = 42
    static let workingRowHeight: CGFloat = 44
    static let statusRowHeight: CGFloat = 30
    /// A new "Today 4:13 AM" separator after this much silence.
    static let separatorGap: TimeInterval = 15 * 60
    static let noticeMaxWidth: CGFloat = 460
    static let noticePadX: CGFloat = 10
    static let noticePadY: CGFloat = 8
    static let noticeIconSize: CGFloat = 14
    static let noticeIconGap: CGFloat = 8

    static var bubbleIndent: CGFloat { horizontalInset + avatarSize + avatarGutter }

}

/// The name sits above the bubble (group chats only); the bubble holds the text and the stamp.
struct BubbleMetrics {
    var textWidth: CGFloat
    var textHeight: CGFloat
    var timeWidth: CGFloat
    var showsName: Bool
    /// Where the bubble starts: after the avatar column in a group, at the inset in a DM.
    var indent: CGFloat

    var timeGutter: CGFloat {
        timeWidth > 0 ? ChatMetrics.timeGap + timeWidth : 0
    }

    var headerHeight: CGFloat {
        showsName ? ChatMetrics.headerLineHeight + ChatMetrics.headerToBody : 0
    }

    var innerWidth: CGFloat { textWidth + timeGutter }
    var bubbleWidth: CGFloat { innerWidth + ChatMetrics.bubblePadX * 2 }
    var bubbleHeight: CGFloat {
        ChatMetrics.bubblePadY + textHeight + ChatMetrics.bubblePadY
    }
    var rowHeight: CGFloat { headerHeight + bubbleHeight }
}

/// Notice box size, with icon and label frames in box coordinates (y grows downward).
struct NoticeMetrics {
    var boxSize: NSSize
    var iconFrame: NSRect
    var labelFrame: NSRect
}

enum ChatRow: Equatable {
    case day(Date)
    case message(id: Message.ID, groupStart: Bool)
    /// Bots with a turn running, shown after the last message.
    case working([Bot.ID])
    /// A one-line note after the last message, such as "Chef stopped without replying".
    case status(String)

    var messageID: Message.ID? {
        if case let .message(id, _) = self { return id }
        return nil
    }
}

/// Renders and measures message bodies once per text revision; the table asks for
/// heights constantly, and re-parsing Markdown on every token would show.
@MainActor
final class ChatLayout {
    private struct Entry {
        var text: String
        var rendered: RenderedMessage
    }

    private var cache: [Message.ID: Entry] = [:]

    func rendered(for message: Message) -> RenderedMessage {
        let color = message.author.isYou ? Theme.userBubbleText : NSColor.labelColor
        let text = message.text
        if let entry = cache[message.id], entry.text == text {
            return entry.rendered
        }
        let rendered = RenderedMessage(text, textColor: color)
        cache[message.id] = Entry(text: text, rendered: rendered)
        return rendered
    }

    func invalidate(_ id: Message.ID) {
        cache.removeValue(forKey: id)
    }

    func invalidateAll() {
        cache.removeAll()
    }

    func availableBubbleWidth(for message: Message, indent: CGFloat, tableWidth: CGFloat) -> CGFloat {
        let reserved =
            message.author.isYou
            ? ChatMetrics.horizontalInset * 2 + ChatMetrics.userLeftGutter
            : indent + ChatMetrics.horizontalInset
        return max(140, min(ChatMetrics.maxBubbleWidth, tableWidth - reserved))
    }

    /// `showsName` is a group chat's bot message: name above, avatar beside the bubble.
    func metrics(for message: Message, showsName: Bool, tableWidth: CGFloat) -> BubbleMetrics {
        let indent = showsName ? ChatMetrics.bubbleIndent : ChatMetrics.horizontalInset
        let maxBubble = availableBubbleWidth(for: message, indent: indent, tableWidth: tableWidth)
        let maxInner = maxBubble - ChatMetrics.bubblePadX * 2
        let timeWidth: CGFloat = {
            guard Preferences.showTimestamps else { return 0 }
            return TextMeasure.labelSize(
                of: NSAttributedString(
                    string: Format.time(message.createdAt),
                    attributes: [.font: Theme.Font.caption])
            ).width
        }()
        let timeGutter = timeWidth > 0 ? ChatMetrics.timeGap + timeWidth : 0
        let maxText = max(24, maxInner - timeGutter)
        let content = rendered(for: message)
        let textWidth = max(24, min(content.preferredWidth(max: maxText), maxText))
        let textHeight = max(
            ChatMetrics.timeLineHeight,
            content.height(forWidth: textWidth) + 1)
        return BubbleMetrics(
            textWidth: textWidth,
            textHeight: textHeight,
            timeWidth: timeWidth,
            showsName: showsName,
            indent: indent
        )
    }

    /// Hugs the text up to `noticeMaxWidth`, with the icon centered on the first line.
    func noticeMetrics(for text: String, tableWidth: CGFloat) -> NoticeMetrics {
        let inset = TextMeasure.labelInset
        let textX = ChatMetrics.noticePadX + ChatMetrics.noticeIconSize + ChatMetrics.noticeIconGap
        let labelX = textX - inset
        let chromeWidth = textX - inset * 2 + ChatMetrics.noticePadX
        let maxBoxWidth = min(ChatMetrics.noticeMaxWidth, tableWidth - ChatMetrics.horizontalInset * 2)
        let font: [NSAttributedString.Key: Any] = [.font: Theme.Font.notice]
        let line = TextMeasure.labelSize(of: NSAttributedString(string: "X", attributes: font))
        let label = TextMeasure.labelSize(
            of: NSAttributedString(string: text, attributes: font),
            width: max(24, maxBoxWidth - chromeWidth))

        return NoticeMetrics(
            boxSize: NSSize(
                width: label.width + chromeWidth,
                height: ChatMetrics.noticePadY + label.height + ChatMetrics.noticePadY),
            iconFrame: NSRect(
                x: ChatMetrics.noticePadX,
                y: ChatMetrics.noticePadY + (line.height - ChatMetrics.noticeIconSize) / 2,
                width: ChatMetrics.noticeIconSize,
                height: ChatMetrics.noticeIconSize),
            labelFrame: NSRect(origin: NSPoint(x: labelX, y: ChatMetrics.noticePadY), size: label)
        )
    }

    func height(
        for row: ChatRow, message: Message?, tableWidth: CGFloat, showsName: Bool
    ) -> CGFloat {
        switch row {
        case .day:
            return ChatMetrics.dayRowHeight

        case .working:
            return ChatMetrics.workingRowHeight

        case .status:
            return ChatMetrics.statusRowHeight

        case let .message(_, groupStart):
            guard let message else { return 0 }
            let top = groupStart ? ChatMetrics.groupTopPadding : ChatMetrics.tightTopPadding

            switch message.body {
            case .text:
                let metrics = metrics(for: message, showsName: showsName, tableWidth: tableWidth)
                return top + metrics.rowHeight

            // Tool calls never show; only a message_bot row reaches here, as a marker.
            case .tool, .handoff:
                return top + 34

            case let .notice(text):
                return top + noticeMetrics(for: text, tableWidth: tableWidth).boxSize.height
            }
        }
    }
}
