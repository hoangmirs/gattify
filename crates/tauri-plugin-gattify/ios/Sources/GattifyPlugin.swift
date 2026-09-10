import CoreBluetooth
import Tauri
import WebKit

final class GattifyPlugin: Plugin {
  private let queue = DispatchQueue(label: "dev.gattify.plugin.corebluetooth")
  private var central: CBCentralManager?
  private var peripheral: CBPeripheralManager?

  @objc override public func load(webview: WKWebView) {
    central = CBCentralManager(delegate: nil, queue: queue)
    peripheral = CBPeripheralManager(delegate: nil, queue: queue)
  }

  @objc public func getState(_ invoke: Invoke) {
    let value: String
    switch central?.state ?? .unknown {
    case .poweredOn: value = "poweredOn"
    case .poweredOff: value = "poweredOff"
    case .unauthorized: value = "unauthorized"
    case .unsupported: value = "unavailable"
    case .resetting: value = "resetting"
    default: value = "unknown"
    }
    invoke.resolve(["state": value])
  }

  @objc public func getCapabilities(_ invoke: Invoke) {
    invoke.resolve([
      "central": support("unknown", "requiresPoweredOnRuntimeProbe"),
      "peripheral": support("unknown", "requiresPoweredOnRuntimeProbe"),
      "advertising": support("unknown", "requiresAdvertisementCallback"),
      "targetedNotify": support("unknown", "requiresSubscriberIsolationQualification"),
      "simultaneousRoles": support("unknown", "requiresDeviceQualification"),
      "background": support("unsupported", "foregroundOnlyContract"),
    ])
  }

  private func support(_ level: String, _ reason: String) -> [String: Any] {
    ["level": level, "reason": reason]
  }
}

@_cdecl("init_plugin_gattify")
public func initPlugin() -> Plugin {
  GattifyPlugin()
}

