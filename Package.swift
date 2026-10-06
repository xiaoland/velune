// swift-tools-version: 5.9
import PackageDescription

// The Mac unit consumes generated UniFFI Swift and the embedded Rust library.
let package = Package(
    name: "VeluneMac",
    platforms: [.macOS(.v14)],
    dependencies: [
        .package(url: "https://github.com/gonzalezreal/swift-markdown-ui", exact: "2.4.1")
    ],
    targets: [
        .systemLibrary(name: "VeluneBindingsFFI", path: "target/swift-ffi"),
        .target(name: "VeluneBindings", dependencies: ["VeluneBindingsFFI"], path: "target/swift-bindings"),
        .executableTarget(name: "VeluneMac", dependencies: ["VeluneBindings", .product(name: "MarkdownUI", package: "swift-markdown-ui")], path: "app/mac", exclude: ["README.md", "Assets", "Licenses"], swiftSettings: [.unsafeFlags(["-parse-as-library"])])
    ]
)
