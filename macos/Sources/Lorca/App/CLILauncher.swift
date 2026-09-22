import Foundation
import Network

/// Runs the CLI as a child of the app. The binary ships inside the bundle; a `lorca serve`
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
            case .idle: L("Waiting to start the CLI")
            case .probing: L("Looking for the CLI…")
            case .starting: L("Starting the CLI…")
            case let .running(external): external ? L("Using the CLI already running on this computer") : L("CLI started by the app")
            case let .failed(reason): reason
            }
        }
    }

    private(set) var status: Status = .idle {
        didSet { if status != oldValue { onStatusChange?(status) } }
    }
    var onStatusChange: ((Status) -> Void)?
    /// The local server is listening, either in our child or in an existing CLI.
    var onReady: (() -> Void)?

    private var process: Process?
    private var processPort: Int?
    private var readinessPipe: Pipe?
    private var readinessBuffer = Data()
    private var probeTask: Task<Void, Never>?
    private var restartTask: Task<Void, Never>?
    private var stopping = false
    private var restartDelay: TimeInterval = 1
    private var logHandle: FileHandle?

    var port: Int { Preferences.cliPort }

    /// Where the CLI binary lives. `LORCA_CLI` overrides; then the bundle; then PATH.
    static func locateBinary() -> URL? {
        let environment = ProcessInfo.processInfo.environment
        if let override = environment["LORCA_CLI"], !override.isEmpty {
            return URL(fileURLWithPath: override)
        }
        let bundle = Bundle.main.bundleURL
        // Not Contents/MacOS: on a case-insensitive volume that would collide with the app binary.
        let candidates = [
            bundle.appendingPathComponent("Contents/Resources/bin/lorca"),
            URL(fileURLWithPath: NSHomeDirectory()).appendingPathComponent(".cargo/bin/lorca"),
            URL(fileURLWithPath: "/opt/homebrew/bin/lorca"),
            URL(fileURLWithPath: "/usr/local/bin/lorca"),
        ]
        for candidate in candidates where FileManager.default.isExecutableFile(atPath: candidate.path) {
            return candidate
        }
        if let path = environment["PATH"] {
            for directory in path.split(separator: ":") {
                let candidate = URL(fileURLWithPath: String(directory)).appendingPathComponent("lorca")
                if FileManager.default.isExecutableFile(atPath: candidate.path) { return candidate }
            }
        }
        return nil
    }

    static var logURL: URL {
        let directory = NSHomeDirectory() + "/Library/Logs/\(AppInfo.name)"
        try? FileManager.default.createDirectory(atPath: directory, withIntermediateDirectories: true)
        return URL(fileURLWithPath: directory).appendingPathComponent("cli.log")
    }

    /// Starts the CLI unless one already answers on the port.
    func ensureRunning() {
        guard !stopping else { return }
        restartTask?.cancel()
        restartTask = nil
        if let process {
            if processPort == port {
                if process.isRunning, case .running = status { onReady?() }
                return
            }
            // A port change replaces only the child this launcher owns.
            self.process = nil
            if process.isRunning { process.terminate() }
            closeChildHandles()
        }
        guard probeTask == nil else { return }
        status = .probing
        let port = port
        probeTask = Task { [weak self] in
            let busy = await Self.portAnswers(port)
            guard !Task.isCancelled, let self, !self.stopping else { return }
            self.probeTask = nil
            guard self.port == port else { self.ensureRunning(); return }
            if busy {
                self.status = .running(external: true)
                self.onReady?()
            } else {
                self.spawn(port: port)
            }
        }
    }

    func stop() {
        stopping = true
        probeTask?.cancel()
        probeTask = nil
        restartTask?.cancel()
        restartTask = nil
        if let process, process.isRunning { process.terminate() }
        process = nil
        closeChildHandles()
    }

    private func spawn(port: Int) {
        guard let binary = Self.locateBinary() else {
            status = .failed(L("The lorca CLI is not bundled with this build and is not on PATH."))
            return
        }
        status = .starting

        let process = Process()
        process.executableURL = binary
        process.arguments = ["serve", "--port", String(port), "--parent-pid", String(ProcessInfo.processInfo.processIdentifier), "--ready-stdout"]
        var environment = ProcessInfo.processInfo.environment
        environment["RUST_LOG"] = environment["RUST_LOG"] ?? "lorca=info"
        environment["LORCA_HOME"] = environment["LORCA_HOME"] ?? AppInfo.defaultCLIHome.path
        // The fallback when neither Settings, `LORCA_RELAY_URL`, nor pairing names a relay.
        if !AppInfo.isDevelopment, environment["LORCA_DEFAULT_RELAY_URL"] == nil {
            environment["LORCA_DEFAULT_RELAY_URL"] = AppInfo.productionRelayURL
        }
        process.environment = environment

        let logURL = Self.logURL
        if !FileManager.default.fileExists(atPath: logURL.path) {
            FileManager.default.createFile(atPath: logURL.path, contents: nil)
        }
        if let handle = try? FileHandle(forWritingTo: logURL) {
            handle.seekToEndOfFile()
            process.standardError = handle
            logHandle = handle
        }

        let pipe = Pipe()
        process.standardOutput = pipe
        readinessPipe = pipe
        readinessBuffer.removeAll()
        pipe.fileHandleForReading.readabilityHandler = { [weak self, weak process] handle in
            // `read(upToCount:)` can wait to fill its buffer on a pipe. Read only the bytes
            // already available: the child keeps stdout open after its short ready record.
            let data = handle.availableData
            if data.isEmpty { handle.readabilityHandler = nil }
            DispatchQueue.main.async { [weak self, weak process] in
                guard let self, let process, self.process === process else { return }
                self.readStartupOutput(data, from: process)
            }
        }

        process.terminationHandler = { [weak self] finished in
            Task { @MainActor [weak self] in
                guard let self, self.process === finished else { return }
                self.process = nil
                self.closeChildHandles()
                guard !self.stopping else { return }
                let code = finished.terminationStatus
                self.status = .failed(L("The CLI exited with code %d. See %@.", Int(code), logURL.path))
                let delay = self.restartDelay
                self.restartDelay = min(self.restartDelay * 2, 15)
                self.restartTask = Task { [weak self] in
                    do { try await Task.sleep(nanoseconds: UInt64(delay * 1_000_000_000)) } catch { return }
                    self?.ensureRunning()
                }
            }
        }

        do {
            try process.run()
            self.process = process
            processPort = port
            // Only the child holds a writer now, so exit produces EOF in the startup channel.
            try? pipe.fileHandleForWriting.close()
        } catch {
            closeChildHandles()
            status = .failed(L("Could not start %@: %@", binary.path, error.localizedDescription))
        }
    }

    private func readStartupOutput(_ data: Data, from process: Process) {
        if data.isEmpty {
            if case .starting = status, process.isRunning {
                status = .failed(L("The CLI closed its startup channel before becoming ready. See %@.", Self.logURL.path))
                process.terminate()
            }
            return
        }
        // Keep draining stdout after readiness, and preserve any output alongside stderr.
        try? logHandle?.write(contentsOf: data)
        guard case .starting = status else { return }
        readinessBuffer.append(data)
        while let newline = readinessBuffer.firstIndex(of: 10) {
            let line = Data(readinessBuffer[..<newline])
            readinessBuffer.removeSubrange(...newline)
            guard let record = (try? JSONSerialization.jsonObject(with: line)) as? [String: Any],
                record["event"] as? String == "ready", record["port"] as? Int == processPort
            else { continue }
            readinessBuffer.removeAll()
            restartDelay = 1
            status = .running(external: false)
            onReady?()
            return
        }
        // Readiness is one short JSON line; unrelated unterminated output stays in the log.
        if readinessBuffer.count > 65_536 { readinessBuffer.removeAll() }
    }

    private func closeChildHandles() {
        readinessPipe?.fileHandleForReading.readabilityHandler = nil
        try? readinessPipe?.fileHandleForReading.close()
        try? readinessPipe?.fileHandleForWriting.close()
        readinessPipe = nil
        readinessBuffer.removeAll()
        processPort = nil
        try? logHandle?.close()
        logHandle = nil
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
            // The connection callback and timeout serialize their access to `finished`.
            let queue = DispatchQueue(label: "app.lorca.cli-probe")
            connection.start(queue: queue)
            queue.asyncAfter(deadline: .now() + 1.5) { finish(false) }
        }
    }
}
