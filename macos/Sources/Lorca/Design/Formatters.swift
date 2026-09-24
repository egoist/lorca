import Foundation

enum Format {
    /// A formatter for a template ("jmm", "EEEE") in the app's language.
    static func formatter(_ template: String) -> DateFormatter {
        let formatter = DateFormatter()
        formatter.locale = AppLanguage.locale
        formatter.setLocalizedDateFormatFromTemplate(template)
        return formatter
    }

    /// The formatters are kept, so a new language makes them again.
    static func languageChanged() {
        timeFormatter = formatter("jmm")
        weekdayFormatter = formatter("EEEE")
        shortDateFormatter = formatter("Md")
        shortDateYearFormatter = formatter("yyMd")
        dateTimeFormatter = formatter("EEEMMMdjmm")
    }

    private static var timeFormatter = formatter("jmm")

    private static var weekdayFormatter = formatter("EEEE")

    private static var shortDateFormatter = formatter("Md")

    private static var shortDateYearFormatter = formatter("yyMd")

    private static var dateTimeFormatter = formatter("EEEMMMdjmm")

    /// Stamp for sidebar rows: the time today, "Yesterday", the weekday within a week, "9/2"
    /// this year, "9/2/25" before that.
    static func stamp(_ date: Date) -> String {
        let calendar = Calendar.current
        if calendar.isDateInToday(date) { return time(date) }
        if calendar.isDateInYesterday(date) { return L("Yesterday") }
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
        if calendar.isDateInToday(date) { return L("Today %@", time(date)) }
        if calendar.isDateInYesterday(date) { return L("Yesterday %@", time(date)) }
        return dateTimeFormatter.string(from: date)
    }

    /// When an offline Device was last on the relay; an online one reads "Online". The relay
    /// stamps `last_seen` as a socket closes, so a Device that just dropped was seen seconds ago.
    static func lastSeen(_ date: Date) -> String {
        let elapsed = Date().timeIntervalSince(date)
        if elapsed < 60 { return L("Last seen just now") }
        if elapsed < 3600 { return L("Last seen %dm ago", Int(elapsed / 60)) }
        if elapsed < 60 * 60 * 24 { return L("Last seen %dh ago", Int(elapsed / 3600)) }
        return L("Last seen %dd ago", Int(elapsed / 86400))
    }

    static func isSameDay(_ lhs: Date, _ rhs: Date) -> Bool {
        Calendar.current.isDate(lhs, inSameDayAs: rhs)
    }
    /// The CLI words a schedule in English ("Weekdays at 9:00 AM and 5:30 PM", "Every 2 hours",
    /// "On the 1st and 15th of every month at 9:00 AM"); in Chinese the same sentence is rebuilt
    /// from its parts, as the phone does (`scheduleText` in `mobile/src/ui/format.ts`). A
    /// sentence that is not one of the CLI's (a raw cron line) reads as it came.
    static func schedule(_ text: String) -> String {
        guard AppLanguage.isChinese else { return text }
        func groups(_ pattern: String, _ input: String) -> [String]? {
            guard let regex = try? NSRegularExpression(pattern: pattern),
                  let match = regex.firstMatch(in: input, range: NSRange(input.startIndex..., in: input)) else { return nil }
            return (1..<match.numberOfRanges).map { Range(match.range(at: $0), in: input).map { String(input[$0]) } ?? "" }
        }
        func list(_ words: String) -> [String] {
            words.replacingOccurrences(of: ", and ", with: ", ").replacingOccurrences(of: " and ", with: ", ").components(separatedBy: ", ")
        }
        func clock(_ word: String) -> String {
            guard let parts = groups(#"^(\d{1,2}):(\d{2}) (AM|PM)$"#, word) else { return word }
            return "\(parts[2] == "PM" ? "下午" : "上午") \(parts[0]):\(parts[1])"
        }
        let units = ["day": "天", "hour": "小时", "minute": "分钟"]
        if let single = groups(#"^Every (day|hour|minute)$"#, text) { return "每\(units[single[0]] ?? single[0])" }
        if let interval = groups(#"^Every (\d+) (day|hour|minute)s$"#, text) { return "每 \(interval[0]) \(units[interval[1]] ?? interval[1])" }
        if let past = groups(#"^Every hour at :(\d{2})$"#, text) { return "每小时的第 \(Int(past[0]) ?? 0) 分" }
        guard let at = text.range(of: " at ", options: .backwards) else { return text }
        let times = list(String(text[at.upperBound...])).map(clock).joined(separator: "、")
        let days = String(text[..<at.lowerBound])
        switch days {
        case "Every day": return "每天 \(times)"
        case "Weekdays": return "工作日 \(times)"
        case "Weekends": return "周末 \(times)"
        default: break
        }
        if let monthly = groups(#"^On the (.+) of every month$"#, days) {
            let dates = list(monthly[0]).map { "\(Int($0.prefix { $0.isNumber }) ?? 0) 日" }.joined(separator: "、")
            return "每月 \(dates) \(times)"
        }
        let weekdays = ["Sunday": "周日", "Monday": "周一", "Tuesday": "周二", "Wednesday": "周三", "Thursday": "周四", "Friday": "周五", "Saturday": "周六"]
        let names = list(days.hasPrefix("Every ") ? String(days.dropFirst(6)) : days).map { weekdays[$0] }
        if names.allSatisfy({ $0 != nil }) { return "每\(names.compactMap { $0 }.joined(separator: "、")) \(times)" }
        return text
    }
}
