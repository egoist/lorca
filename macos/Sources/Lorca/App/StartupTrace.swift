import Foundation

/// Opt-in launch timings (`LORCA_TRACE_STARTUP=1`), measured from entering main.
@MainActor
enum StartupTrace {
    private static let started = ProcessInfo.processInfo.systemUptime
    private static let enabled = ProcessInfo.processInfo.environment["LORCA_TRACE_STARTUP"] == "1"
    private static var phases: Set<String> = []
    private static let output: FileHandle? = {
        let directory = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Logs/\(AppInfo.name)")
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let url = directory.appendingPathComponent("startup.log")
        FileManager.default.createFile(atPath: url.path, contents: nil)
        return try? FileHandle(forWritingTo: url)
    }()

    static func mark(_ phase: String) {
        guard enabled, phases.insert(phase).inserted else { return }
        let start = started
        let elapsed = (ProcessInfo.processInfo.systemUptime - start) * 1_000
        let line = String(format: "+%.1f ms: %@\n", elapsed, phase)
        try? output?.write(contentsOf: Data(line.utf8))
    }
}
