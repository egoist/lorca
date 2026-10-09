import AppKit

/// Durable work from the CLI, independent of a single running bot turn or terminal command.
struct DurableTask: Codable, Hashable, Identifiable {
    var id: String
    var revision: UInt64
    var authorityRunnerId: String
    var ownerBotId: String
    var runnerId: String
    var goal: String
    var acceptanceCriteria: [String]
    var dependencies: [String]
    var nextAction: String
    var chatIds: [String]
    var links: [Link]
    var state: State
    var reason: String?
    var result: String?
    var evidence: [Evidence]
    var activeRun: Run?
    var createdAt: Double
    var updatedAt: Double

    enum State: String, Codable {
        case queued, working, blocked, awaitingReview = "awaiting_review", completed, cancelled

        var title: String {
            switch self {
            case .queued: return L("Not started")
            case .working: return L("Working")
            case .blocked: return L("Blocked")
            case .awaitingReview: return L("Ready for review")
            case .completed: return L("Completed")
            case .cancelled: return L("Cancelled")
            }
        }

        var symbol: String {
            switch self {
            case .queued: return "circle"
            case .working: return "arrow.triangle.2.circlepath"
            case .blocked: return "exclamationmark.circle.fill"
            case .awaitingReview: return "eye"
            case .completed: return "checkmark.circle"
            case .cancelled: return "xmark.circle"
            }
        }

        /// Color for the states that wait on the user; the rest stay quiet.
        var tint: NSColor {
            switch self {
            case .blocked: return .systemOrange
            case .awaitingReview: return .controlAccentColor
            case .working, .queued: return .secondaryLabelColor
            case .completed, .cancelled: return .tertiaryLabelColor
            }
        }

        var isFinished: Bool { self == .completed || self == .cancelled }

        /// Open work first, what waits on the user before the rest.
        var order: Int {
            switch self {
            case .blocked: return 0
            case .awaitingReview: return 1
            case .working: return 2
            case .queued: return 3
            case .completed, .cancelled: return 4
            }
        }
    }

    struct Link: Codable, Hashable { var label: String; var url: String }
    struct Run: Codable, Hashable {
        var id: String; var botId: String; var runnerId: String; var chatId: String; var startedAt: Double
    }
    struct Evidence: Codable, Hashable {
        var kind: String
        var label: String
        var chatId: String? = nil
        var messageId: String? = nil
        var attachmentId: String? = nil
        var url: String? = nil
        var outputId: String? = nil
        var version: UInt64? = nil
        var reviewId: String? = nil

        var symbol: String {
            switch kind {
            case "url": return "link"
            case "file": return "doc.text"
            case "output": return "doc.richtext"
            case "review": return "checkmark.circle"
            default: return "bubble.left"
            }
        }
    }

    /// Why a task the user cancelled in the app stopped, for its bot to read. The app shows the
    /// state alone for it.
    static let cancelledByUser = "Cancelled by the user."

    /// Can start a run: queued or blocked, and not running.
    var canStart: Bool { (state == .queued || state == .blocked) && activeRun == nil }
}
