// swift-tools-version:5.9
import PackageDescription

let package = Package(
  name: "tauri-plugin-gattify",
  platforms: [
    .macOS(.v10_15),
    .iOS(.v15),
  ],
  products: [
    .library(
      name: "tauri-plugin-gattify",
      type: .static,
      targets: ["tauri-plugin-gattify"])
  ],
  dependencies: [
    .package(name: "Tauri", path: "../.tauri/tauri-api")
  ],
  targets: [
    .target(
      name: "tauri-plugin-gattify",
      dependencies: [
        .byName(name: "Tauri")
      ],
      path: "Sources"),
    .testTarget(
      name: "tauri-plugin-gattify-tests",
      dependencies: [
        "tauri-plugin-gattify",
        .byName(name: "Tauri"),
      ],
      path: "Tests")
  ]
)
