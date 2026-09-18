// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "Tinybot",
    platforms: [.macOS(.v14)],
    targets: [
        // The Markdown parser from crates/markdown: the Rust static library and its C header,
        // built into Libraries/ by scripts/app.ts.
        .binaryTarget(
            name: "TinybotMarkdownFFI",
            path: "Libraries/TinybotMarkdownFFI.xcframework"
        ),
        // The UniFFI Swift bindings over it, generated into Sources/TinybotMarkdown by the same step.
        .target(
            name: "TinybotMarkdown",
            dependencies: ["TinybotMarkdownFFI"],
            path: "Sources/TinybotMarkdown",
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
        .executableTarget(
            name: "Tinybot",
            dependencies: ["TinybotMarkdown"],
            path: "Sources/Tinybot",
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
    ]
)
