import Foundation

/// The app's only network client: a websocket to the local CLI. Reconnects on its own.
@MainActor
final class CLIClient: NSObject {
    enum ConnectionState: Equatable {
        case disconnected
        case connecting
        case connected
    }

    struct RequestError: LocalizedError {
        let message: String
        var errorDescription: String? { message }
    }

    private(set) var state: ConnectionState = .disconnected

    /// Called on the main actor whenever the connection opens or drops.
    var onStateChange: ((ConnectionState) -> Void)?
    /// Called on the main actor for every event frame, with the raw JSON.
    var onEvent: ((String, Data) -> Void)?

    private var session: URLSession!
    private var task: URLSessionWebSocketTask?
    private var pending: [Int: CheckedContinuation<Data, Error>] = [:]
    private var nextID = 1
    private var wantsConnection = false
    private var reconnectDelay: TimeInterval = 0.4
    private var generation = 0

    override init() {
        super.init()
        let configuration = URLSessionConfiguration.ephemeral
        configuration.waitsForConnectivity = false
        session = URLSession(configuration: configuration, delegate: self, delegateQueue: nil)
    }

    var url: URL {
        URL(string: "ws://127.0.0.1:\(Preferences.cliPort)/ws")!
    }

    func connect() {
        wantsConnection = true
        guard task == nil else { return }
        open()
    }

    func disconnect() {
        wantsConnection = false
        close()
    }

    /// Drops the socket and dials again, for a port change or a manual retry.
    func reconnect() {
        wantsConnection = true
        reconnectDelay = 0.4
        close()
        open()
    }

    private func open() {
        generation += 1
        let myGeneration = generation
        state = .connecting
        let task = session.webSocketTask(with: url)
        // The default is 1 MiB, and a message over it fails the receive, which reads as a dropped
        // connection. A snapshot carries every chat's messages and outgrows that.
        task.maximumMessageSize = 256 * 1024 * 1024
        self.task = task
        task.resume()
        receive(on: task, generation: myGeneration)
    }

    private func close() {
        task?.cancel(with: .goingAway, reason: nil)
        task = nil
        failPending(L("The CLI connection closed"))
        if state != .disconnected {
            state = .disconnected
            onStateChange?(.disconnected)
        }
    }

    private func receive(on task: URLSessionWebSocketTask, generation myGeneration: Int) {
        task.receive { [weak self] result in
            Task { @MainActor [weak self] in
                guard let self, self.generation == myGeneration else { return }
                switch result {
                case let .success(message):
                    switch message {
                    case let .string(text): self.handle(text: text)
                    case let .data(data): self.handle(text: String(decoding: data, as: UTF8.self))
                    @unknown default: break
                    }
                    self.receive(on: task, generation: myGeneration)
                case .failure:
                    self.dropped()
                }
            }
        }
    }

    private func dropped() {
        task = nil
        failPending(L("The CLI connection dropped"))
        if state != .disconnected {
            state = .disconnected
            onStateChange?(.disconnected)
        }
        scheduleReconnect()
    }

    private func scheduleReconnect() {
        guard wantsConnection, task == nil else { return }
        let delay = reconnectDelay
        reconnectDelay = min(reconnectDelay * 1.6, 5)
        Task { @MainActor [weak self] in
            try? await Task.sleep(nanoseconds: UInt64(delay * 1_000_000_000))
            guard let self, self.wantsConnection, self.task == nil else { return }
            self.open()
        }
    }

    private func failPending(_ reason: String) {
        let waiting = pending
        pending.removeAll()
        for continuation in waiting.values {
            continuation.resume(throwing: RequestError(message: reason))
        }
    }

    private func handle(text: String) {
        guard let data = text.data(using: .utf8),
            let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return }

        if let event = object["event"] as? String {
            onEvent?(event, data)
            return
        }

        guard let id = object["id"] as? Int, let continuation = pending.removeValue(forKey: id) else { return }
        if let error = object["error"] as? [String: Any] {
            continuation.resume(throwing: RequestError(message: error["message"] as? String ?? L("Request failed")))
            return
        }
        let result = object["result"] ?? NSNull()
        let encoded = (try? JSONSerialization.data(withJSONObject: result, options: [.fragmentsAllowed])) ?? Data("null".utf8)
        continuation.resume(returning: encoded)
    }

    // MARK: - Requests

    /// Sends a request and returns the JSON-encoded `result`.
    func request(_ method: String, _ params: [String: Any] = [:]) async throws -> Data {
        guard let task, state != .disconnected else {
            throw RequestError(message: L("The Lorca CLI is not running"))
        }
        let id = nextID
        nextID += 1
        let frame: [String: Any] = ["id": id, "method": method, "params": params]
        let body = try JSONSerialization.data(withJSONObject: frame)
        let text = String(decoding: body, as: UTF8.self)

        return try await withCheckedThrowingContinuation { continuation in
            pending[id] = continuation
            task.send(.string(text)) { [weak self] error in
                guard let error else { return }
                Task { @MainActor [weak self] in
                    guard let self, let waiting = self.pending.removeValue(forKey: id) else { return }
                    waiting.resume(throwing: RequestError(message: error.localizedDescription))
                }
            }
        }
    }

    func request<T: Decodable>(_ method: String, _ params: [String: Any] = [:], as type: T.Type) async throws -> T {
        let data = try await request(method, params)
        return try Wire.decoder.decode(T.self, from: data)
    }
}

extension CLIClient: URLSessionWebSocketDelegate {
    nonisolated func urlSession(
        _ session: URLSession, webSocketTask: URLSessionWebSocketTask, didOpenWithProtocol protocol: String?
    ) {
        Task { @MainActor in
            guard webSocketTask === self.task else { return }
            self.reconnectDelay = 0.4
            self.state = .connected
            self.onStateChange?(.connected)
        }
    }

    nonisolated func urlSession(
        _ session: URLSession, webSocketTask: URLSessionWebSocketTask,
        didCloseWith closeCode: URLSessionWebSocketTask.CloseCode, reason: Data?
    ) {
        Task { @MainActor in
            guard webSocketTask === self.task else { return }
            self.dropped()
        }
    }
}
