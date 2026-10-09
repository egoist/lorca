import Foundation

/// Secrets the user saved for bots: a password, an API key, or a one-time code a bot asked for on
/// a card. They are kept on the bot's Runner, and a value only ever travels there: when the user
/// answers a card or replaces one. The Secrets pane lists what each is, never its value.
extension AppStore {
    /// The secrets kept on `runnerID`, by bot and then by label.
    func secrets(on runnerID: Device.ID) async throws -> [SavedSecret] {
        if isMock { return MockData.secrets.filter { $0.runnerID == runnerID }.map(\.secret) }
        return try await client.request("secrets.list", ["runner_id": runnerID], as: Wire.SecretList.self).secrets.map { $0.toModel() }
    }

    /// A new value for a secret, from the Secrets pane.
    func replaceSecret(_ id: SavedSecret.ID, value: String, on runnerID: Device.ID) async throws {
        if isMock {
            guard let index = MockData.secrets.firstIndex(where: { $0.secret.id == id }) else { return }
            MockData.secrets[index].secret.updatedAt = Date()
            return
        }
        _ = try await client.request("secrets.set", ["runner_id": runnerID, "id": id, "value": value])
    }

    func deleteSecret(_ id: SavedSecret.ID, on runnerID: Device.ID) async throws {
        if isMock {
            MockData.secrets.removeAll { $0.secret.id == id }
            return
        }
        _ = try await client.request("secrets.delete", ["runner_id": runnerID, "id": id])
    }

    /// Answers a secret request with a value for each of its fields, by name. The CLI seals them to
    /// the bot's Runner; the card reads Saved once the Runner has them. Throws why they did not go,
    /// such as the Runner being offline.
    func answerSecret(chatID: Chat.ID, messageID: Message.ID, values: [String: String]) async throws {
        if !isMock {
            _ = try await client.request(
                "chats.permission", ["chat_id": chatID, "message_id": messageID, "decision": "allow", "values": values])
        }
        update(messageID, in: chatID) { message in
            guard case var .permission(request) = message.body, request.isPending else { return }
            request.decision = .allowed
            message.body = .permission(request)
        }
    }
}
