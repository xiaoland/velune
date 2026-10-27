// swift-tools-version: 5.9
import PackageDescription

// The Mac unit consumes generated UniFFI Swift and the embedded Rust library.
let package = Package(
    name: "VeluneMac",
    platforms: [.macOS("15.0")],
    dependencies: [
        .package(url: "https://github.com/swiftlang/swift-markdown.git", exact: "0.9.0"),
        .package(url: "https://github.com/open-telemetry/opentelemetry-swift.git", exact: "2.6.0"),
        .package(url: "https://github.com/open-telemetry/opentelemetry-swift-core.git", exact: "2.6.0"),
        .package(url: "https://github.com/grpc/grpc-swift.git", exact: "1.27.6"),
        .package(url: "https://github.com/apple/swift-nio.git", exact: "2.101.3"),
        .package(url: "https://github.com/apple/swift-nio-transport-services.git", exact: "1.28.0")
    ],
    targets: [
        .systemLibrary(name: "VeluneBindingsFFI", path: "target/swift-ffi"),
        .target(name: "VeluneBindings", dependencies: ["VeluneBindingsFFI"], path: "target/swift-bindings", swiftSettings: [.unsafeFlags(["-warnings-as-errors"])]),
        .executableTarget(name: "VeluneMac", dependencies: ["VeluneBindings", .product(name: "Markdown", package: "swift-markdown"), .product(name: "OpenTelemetryProtocolExporter", package: "opentelemetry-swift"), .product(name: "OpenTelemetryApi", package: "opentelemetry-swift-core"), .product(name: "OpenTelemetrySdk", package: "opentelemetry-swift-core"), .product(name: "GRPC", package: "grpc-swift"), .product(name: "NIO", package: "swift-nio"), .product(name: "NIOTransportServices", package: "swift-nio-transport-services")], path: "app/mac", exclude: ["README.md", "Assets", "Licenses"], swiftSettings: [.unsafeFlags(["-parse-as-library", "-warnings-as-errors"])])
    ]
)
