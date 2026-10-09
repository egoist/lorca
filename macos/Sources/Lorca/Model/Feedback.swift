import Foundation

/// What a note, a suggestion, or a change is about: a routine's task, or a saved skill of the
/// bot's or of a group it is in. The CLI names them; the app sends one back as it came.
struct FeedbackTarget: Decodable, Hashable {
    struct Scope: Decodable, Hashable {
        var kind: String
        var id: String
    }

    var kind: String
    var id: String?
    var scope: Scope?

    var isSkill: Bool { kind == "playbook" }

    var params: [String: Any] {
        var params: [String: Any] = ["kind": kind]
        if let id { params["id"] = id }
        if let scope { params["scope"] = ["kind": scope.kind, "id": scope.id] }
        return params
    }
}

/// One thing the user said about a bot's work, or a routine run that failed, with the message it
/// is about.
struct FeedbackNote: Hashable, Identifiable {
    enum Kind: String {
        case accepted
        case rejected
        case edited
        case explicit
        case routineFailure = "routine_failure"
        case ignoredAlert = "ignored_alert"
    }

    let id: String
    let kind: Kind
    let chatID: Chat.ID
    let messageID: Message.ID
    /// The user's words, else the start of the message it is about.
    let text: String
    let target: FeedbackTarget?
    let createdAt: Date
}

/// A change the bot suggests to one of its routines or skills, waiting for the user. Accepting
/// or rejecting it sends back `diffHash`, so a decision is about the change the user saw.
struct FeedbackSuggestion: Hashable, Identifiable {
    let id: String
    let target: FeedbackTarget
    let explanation: String
    let diff: String
    let diffHash: String
    let evidence: [FeedbackNote.ID]
    let createdAt: Date
}

/// A change the user accepted, or the undo of one. It can be undone while the routine or skill
/// still reads as the change left it; `currentHash` guards that.
struct FeedbackChange: Hashable, Identifiable {
    let id: String
    let target: FeedbackTarget
    let diff: String
    let isUndo: Bool
    let canUndo: Bool
    let currentHash: String?
    let createdAt: Date
}

/// A bot's workflow feedback, as its Runner reports it: the newest notes, the suggestions waiting,
/// and the newest changes, each newest first.
struct BotFeedback: Hashable {
    var notes: [FeedbackNote]
    var noteCount: Int
    var suggestions: [FeedbackSuggestion]
    var changes: [FeedbackChange]
    /// How often the bot looks for changes to suggest, or nil when it does only when asked.
    var reviewEvery: Int?
    var targets: [Target]

    struct Target: Hashable {
        var name: String
        var target: FeedbackTarget
    }

    static let empty = BotFeedback(notes: [], noteCount: 0, suggestions: [], changes: [], reviewEvery: nil, targets: [])

    /// Nothing to show yet: the inspector leaves the section out.
    var isEmpty: Bool { noteCount == 0 && suggestions.isEmpty && changes.isEmpty && reviewEvery == nil }

    @MainActor func name(of target: FeedbackTarget) -> String {
        if let known = targets.first(where: { $0.target == target }) { return known.name }
        if target.kind == "routine_prompt", let id = target.id, let routine = AppStore.shared.routine(id) { return routine.name }
        return target.id ?? L("Workflow")
    }

    func note(_ id: FeedbackNote.ID) -> FeedbackNote? {
        notes.first { $0.id == id }
    }
}

extension Wire {
    struct Feedback: Decodable {
        struct Origin: Decodable {
            var chatId: String
            var messageId: String
        }
        struct Note: Decodable {
            var id: String
            var kind: String
            var origin: Origin
            var note: String
            var example: String
            var target: FeedbackTarget?
            var createdAt: Double
        }
        struct Proposal: Decodable {
            var id: String
            var target: FeedbackTarget
            var explanation: String
            var diff: String
            var diffHash: String
            var evidence: [String]
            var createdAt: Double
        }
        struct Revision: Decodable {
            var id: String
            var target: FeedbackTarget
            var createdAt: Double
            var rollbackOf: String?
            var diff: String
            var canRollback: Bool
            var currentHash: String?
        }
        struct Settings: Decodable {
            var reviewEverySecs: Int?
        }
        struct Target: Decodable {
            var name: String
            var target: FeedbackTarget
        }

        var feedback: [Note]
        var feedbackCount: Int
        var proposals: [Proposal]
        var revisions: [Revision]
        var settings: Settings
        var targets: [Target]

        func toModel() -> BotFeedback {
            BotFeedback(
                notes: feedback.compactMap { note in
                    FeedbackNote.Kind(rawValue: note.kind).map { kind in
                        FeedbackNote(
                            id: note.id, kind: kind, chatID: note.origin.chatId, messageID: note.origin.messageId,
                            text: (note.note.isEmpty ? note.example : note.note).trimmingCharacters(in: .whitespacesAndNewlines),
                            target: note.target, createdAt: Date(timeIntervalSince1970: note.createdAt))
                    }
                },
                noteCount: feedbackCount,
                suggestions: proposals.reversed().map {
                    FeedbackSuggestion(
                        id: $0.id, target: $0.target, explanation: $0.explanation, diff: $0.diff, diffHash: $0.diffHash,
                        evidence: $0.evidence, createdAt: Date(timeIntervalSince1970: $0.createdAt))
                },
                changes: revisions.map {
                    FeedbackChange(
                        id: $0.id, target: $0.target, diff: $0.diff, isUndo: $0.rollbackOf != nil, canUndo: $0.canRollback,
                        currentHash: $0.currentHash, createdAt: Date(timeIntervalSince1970: $0.createdAt))
                },
                reviewEvery: settings.reviewEverySecs,
                targets: targets.map { BotFeedback.Target(name: $0.name, target: $0.target) })
        }
    }

    struct FeedbackReview: Decodable {
        var proposals: [Feedback.Proposal]
    }
}

/// A line of a suggested change: the same, taken out, or put in.
enum DiffLine: Hashable {
    case removed(String)
    case added(String)

    /// The changed lines of the CLI's diff, without its file and hunk headers.
    static func lines(of diff: String) -> [DiffLine] {
        diff.split(separator: "\n", omittingEmptySubsequences: false).compactMap { line in
            if line.hasPrefix("---") || line.hasPrefix("+++") || line.hasPrefix("@@") { return nil }
            if line.hasPrefix("-") { return .removed(String(line.dropFirst())) }
            if line.hasPrefix("+") { return .added(String(line.dropFirst())) }
            return nil
        }
    }
}
