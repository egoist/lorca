import Foundation

enum Format {
    private static let timeFormatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.dateFormat = "h:mm a"
        return formatter
    }()

    private static let weekdayFormatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.dateFormat = "EEEE"
        return formatter
    }()

    private static let shortDateFormatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.dateFormat = "M/d"
        return formatter
    }()

    private static let shortDateYearFormatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.dateFormat = "M/d/yy"
        return formatter
    }()

    private static let dateTimeFormatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.dateFormat = "EEE, MMM d h:mm a"
        return formatter
    }()

    /// Stamp for sidebar rows: the time today, "Yesterday", the weekday within a week, "9/2"
    /// this year, "9/2/25" before that.
    static func stamp(_ date: Date) -> String {
        let calendar = Calendar.current
        if calendar.isDateInToday(date) { return time(date) }
        if calendar.isDateInYesterday(date) { return "Yesterday" }
        if Date().timeIntervalSince(date) < 60 * 60 * 24 * 7 { return weekdayFormatter.string(from: date) }
        if calendar.isDate(date, equalTo: Date(), toGranularity: .year) { return shortDateFormatter.string(from: date) }
        return shortDateYearFormatter.string(from: date)
    }

    static func time(_ date: Date) -> String {
        timeFormatter.string(from: date)
    }

    /// Separator before a cluster of messages: "Today 4:13 AM", "Yesterday 9:55 AM",
    /// "Thu, Sep 10 9:48 AM".
    static func daySeparator(_ date: Date) -> String {
        let calendar = Calendar.current
        if calendar.isDateInToday(date) { return "Today \(time(date))" }
        if calendar.isDateInYesterday(date) { return "Yesterday \(time(date))" }
        return dateTimeFormatter.string(from: date)
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
