import Foundation

/// Exact review payloads come from the local CLI. Dictionary keys inside tool arguments
/// retain their spelling; only the surrounding wire model uses snake-case conversion.
enum ReviewJSON: Codable, Equatable {
    case null, bool(Bool), integer(Int64), unsigned(UInt64), number(Double), string(String), array([ReviewJSON]), object([String: ReviewJSON])

    init(from decoder: Decoder) throws {
        let value = try decoder.singleValueContainer()
        if value.decodeNil() { self = .null }
        else if let bool = try? value.decode(Bool.self) { self = .bool(bool) }
        else if let integer = try? value.decode(Int64.self) { self = .integer(integer) }
        else if let unsigned = try? value.decode(UInt64.self) { self = .unsigned(unsigned) }
        else if let number = try? value.decode(Double.self) { self = .number(number) }
        else if let string = try? value.decode(String.self) { self = .string(string) }
        else if let array = try? value.decode([ReviewJSON].self) { self = .array(array) }
        else { self = .object(try value.decode([String: ReviewJSON].self)) }
    }

    func encode(to encoder: Encoder) throws {
        var value = encoder.singleValueContainer()
        switch self {
        case .null: try value.encodeNil()
        case let .bool(bool): try value.encode(bool)
        case let .integer(integer): try value.encode(integer)
        case let .unsigned(unsigned): try value.encode(unsigned)
        case let .number(number): try value.encode(number)
        case let .string(string): try value.encode(string)
        case let .array(array): try value.encode(array)
        case let .object(object): try value.encode(object)
        }
    }

    var object: Any {
        switch self {
        case .null: NSNull()
        case let .bool(value): value
        case let .integer(value): value
        case let .unsigned(value): value
        case let .number(value): value
        case let .string(value): value
        case let .array(value): value.map(\.object)
        case let .object(value): value.mapValues(\.object)
        }
    }

    var pretty: String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
        return (try? encoder.encode(self)).flatMap { String(data: $0, encoding: .utf8) } ?? ""
    }
}

/// A draft or an exact call a bot left for the user to approve, as its Runner keeps it. Only that
/// Runner changes it; a decision names the version the user saw.
struct ReviewItem: Decodable, Identifiable {
    struct Origin: Decodable {
        var chatId: String
        var messageId: String?
        var routineId: String?
        var taskId: String?
    }
    struct Target: Decodable { var account: String; var resource: String }
    struct Payload: Decodable {
        var kind: String
        var text: String?
        var pluginId: String?
        var serverName: String?
        var tool: String?
        var arguments: ReviewJSON?

        var isDraft: Bool { kind == "draft" }
        var isShell: Bool { kind == "shell" }

        /// What the sheet edits: a draft's text, a shell command, or a call's arguments as JSON.
        var editorText: String {
            switch kind {
            case "draft": text ?? ""
            case "shell": if case let .object(fields) = arguments, case let .string(command) = fields["command"] { command } else { "" }
            default: arguments?.pretty ?? "{}"
            }
        }

        /// The payload with `editedText` in place of what `editorText` showed; a command's other
        /// arguments and a call's server and tool stay as they were.
        func parameters(editedText: String) throws -> [String: Any] {
            switch kind {
            case "draft":
                return ["kind": kind, "text": editedText]
            case "shell":
                var fields = arguments?.object as? [String: Any] ?? [:]
                fields["command"] = editedText
                return ["kind": kind, "arguments": fields]
            default:
                guard let arguments = try? JSONSerialization.jsonObject(with: Data(editedText.utf8)), arguments is [String: Any] else {
                    throw ReviewEditError.argumentsObject
                }
                return ["kind": kind, "plugin_id": pluginId ?? "", "server_name": serverName ?? "", "tool": tool ?? "", "arguments": arguments]
            }
        }
    }
    struct Outcome: Decodable { var summary: String; var result: ReviewJSON?; var messageId: String }
    struct Preconditions: Decodable {
        struct File: Decodable { var path: String; var hash: String? }
        var workdir: String
        var files: [File]
    }

    var id: String
    var runnerId: String
    var botId: String
    var origin: Origin
    var target: Target
    var rationale: String
    var payload: Payload
    var version: UInt64
    var revision: UInt64
    var preconditions: Preconditions
    var state: String
    var outcome: Outcome?
    var createdAt: Double
    /// An email or Slack message: its draft card in the chat is where it is decided.
    var isMessage: Bool?

    var isPending: Bool { state == "pending" }
    /// Waiting for the user, or approved and about to run.
    var isOpen: Bool { isPending || state == "approved" || state == "executing" }

    /// How it ended or where it stands, in a word or two; nil while it waits for the user.
    var stateText: String? {
        switch state {
        case "approved", "executing": L("Running…")
        case "succeeded": payload.isDraft ? L("Accepted") : L("Done")
        case "failed": L("Failed")
        case "rejected": L("Rejected")
        case "cancelled": L("Cancelled")
        case "uncertain": L("Didn't finish")
        default: nil
        }
    }

    /// The command, or what a draft is for; the inspector names a call by its plugin instead.
    var headline: String {
        payload.isShell ? payload.editorText.split(separator: "\n").map { $0.trimmingCharacters(in: .whitespaces) }.first { !$0.isEmpty } ?? "" : target.resource
    }

    /// What the call printed or returned, or the accepted draft.
    var output: String? {
        guard case let .object(fields) = outcome?.result, case let .string(text) = fields["text"], !text.isEmpty else { return nil }
        return text
    }
}

enum ReviewEditError: LocalizedError {
    case argumentsObject
    var errorDescription: String? { L("The arguments need to be a JSON object.") }
}
