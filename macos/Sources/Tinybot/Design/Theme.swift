import AppKit

enum Accent: String, Hashable, CaseIterable {
    case indigo, blue, teal, green, orange, pink, purple, red

    var color: NSColor {
        switch self {
        case .indigo: .systemIndigo
        case .blue: .systemBlue
        case .teal: .systemTeal
        case .green: .systemGreen
        case .orange: .systemOrange
        case .pink: .systemPink
        case .purple: .systemPurple
        case .red: .systemRed
        }
    }

    /// Slightly lifted twin used as the top stop of avatar gradients.
    var highlight: NSColor {
        color.blended(withFraction: 0.28, of: .white) ?? color
    }
}

enum Theme {
    static func dynamic(
        light: @escaping @Sendable () -> NSColor,
        dark: @escaping @Sendable () -> NSColor
    ) -> NSColor {
        NSColor(name: nil) { appearance in
            appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua ? dark() : light()
        }
    }

    static let botBubble = dynamic(
        light: { NSColor(calibratedWhite: 0.945, alpha: 1) },
        dark: { NSColor(calibratedWhite: 1, alpha: 0.10) }
    )

    static let botBubbleBorder = dynamic(
        light: { NSColor(calibratedWhite: 0, alpha: 0.04) },
        dark: { NSColor(calibratedWhite: 1, alpha: 0.06) }
    )

    /// Sits inside a bubble, so it has to stay legible against the bubble fill
    /// rather than the transcript behind it.
    static let codeBackground = dynamic(
        light: { NSColor(calibratedWhite: 0, alpha: 0.06) },
        dark: { NSColor(calibratedWhite: 0, alpha: 0.24) }
    )

    static let chipBackground = dynamic(
        light: { NSColor(calibratedWhite: 0, alpha: 0.05) },
        dark: { NSColor(calibratedWhite: 1, alpha: 0.08) }
    )

    static let composerField = dynamic(
        light: { NSColor(calibratedWhite: 1, alpha: 1) },
        dark: { NSColor(calibratedWhite: 1, alpha: 0.07) }
    )

    static let transcriptBackground = dynamic(
        light: { NSColor(calibratedWhite: 1, alpha: 1) },
        dark: { NSColor(calibratedWhite: 0.11, alpha: 1) }
    )

    static var userBubble: NSColor { .controlAccentColor }

    /// Accent colors can be light (yellow, graphite); pick legible bubble text.
    static var userBubbleText: NSColor {
        let accent = NSColor.controlAccentColor.usingColorSpace(.sRGB) ?? .systemBlue
        let luminance =
            0.2126 * accent.redComponent + 0.7152 * accent.greenComponent + 0.0722 * accent.blueComponent
        return luminance > 0.62 ? NSColor.black.withAlphaComponent(0.85) : .white
    }

    enum Font {
        static var message: NSFont { .systemFont(ofSize: 13.5) }
        static var messageBold: NSFont { .systemFont(ofSize: 13.5, weight: .semibold) }
        static var code: NSFont { .monospacedSystemFont(ofSize: 12, weight: .regular) }
        static var inlineCode: NSFont { .monospacedSystemFont(ofSize: 12.5, weight: .regular) }
        static var author: NSFont { .systemFont(ofSize: 11, weight: .semibold) }
        static var caption: NSFont { .systemFont(ofSize: 10.5) }
        static var notice: NSFont { .systemFont(ofSize: 11.5) }
        static var sidebarTitle: NSFont { .systemFont(ofSize: 13, weight: .medium) }
        static var sidebarPreview: NSFont { .systemFont(ofSize: 11.5) }
    }

    enum Metric {
        static let bubbleCornerRadius: CGFloat = 14
        static let bubblePaddingX: CGFloat = 12
        static let bubblePaddingY: CGFloat = 9
        static let bubbleMaxWidth: CGFloat = 560
        static let transcriptInsetX: CGFloat = 20
        static let avatarSize: CGFloat = 26
        static let rowSpacing: CGFloat = 4
        static let groupSpacing: CGFloat = 14
    }
}

extension NSColor {
    func hexish() -> String {
        guard let srgb = usingColorSpace(.sRGB) else { return "" }
        return String(
            format: "%02X%02X%02X",
            Int(srgb.redComponent * 255),
            Int(srgb.greenComponent * 255),
            Int(srgb.blueComponent * 255)
        )
    }
}
