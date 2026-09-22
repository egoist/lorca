import Foundation

/// Main-thread presentation of the CLI lifecycle. Probing, spawning, and reading readiness
/// run on the worker's serial queue, independently of AppKit constructing the first window.
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
    var onReady: (() -> Void)?

    private var stopping = false
    private var requestedPort: Int?
    private var generation = UUID()
    private lazy var worker = CLILaunchWorker { [weak self] generation, event in
        DispatchQueue.main.async { [weak self] in
            guard let self, !self.stopping, self.generation == generation else { return }
            switch event {
            case .probing: self.status = .probing
            case .starting: self.status = .starting
            case let .ready(external):
                self.status = .running(external: external)
                self.onReady?()
            case let .failed(failure): self.status = .failed(failure.message)
            }
        }
    }

    var port: Int { Preferences.cliPort }

    static func locateBinary() -> URL? {
        CLILaunchWorker.locateBinary(environment: ProcessInfo.processInfo.environment, bundle: Bundle.main.bundleURL)
    }

    static var logURL: URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Logs/\(AppInfo.name)/cli.log")
    }

    func ensureRunning() {
        guard !stopping else { return }
        let port = port
        if requestedPort != port {
            requestedPort = port
            generation = UUID()
        }
        var environment = ProcessInfo.processInfo.environment
        environment["RUST_LOG"] = environment["RUST_LOG"] ?? "lorca=info"
        environment["LORCA_HOME"] = environment["LORCA_HOME"] ?? AppInfo.defaultCLIHome.path
        if !AppInfo.isDevelopment, environment["LORCA_DEFAULT_RELAY_URL"] == nil {
            environment["LORCA_DEFAULT_RELAY_URL"] = AppInfo.productionRelayURL
        }
        worker.ensureRunning(.init(
            generation: generation, port: port, environment: environment,
            bundle: Bundle.main.bundleURL, logURL: Self.logURL))
    }

    func stop() {
        stopping = true
        // Fence queued starts before returning: quitting must never leave a child that starts
        // after the main thread has already stopped observing it.
        worker.stop()
    }
}
