// swift-tools-version: 5.9
import PackageDescription

// The Mac unit consumes generated UniFFI Swift and the embedded Rust library.
let package = Package(
    name: "VeluneMac",
    platforms: [.macOS("15.0")],
    dependencies: [
        .package(url: "https://github.com/swiftlang/swift-markdown.git", exact: "0.9.0")
    ],
    targets: [
        .systemLibrary(name: "VeluneBindingsFFI", path: "target/swift-ffi"),
        .target(name: "VeluneBindings", dependencies: ["VeluneBindingsFFI"], path: "target/swift-bindings", swiftSettings: [.unsafeFlags(["-warnings-as-errors"])]),
        .executableTarget(name: "VeluneMac", dependencies: ["VeluneBindings", .product(name: "Markdown", package: "swift-markdown")], path: "app/mac", exclude: ["README.md", "Assets", "Licenses"], swiftSettings: [.unsafeFlags(["-parse-as-library", "-warnings-as-errors"])])
    ]
)
