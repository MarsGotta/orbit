// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "OrbitNative",
    platforms: [.macOS("15.0")],
    products: [
        .library(name: "OrbitNativeHost", targets: ["OrbitNativeHost"]),
        .executable(name: "OrbitNative", targets: ["OrbitNative"]),
    ],
    targets: [
        .target(name: "OrbitNativeProtocol"),
        .target(name: "OrbitNativeState", dependencies: ["OrbitNativeProtocol"]),
        .target(name: "OrbitNativeCalendar", dependencies: ["OrbitNativeProtocol"],
                linkerSettings: [.linkedFramework("EventKit")]),
        .target(name: "OrbitNativeTransport", dependencies: ["OrbitNativeProtocol"],
                swiftSettings: [.swiftLanguageMode(.v6)]),
        .target(name: "OrbitAudioRing", publicHeadersPath: "include"),
        .target(
            name: "OrbitNativeHost",
            dependencies: [
                "OrbitNativeProtocol", "OrbitNativeState", "OrbitNativeTransport",
                "OrbitNativeCalendar", "OrbitAudioRing",
            ],
            linkerSettings: [
                .linkedFramework("AppKit"), .linkedFramework("AVFoundation"),
                .linkedFramework("ScreenCaptureKit"), .linkedFramework("Carbon"),
                .linkedFramework("Speech"),
                .linkedFramework("Security"), .linkedFramework("ServiceManagement"),
                .linkedFramework("PDFKit"), .linkedFramework("ImageIO"),
                .linkedFramework("UniformTypeIdentifiers")
            ]
        ),
        .executableTarget(
            name: "OrbitNative",
            dependencies: [
                "OrbitNativeHost", "OrbitNativeProtocol", "OrbitNativeState",
                "OrbitNativeTransport", "OrbitNativeCalendar",
            ],
            linkerSettings: [.linkedFramework("AppKit")]
        ),
        .testTarget(name: "OrbitNativeProtocolTests", dependencies: [
            "OrbitNativeHost", "OrbitNativeProtocol", "OrbitNativeTransport", "OrbitAudioRing",
        ]),
        .testTarget(name: "OrbitNativeTransportTests", dependencies: ["OrbitNativeTransport", "OrbitNativeProtocol"]),
        .testTarget(name: "OrbitNativeStateTests", dependencies: ["OrbitNativeState"]),
        .testTarget(name: "OrbitNativeCalendarTests", dependencies: ["OrbitNativeCalendar", "OrbitNativeProtocol"])
    ],
    swiftLanguageModes: [.v5]
)
