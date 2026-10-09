import Foundation

/// What a turn, a routine, or a task may use on its Runner. A missing limit is no limit.
struct BudgetLimits: Decodable, Hashable {
    var maxUsd: Double?
    var maxTokens: Int?
    var maxRuntimeSecs: Int?
    var maxRetries: Int?
    var maxConnectorCalls: Int?

    var isEmpty: Bool { self == BudgetLimits() }

    var params: [String: Any] {
        var params: [String: Any] = [:]
        params["max_usd"] = maxUsd
        params["max_tokens"] = maxTokens
        params["max_runtime_secs"] = maxRuntimeSecs
        params["max_retries"] = maxRetries
        params["max_connector_calls"] = maxConnectorCalls
        return params
    }

    /// "$1.00 · 50k tokens", or None.
    var summary: String {
        var parts: [String] = []
        if let maxUsd { parts.append(Format.dollars(maxUsd)) }
        if let maxTokens { parts.append(L("%@ tokens", Format.tokens(maxTokens))) }
        if let maxRuntimeSecs { parts.append(Format.minutes(maxRuntimeSecs)) }
        if let maxRetries { parts.append(maxRetries == 1 ? L("1 retry") : L("%d retries", maxRetries)) }
        if let maxConnectorCalls { parts.append(maxConnectorCalls == 1 ? L("1 plugin call") : L("%d plugin calls", maxConnectorCalls)) }
        return parts.isEmpty ? L("None", context: "limits") : parts.joined(separator: " · ")
    }
}

/// One allowance as its Runner keeps it: the limits, what the work has used, and whether it
/// stopped. A stopped turn or routine waits until the user resumes it.
struct BudgetState: Decodable, Hashable {
    struct Usage: Decodable, Hashable {
        var tokens: Int
        var apiCostUsd: Double
        var subscriptionEstimateUsd: Double
        var unknownPriceCalls: Int
        var runtimeSecs: Double
        var retries: Int
        var connectorCalls: Int

        var dollars: Double { apiCostUsd + subscriptionEstimateUsd }
    }

    /// `chat` is the limits each new turn in a DM starts with; `job` is one turn.
    var kind: String
    var id: String
    var runnerId: String
    var chatId: String
    var limits: BudgetLimits
    var usage: Usage
    /// `ready`, `running`, `complete`, `budget_exhausted`, or `interrupted`.
    var state: String
    /// The limit it reached: `usd`, `tokens`, `runtime`, `retries`, or `connector_calls`.
    var reached: String?
    var updatedAt: Double

    var isStopped: Bool { state == "budget_exhausted" || state == "interrupted" }

    /// The inspector's word for a stopped allowance.
    var stoppedLabel: String { state == "interrupted" ? L("Interrupted") : L("Limit reached") }

    /// The status card's title: which limit, or the interruption.
    var stoppedTitle: String {
        guard state == "budget_exhausted" else { return L("Interrupted") }
        switch reached {
        case "usd": return L("Spending limit reached")
        case "tokens": return L("Token limit reached")
        case "runtime": return L("Run time limit reached")
        case "retries": return L("Retry limit reached")
        case "connector_calls": return L("Plugin call limit reached")
        case "unknown_price": return L("Price unknown")
        default: return L("Limit reached")
        }
    }

    /// Below the title: what it used of the limit it reached, or why it stopped.
    var stoppedDetail: String {
        guard state == "budget_exhausted" else {
            return L("The Runner restarted while this was running. Check what it already did, then resume.")
        }
        switch reached {
        case "usd":
            return L("Used %@ of %@.", Format.spend(api: usage.apiCostUsd, estimate: usage.subscriptionEstimateUsd), Format.dollars(limits.maxUsd ?? 0))
        case "tokens":
            return L("Used %@ of %@ tokens.", Format.count(usage.tokens), Format.count(limits.maxTokens ?? 0))
        case "runtime":
            return L("Ran %@ of %@.", Format.minutes(Int(usage.runtimeSecs)), Format.minutes(limits.maxRuntimeSecs ?? 0))
        case "retries":
            return L("Retried %d of %d times.", usage.retries, limits.maxRetries ?? 0)
        case "connector_calls":
            return L("Made %d of %d plugin calls.", usage.connectorCalls, limits.maxConnectorCalls ?? 0)
        case "unknown_price":
            return L("A spending limit can't cover a model without a known price. Add a token or run time limit.")
        default:
            return L("Raise a limit to resume.")
        }
    }

    /// New limits leave room for more work: the limit it reached went up (or away), and no
    /// other limit is used up. Otherwise resuming needs a fresh allowance.
    func fits(_ new: BudgetLimits) -> Bool {
        func raised<T: Comparable>(_ new: T?, _ old: T?) -> Bool { new == nil || old.map { new! > $0 } ?? true }
        let roomForReached =
            switch reached {
            case "usd": raised(new.maxUsd, limits.maxUsd)
            case "unknown_price": new.maxUsd == nil || new.maxTokens != nil || new.maxRuntimeSecs != nil
            case "tokens": raised(new.maxTokens, limits.maxTokens)
            case "runtime": raised(new.maxRuntimeSecs, limits.maxRuntimeSecs)
            case "retries": raised(new.maxRetries, limits.maxRetries)
            case "connector_calls": raised(new.maxConnectorCalls, limits.maxConnectorCalls)
            default: true
            }
        if let max = new.maxUsd, usage.dollars >= max { return false }
        if let max = new.maxTokens, usage.tokens >= max { return false }
        if let max = new.maxRuntimeSecs, usage.runtimeSecs >= Double(max) { return false }
        if let max = new.maxConnectorCalls, max > 0, usage.connectorCalls >= max { return false }
        return roomForReached
    }
}

/// A plugin account's shared call limit on its Runner.
struct CallLimits: Hashable {
    var maxCalls: Int
    var windowSecs: Int
    var maxConcurrency: Int
    /// When the service asked to slow down, the calls wait until then.
    var retryAt: Date? = nil
    /// Another account of the same service shares a service-wide limit.
    var sharesService: Bool

    /// "60 a minute", "10 every 30 s".
    var summary: String {
        switch windowSecs {
        case 60: L("%d a minute", maxCalls)
        case 3600: L("%d an hour", maxCalls)
        case 86_400: L("%d a day", maxCalls)
        default: L("%d every %@", maxCalls, Format.duration(windowSecs))
        }
    }
}

extension Format {
    /// "$5.00", "<$0.01".
    static func dollars(_ value: Double) -> String {
        value > 0 && value < 0.01 ? "<$0.01" : String(format: "$%.2f", value)
    }

    /// Money the turns used, with an estimate marked as one: "$0.42", "$0.42 est.", or both.
    static func spend(api: Double, estimate: Double) -> String {
        switch (api > 0, estimate > 0) {
        case (true, true): L("%@ + %@ est.", dollars(api), dollars(estimate))
        case (false, true): L("%@ est.", dollars(estimate))
        default: dollars(api)
        }
    }

    /// "12,400"
    static func count(_ value: Int) -> String {
        value.formatted(.number)
    }

    /// "45 min", "2 h 30 min", "30 s".
    static func minutes(_ seconds: Int) -> String {
        if seconds < 60 { return L("%d s", seconds) }
        let minutes = Int((Double(seconds) / 60).rounded())
        if minutes < 60 { return L("%d min", minutes) }
        return minutes % 60 == 0 ? L("%d h", minutes / 60) : L("%d h %d min", minutes / 60, minutes % 60)
    }

    /// A call window: "30 s", "5 min", "2 h".
    static func duration(_ seconds: Int) -> String {
        if seconds % 3600 == 0 { return L("%d h", seconds / 3600) }
        if seconds % 60 == 0 { return L("%d min", seconds / 60) }
        return L("%d s", seconds)
    }
}
