import Foundation

/// The account's email address on the relay's mail domain (`mail.get`, the snapshot's `mail`,
/// `mail.changed`). Every bot shares it and writes from an address of its own,
/// `name+tag@domain`; mail without a bot's tag goes to the lead bot.
struct MailStatus: Decodable, Equatable {
    struct Address: Decodable, Equatable {
        enum State: String, Decodable {
            case active
            /// Mail to it bounces for now, because mail sent from it kept bouncing.
            case suspended
        }

        var name: String
        var email: String
        var state: State
    }

    struct BotAddress: Decodable, Equatable {
        var botId: String
        var email: String
    }

    /// The relay offers email. Without it, the apps show nothing of it.
    var available: Bool
    var domain: String?
    var address: Address?
    var leadBotId: String?
    var bots: [BotAddress]

    init(available: Bool, domain: String?, address: Address?, leadBotId: String?, bots: [BotAddress]) {
        self.available = available
        self.domain = domain
        self.address = address
        self.leadBotId = leadBotId
        self.bots = bots
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        available = try container.decodeIfPresent(Bool.self, forKey: .available) ?? false
        domain = try container.decodeIfPresent(String.self, forKey: .domain)
        address = try container.decodeIfPresent(Address.self, forKey: .address)
        leadBotId = try container.decodeIfPresent(String.self, forKey: .leadBotId)
        bots = try container.decodeIfPresent([BotAddress].self, forKey: .bots) ?? []
    }

    private enum CodingKeys: String, CodingKey {
        case available, domain, address, leadBotId, bots
    }

    func email(of botID: Bot.ID) -> String? {
        bots.first { $0.botId == botID }?.email
    }

    /// The id of the built-in connection a bot's Access lists for email.
    static let connectionID = "email"

    /// Lowercases what is typed; the CLI checks the name again.
    static func normalized(_ name: String) -> String {
        name.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
    }

    /// 3 to 32 letters, digits, dots, and hyphens, starting and ending with a letter or digit,
    /// with no two dots in a row.
    static func isValidName(_ name: String) -> Bool {
        let allowed = Set("abcdefghijklmnopqrstuvwxyz0123456789.-")
        guard (3...32).contains(name.count), name.allSatisfy(allowed.contains), !name.contains("..") else { return false }
        let ends = Set("abcdefghijklmnopqrstuvwxyz0123456789")
        return name.first.map(ends.contains) == true && name.last.map(ends.contains) == true
    }
}

/// Why `mail.apply` took no name.
enum MailNameProblem: String, Decodable {
    case taken
    case reserved
    case invalid

    var text: String {
        switch self {
        case .taken: L("That name is taken.")
        case .reserved: L("That name is reserved.")
        case .invalid: L("Use letters, digits, dots, and hyphens.")
        }
    }
}

extension Wire {
    /// `mail.apply` and `mail.release`: the status, or why the name was not taken.
    struct MailReply: Decodable {
        var mail: MailStatus?
        var problem: MailNameProblem?
    }
}
