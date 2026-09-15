import Foundation

enum Format {
    private static let timeFormatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.dateFormat = "h:mm a"
        return formatter
    }()

    private static let weekdayFormatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.dateFormat = "EEE"
        return formatter
    }()

    private static let shortDateFormatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.dateFormat = "M/d/yy"
        return formatter
    }()

    private static let longDateFormatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.dateFormat = "EEEE, MMMM d"
        return formatter
    }()

    /// Compact stamp for sidebar rows: "now", "12m", "3h", "Tue", "9/2/26".
    static func stamp(_ date: Date) -> String {
        let elapsed = Date().timeIntervalSince(date)
        if elapsed < 60 { return "now" }
        if elapsed < 3600 { return "\(Int(elapsed / 60))m" }

        let calendar = Calendar.current
        if calendar.isDateInToday(date) { return "\(Int(elapsed / 3600))h" }
        if calendar.isDateInYesterday(date) { return "Yesterday" }
        if elapsed < 60 * 60 * 24 * 7 { return weekdayFormatter.string(from: date) }
        return shortDateFormatter.string(from: date)
    }

    static func time(_ date: Date) -> String {
        timeFormatter.string(from: date)
    }

    /// Header shown between days in a transcript.
    static func daySeparator(_ date: Date) -> String {
        let calendar = Calendar.current
        if calendar.isDateInToday(date) { return "Today" }
        if calendar.isDateInYesterday(date) { return "Yesterday" }
        return longDateFormatter.string(from: date)
    }

    static func lastSeen(_ date: Date) -> String {
        let elapsed = Date().timeIntervalSince(date)
        if elapsed < 90 { return "Active now" }
        if elapsed < 3600 { return "Last seen \(Int(elapsed / 60))m ago" }
        if elapsed < 60 * 60 * 24 { return "Last seen \(Int(elapsed / 3600))h ago" }
        return "Last seen \(Int(elapsed / 86400))d ago"
    }

    static func isSameDay(_ lhs: Date, _ rhs: Date) -> Bool {
        Calendar.current.isDate(lhs, inSameDayAs: rhs)
    }
}
