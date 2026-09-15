// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "Tinybot",
    platforms: [.macOS(.v14)],
    targets: [
        .executableTarget(
            name: "Tinybot",
            path: "Sources/Tinybot",
            swiftSettings: [.swiftLanguageMode(.v5)]
        )
    ]
)
