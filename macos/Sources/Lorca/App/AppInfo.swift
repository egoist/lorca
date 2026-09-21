import Foundation

enum AppInfo {
    static let name = Bundle.main.object(forInfoDictionaryKey: "CFBundleDisplayName") as? String
        ?? "Lorca"
    static let isDevelopment = Bundle.main.bundleIdentifier == "app.lorca.dev"
    static let defaultCLIHome = FileManager.default.homeDirectoryForCurrentUser
        .appendingPathComponent(isDevelopment ? ".lorca-dev" : ".lorca")
    static let defaultCLIPort = isDevelopment ? 4863 : 4862
    static let cliCommand = isDevelopment
        ? "lorca serve --home ~/.lorca-dev --port 4863"
        : "lorca serve"
}
