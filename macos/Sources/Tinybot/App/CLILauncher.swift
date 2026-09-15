import Foundation
import Network

/// Runs the CLI as a child of the app. The binary ships inside the bundle; a `tinybot serve`
/// already listening on the port (a terminal session, or a dev build) is used instead of a
/// second instance.
@MainActor
final class CLILauncher {
    enum Status: Equatable {
        case idle
        case probing
        case starting
        case running(external: Bool)
        case failed(String)

        var message: String {
            switch self {
            case .idle: "Waiting to start the CLI"
            case .probing: "Looking for the CLI…"
            case .starting: "Starting the CLI…"
            case let .running(external): external ? "Using the CLI already running on this Mac" : "CLI started by the app"
            case let .failed(reason): reason
            }
        }
    }

    private(set) var status: Status = .idle {
        didSet { if status != oldValue { onStatusChange?(status) } }
    }
    var onStatusChange: ((Status) -> Void)?

    private var process: Process?
    private var stopping = false
    private var restartDelay: TimeInterval = 1
    private var logHandle: FileHandle?

    var port: Int { Preferences.cliPort }

    /// Where the CLI binary lives. `TINYBOT_CLI` overrides; then the bundle; then PATH.
    static func locateBinary() -> URL? {
        let environment = ProcessInfo.processInfo.environment
        if let override = environment["TINYBOT_CLI"], !override.isEmpty {
            return URL(fileURLWithPath: override)
        }
        let bundle = Bundle.main.bundleURL
        // Not Contents/MacOS: on a case-insensitive volume that would collide with the app binary.
        let candidates = [
            bundle.appendingPathComponent("Contents/Resources/bin/tinybot"),
            URL(fileURLWithPath: NSHomeDirectory()).appendingPathComponent(".cargo/bin/tinybot"),
            URL(fileURLWithPath: "/opt/homebrew/bin/tinybot"),
            URL(fileURLWithPath: "/usr/local/bin/tinybot"),
        ]
        for candidate in candidates where FileManager.default.isExecutableFile(atPath: candidate.path) {
            return candidate
        }
        if let path = environment["PATH"] {
            for directory in path.split(separator: ":") {
                let candidate = URL(fileURLWithPath: String(directory)).appendingPathComponent("tinybot")
                if FileManager.default.isExecutableFile(atPath: candidate.path) { return candidate }
            }
        }
        return nil
    }

    static var logURL: URL {
        let directory = NSHomeDirectory() + "/Library/Logs/Tinybot"
        try? FileManager.default.createDirectory(atPath: directory, withIntermediateDirectories: true)
        return URL(fileURLWithPath: directory).appendingPathComponent("cli.log")
    }

    /// Starts the CLI unless one already answers on the port.
    func ensureRunning() {
        guard process == nil, !stopping else { return }
        status = .probing
        let port = port
        Task { [weak self] in
            let busy = await Self.portAnswers(port)
            guard let self, self.process == nil, !self.stopping else { return }
            if busy {
                self.status = .running(external: true)
            } else {
                self.spawn()
            }
        }
    }

    func stop() {
        stopping = true
        process?.terminate()
        process = nil
        logHandle?.closeFile()
        logHandle = nil
    }

    private func spawn() {
        guard let binary = Self.locateBinary() else {
            status = .failed("The tinybot CLI is not bundled with this build and is not on PATH.")
            return
        }
        status = .starting

        let process = Process()
        process.executableURL = binary
        process.arguments = ["serve", "--port", String(port), "--parent-pid", String(ProcessInfo.processInfo.processIdentifier)]
        var environment = ProcessInfo.processInfo.environment
        environment["RUST_LOG"] = environment["RUST_LOG"] ?? "tinybot=info"
        process.environment = environment

        let logURL = Self.logURL
        if !FileManager.default.fileExists(atPath: logURL.path) {
            FileManager.default.createFile(atPath: logURL.path, contents: nil)
        }
        if let handle = try? FileHandle(forWritingTo: logURL) {
            handle.seekToEndOfFile()
            process.standardOutput = handle
            process.standardError = handle
            logHandle = handle
        }

        process.terminationHandler = { [weak self] finished in
            Task { @MainActor [weak self] in
                guard let self, self.process === finished else { return }
                self.process = nil
                self.logHandle?.closeFile()
                self.logHandle = nil
                guard !self.stopping else { return }
                let code = finished.terminationStatus
                self.status = .failed("The CLI exited with code \(code). See \(logURL.path).")
                let delay = self.restartDelay
                self.restartDelay = min(self.restartDelay * 2, 15)
                try? await Task.sleep(nanoseconds: UInt64(delay * 1_000_000_000))
                self.ensureRunning()
            }
        }

        do {
            try process.run()
            self.process = process
            status = .running(external: false)
        } catch {
            status = .failed("Could not start \(binary.path): \(error.localizedDescription)")
        }
    }

    /// True when something accepts TCP connections on 127.0.0.1:port.
    private static func portAnswers(_ port: Int) async -> Bool {
        await withCheckedContinuation { continuation in
            let connection = NWConnection(
                host: "127.0.0.1", port: NWEndpoint.Port(rawValue: UInt16(port)) ?? 4862, using: .tcp)
            var finished = false
            let finish: (Bool) -> Void = { answered in
                guard !finished else { return }
                finished = true
                connection.cancel()
                continuation.resume(returning: answered)
            }
            connection.stateUpdateHandler = { state in
                switch state {
                case .ready: finish(true)
                case .failed, .cancelled: finish(false)
                case .waiting: finish(false)
                default: break
                }
            }
            connection.start(queue: .global())
            DispatchQueue.global().asyncAfter(deadline: .now() + 1.5) { finish(false) }
        }
    }
}
