// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "HiFi",
    platforms: [.macOS(.v14)],
    products: [
        .executable(name: "HiFiApp", targets: ["HiFiApp"]),
        .executable(name: "hifi", targets: ["hifi"])
    ],
    dependencies: [
        .package(url: "https://github.com/migueldeicaza/SwiftTerm", from: "1.2.0")
    ],
    targets: [
        // Engine-agnostic domain: models, IPC protocol, unix socket, TOML.
        // MUST NOT import WebKit.
        .target(name: "HiFiCore", dependencies: []),
        // macOS chrome + WebKit host.
        .executableTarget(
            name: "HiFiApp",
            dependencies: [
                "HiFiCore",
                .product(name: "SwiftTerm", package: "SwiftTerm")
            ],
            path: "Sources/HiFiApp"
        ),
        // `hifi` command line client.
        .executableTarget(
            name: "hifi",
            dependencies: ["HiFiCore"],
            path: "Sources/hifi"
        )
    ]
)
