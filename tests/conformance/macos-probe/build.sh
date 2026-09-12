#!/bin/sh
# Builds the probe as an app bundle. macOS asks an app bundle for its own
# Bluetooth permission; a bare command-line tool inherits the permission of
# the app that started it, and macOS kills it without a usage description.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
out=${1:-"$here/build"}
bundle="$out/GattifyProbe.app"
mkdir -p "$bundle/Contents/MacOS"
swiftc -O -o "$bundle/Contents/MacOS/probe" "$here/main.swift"
cp "$here/Info.plist" "$bundle/Contents/Info.plist"
# An ad-hoc signature changes with every build, and macOS then asks for the
# Bluetooth permission again. A development identity keeps the permission.
codesign -s "${PROBE_SIGNING_IDENTITY:--}" -f --deep --identifier dev.gattify.probe "$bundle"
echo "$bundle"
