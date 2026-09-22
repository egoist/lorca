// Compiled with the production worker by `bun run test:mac-startup`; AppKit is not loaded.
import Foundation
import Darwin

func L(_ text: String, _ args: CVarArg...) -> String { String(format: text, arguments: args) }
enum AppInfo { static let name = "Lorca Launch Tests" }

private final class Events: @unchecked Sendable {
    let ready = DispatchSemaphore(value: 0)
    let failed = DispatchSemaphore(value: 0)
    private let lock = NSLock()
    private var received: [CLILaunchWorker.Event] = []

    var count: Int {
        lock.lock()
        defer { lock.unlock() }
        return received.count
    }

    func receive(_ event: CLILaunchWorker.Event) {
        lock.lock()
        received.append(event)
        lock.unlock()
        switch event {
        case .ready: ready.signal()
        case .failed: failed.signal()
        default: break
        }
    }
}

@main
struct LaunchConcurrency {
    static func main() throws {
        precondition(Thread.isMainThread)
        let directory = URL(fileURLWithPath: CommandLine.arguments[1])
        let complete = "printf '{\"event\":\"ready\",\"port\":%s}\\n' \"$TEST_PORT\"\nexec /bin/sleep 30"
        let fragmented = "printf '{\"event\":\"ready\",'\n/bin/sleep 0.05\nprintf '\"port\":%s}\\n' \"$TEST_PORT\"\nexec /bin/sleep 30"
        for (name, body) in [("main thread occupied", complete), ("fragmented readiness", fragmented), ("early exit", "exit 17"), ("queued stop", complete)] {
            let root = directory.appendingPathComponent(UUID().uuidString)
            try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
            let binary = root.appendingPathComponent("cli")
            try ("#!/bin/sh\necho $$ > \"$TEST_PID_FILE\"\n" + body + "\n").write(to: binary, atomically: true, encoding: .utf8)
            try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: binary.path)
            let pidFile = root.appendingPathComponent("pid")
            let port = try unusedPort()
            let events = Events()
            let worker = CLILaunchWorker { _, event in events.receive(event) }
            var environment = ProcessInfo.processInfo.environment
            environment["LORCA_CLI"] = binary.path
            environment["TEST_PORT"] = String(port)
            environment["TEST_PID_FILE"] = pidFile.path
            let configuration = CLILaunchWorker.Configuration(
                generation: UUID(), port: port, environment: environment,
                bundle: root, logURL: root.appendingPathComponent("cli.log"))
            worker.ensureRunning(configuration)
            if name == "queued stop" {
                worker.stop()
                let count = events.count
                Thread.sleep(forTimeInterval: 0.1)
                precondition(events.count == count, "Events arrived after stopping")
            } else {
                // Keep the main thread completely unavailable. A main-actor probe, spawn,
                // or readiness callback would time out here instead of making progress.
                let signal = name == "early exit" ? events.failed : events.ready
                precondition(signal.wait(timeout: .now() + 5) == .success, "Worker stalled behind the main thread: \(name)")
                if name == "early exit" { precondition(events.ready.wait(timeout: .now()) == .timedOut) }
                worker.stop()
            }
            if let raw = try? String(contentsOf: pidFile, encoding: .utf8), let pid = Int32(raw.trimmingCharacters(in: .whitespacesAndNewlines)) {
                let deadline = Date().addingTimeInterval(2)
                while kill(pid, 0) == 0 && Date() < deadline { Thread.sleep(forTimeInterval: 0.005) }
                precondition(kill(pid, 0) != 0, "Child survived stop: \(name)")
            }
            print("PASS: \(name)")
        }
    }

    private static func unusedPort() throws -> Int {
        let descriptor = socket(AF_INET, SOCK_STREAM, 0)
        precondition(descriptor >= 0)
        defer { close(descriptor) }
        var address = sockaddr_in()
        address.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        address.sin_family = sa_family_t(AF_INET)
        address.sin_addr.s_addr = inet_addr("127.0.0.1")
        var length = socklen_t(MemoryLayout<sockaddr_in>.size)
        try withUnsafeMutablePointer(to: &address) { pointer in
            try pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) { address in
                guard bind(descriptor, address, length) == 0, getsockname(descriptor, address, &length) == 0 else {
                    throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno))
                }
            }
        }
        return Int(UInt16(bigEndian: address.sin_port))
    }
}
