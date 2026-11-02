// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "MultiFinder",
    platforms: [.macOS(.v14)],
    products: [
        .library(name: "MultiFinderCore", targets: ["MultiFinderCore"]),
        .executable(name: "mfinder", targets: ["MultiFinderCLI"]),
        .executable(name: "MultiFinderApp", targets: ["MultiFinderApp"]),
    ],
    targets: [
        .target(name: "MultiFinderCore"),
        .executableTarget(name: "MultiFinderCLI", dependencies: ["MultiFinderCore"]),
        .executableTarget(name: "MultiFinderApp", dependencies: ["MultiFinderCore"]),
        .testTarget(name: "MultiFinderCoreTests", dependencies: ["MultiFinderCore"]),
    ]
)
