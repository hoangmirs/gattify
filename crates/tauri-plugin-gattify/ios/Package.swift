// swift-tools-version: 5.9
import PackageDescription

let package = Package(
  name: "TauriPluginBle",
  platforms: [.iOS(.v15)],
  products: [
    .library(name: "TauriPluginBle", targets: ["TauriPluginBle"]),
  ],
  dependencies: [
    .package(url: "https://github.com/tauri-apps/tauri-swift", from: "2.0.0"),
  ],
  targets: [
    .target(
      name: "TauriPluginBle",
      dependencies: [.product(name: "Tauri", package: "tauri-swift")],
      path: "Sources"
    ),
  ]
)

