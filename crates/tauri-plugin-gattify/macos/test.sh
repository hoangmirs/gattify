#!/bin/sh
# Runs the Swift engine tests of ios/Tests on macOS. build.rs links the same engine sources
# into macOS apps; the iOS package cannot build for macOS because it depends on Tauri's iOS API.
set -eu

crate=$(cd "$(dirname "$0")/.." && pwd)
package="$crate/../../target/gattify-macos-swift-tests"
rm -rf "$package"
mkdir -p "$package/Sources/Engine" "$package/Tests/EngineTests"

for source in "$crate"/ios/Sources/*.swift; do
  [ "$(basename "$source")" = GattifyPlugin.swift ] || cp "$source" "$package/Sources/Engine/"
done
cp "$crate"/ios/Tests/*.swift "$package/Tests/EngineTests/"

cat >"$package/Package.swift" <<'EOF'
// swift-tools-version:5.9
import PackageDescription

let package = Package(
  name: "gattify-macos-swift-tests",
  platforms: [.macOS(.v10_15)],
  targets: [
    .target(name: "tauri_plugin_gattify", path: "Sources/Engine"),
    .testTarget(name: "EngineTests", dependencies: ["tauri_plugin_gattify"], path: "Tests/EngineTests"),
  ]
)
EOF

swift test --package-path "$package"
