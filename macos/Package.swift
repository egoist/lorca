// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "Lorca",
    platforms: [.macOS(.v14)],
    targets: [
        // The Markdown parser from crates/markdown: the Rust static library and its C header,
        // built into Libraries/ by scripts/app.ts.
        .binaryTarget(
            name: "LorcaMarkdownFFI",
            path: "Libraries/LorcaMarkdownFFI.xcframework"
        ),
        // The UniFFI Swift bindings over it, generated into Sources/LorcaMarkdown by the same step.
        .target(
            name: "LorcaMarkdown",
            dependencies: ["LorcaMarkdownFFI"],
            path: "Sources/LorcaMarkdown",
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
        .executableTarget(
            name: "Lorca",
            dependencies: ["LorcaMarkdown"],
            path: "Sources/Lorca",
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
    ]
)
