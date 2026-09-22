import Foundation
import Network

/// All mutable state and process/pipe operations are confined to `queue`. Only immutable
/// configuration and events cross the main-thread boundary.
final class CLILaunchWorker: @unchecked Sendable {
    struct Configuration: Sendable {
        let generation: UUID
        let port: Int
        let environment: [String: String]
        let bundle: URL
        let logURL: URL
    }

    enum Failure: Sendable {
        case missingBinary
        case launch(URL, String)
        case exited(Int32, URL)
        case startupClosed(URL)

        @MainActor var message: String {
            switch self {
            case .missingBinary: L("The lorca CLI is not bundled with this build and is not on PATH.")
            case let .launch(binary, reason): L("Could not start %@: %@", binary.path, reason)
            case let .exited(code, log): L("The CLI exited with code %d. See %@.", Int(code), log.path)
            case let .startupClosed(log): L("The CLI closed its startup channel before becoming ready. See %@.", log.path)
            }
        }
    }

    enum Event: Sendable {
        case probing
        case starting
        case ready(external: Bool)
        case failed(Failure)
    }

    private let queue = DispatchQueue(label: "app.lorca.cli-launch", qos: .userInitiated)
    private let onEvent: @Sendable (UUID, Event) -> Void
    private var configuration: Configuration?
    private var stopped = false
    private var process: Process?
    private var ready = false
    private var pipe: Pipe?
    private var reader: DispatchSourceRead?
    private var buffer = Data()
    private var logHandle: FileHandle?
    private var probe: NWConnection?
    private var probeTimeout: DispatchWorkItem?
    private var restart: DispatchWorkItem?
    private var restartDelay: TimeInterval = 1

    init(onEvent: @escaping @Sendable (UUID, Event) -> Void) {
        self.onEvent = onEvent
    }

    func ensureRunning(_ configuration: Configuration) {
        queue.async { self.ensureOnQueue(configuration) }
    }

    func stop() {
        queue.sync {
            stopped = true
            cancelProbe()
            restart?.cancel()
            restart = nil
            stopChild()
        }
    }

    private func ensureOnQueue(_ configuration: Configuration) {
        guard !stopped else { return }
        restart?.cancel()
        restart = nil
        if self.configuration?.generation != configuration.generation {
            cancelProbe()
            stopChild()
            restartDelay = 1
            self.configuration = configuration
        }
        if let process {
            if ready, process.isRunning { emit(.ready(external: false)) }
            return
        }
        guard probe == nil else { return }
        StartupTrace.mark("CLI probe started")
        emit(.probing)
        let connection = NWConnection(
            host: "127.0.0.1", port: NWEndpoint.Port(rawValue: UInt16(configuration.port)) ?? 4862, using: .tcp)
        probe = connection
        connection.stateUpdateHandler = { [weak self, weak connection] state in
            guard let self, let connection, self.probe === connection else { return }
            switch state {
            case .ready: self.finishProbe(answered: true)
            case .waiting, .failed, .cancelled: self.finishProbe(answered: false)
            default: break
            }
        }
        let timeout = DispatchWorkItem { [weak self, weak connection] in
            guard let self, let connection, self.probe === connection else { return }
            self.finishProbe(answered: false)
        }
        probeTimeout = timeout
        connection.start(queue: queue)
        queue.asyncAfter(deadline: .now() + 1.5, execute: timeout)
    }

    private func finishProbe(answered: Bool) {
        cancelProbe()
        guard !stopped else { return }
        if answered {
            StartupTrace.mark("CLI listening")
            emit(.ready(external: true))
        } else {
            spawn()
        }
    }

    private func cancelProbe() {
        probeTimeout?.cancel()
        probeTimeout = nil
        probe?.stateUpdateHandler = nil
        probe?.cancel()
        probe = nil
    }

    private func spawn() {
        guard let configuration else { return }
        guard let binary = Self.locateBinary(environment: configuration.environment, bundle: configuration.bundle) else {
            emit(.failed(.missingBinary))
            return
        }
        StartupTrace.mark("CLI starting")
        emit(.starting)
        let process = Process()
        process.executableURL = binary
        process.arguments = ["serve", "--port", String(configuration.port), "--parent-pid", String(ProcessInfo.processInfo.processIdentifier), "--ready-stdout"]
        process.environment = configuration.environment

        let logURL = configuration.logURL
        try? FileManager.default.createDirectory(at: logURL.deletingLastPathComponent(), withIntermediateDirectories: true)
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
        self.pipe = pipe
        self.process = process
        ready = false
        let handle = pipe.fileHandleForReading
        let reader = DispatchSource.makeReadSource(fileDescriptor: handle.fileDescriptor, queue: queue)
        reader.setEventHandler { [weak self, weak process] in
            guard let self, let process, self.process === process else { return }
            self.readOutput(handle.availableData, from: process)
        }
        // Closing on cancellation avoids reusing a descriptor while Dispatch still watches it.
        reader.setCancelHandler { try? handle.close() }
        self.reader = reader
        reader.resume()

        process.terminationHandler = { [weak self] finished in
            guard let self else { return }
            self.queue.async { [self] in
                guard self.process === finished else { return }
                self.process = nil
                self.closeHandles()
                guard !self.stopped else { return }
                self.emit(.failed(.exited(finished.terminationStatus, logURL)))
                let delay = self.restartDelay
                self.restartDelay = min(self.restartDelay * 2, 15)
                let restart = DispatchWorkItem { [weak self] in
                    guard let self, let configuration = self.configuration else { return }
                    self.ensureOnQueue(configuration)
                }
                self.restart = restart
                self.queue.asyncAfter(deadline: .now() + delay, execute: restart)
            }
        }
        do {
            try process.run()
            StartupTrace.mark("CLI spawned")
            try? pipe.fileHandleForWriting.close()
        } catch {
            self.process = nil
            closeHandles()
            emit(.failed(.launch(binary, error.localizedDescription)))
        }
    }

    private func readOutput(_ data: Data, from process: Process) {
        if data.isEmpty {
            reader?.cancel()
            reader = nil
            if !ready, process.isRunning, let configuration {
                emit(.failed(.startupClosed(configuration.logURL)))
                process.terminate()
            }
            return
        }
        try? logHandle?.write(contentsOf: data)
        guard !ready, let configuration else { return }
        buffer.append(data)
        while let newline = buffer.firstIndex(of: 10) {
            let line = Data(buffer[..<newline])
            buffer.removeSubrange(...newline)
            guard let record = (try? JSONSerialization.jsonObject(with: line)) as? [String: Any],
                record["event"] as? String == "ready", record["port"] as? Int == configuration.port
            else { continue }
            buffer.removeAll()
            ready = true
            restartDelay = 1
            StartupTrace.mark("CLI listening")
            emit(.ready(external: false))
            return
        }
        if buffer.count > 65_536 { buffer.removeAll() }
    }

    private func emit(_ event: Event) {
        guard let configuration else { return }
        onEvent(configuration.generation, event)
    }

    private func stopChild() {
        if let process, process.isRunning { process.terminate() }
        process = nil
        closeHandles()
    }

    private func closeHandles() {
        reader?.cancel()
        reader = nil
        try? pipe?.fileHandleForWriting.close()
        pipe = nil
        buffer.removeAll()
        ready = false
        try? logHandle?.close()
        logHandle = nil
    }

    static func locateBinary(environment: [String: String], bundle: URL) -> URL? {
        if let override = environment["LORCA_CLI"], !override.isEmpty { return URL(fileURLWithPath: override) }
        let candidates = [
            bundle.appendingPathComponent("Contents/Resources/bin/lorca"),
            FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".cargo/bin/lorca"),
            URL(fileURLWithPath: "/opt/homebrew/bin/lorca"),
            URL(fileURLWithPath: "/usr/local/bin/lorca"),
        ] + (environment["PATH"] ?? "").split(separator: ":").map {
            URL(fileURLWithPath: String($0)).appendingPathComponent("lorca")
        }
        return candidates.first { FileManager.default.isExecutableFile(atPath: $0.path) }
    }
}
