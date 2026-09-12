import CoreBluetooth

enum Authorization: Equatable {
  case notDetermined
  case restricted
  case denied
  case allowedAlways
  case unknown
}

/// Reads the app's Bluetooth authorization without creating a manager, which would show the prompt.
func currentAuthorization() -> Authorization {
  guard #available(iOS 13.1, macOS 10.15, *) else { return .unknown }
  switch CBManager.authorization {
  case .notDetermined:
    return .notDetermined
  case .restricted:
    return .restricted
  case .denied:
    return .denied
  case .allowedAlways:
    return .allowedAlways
  @unknown default:
    return .unknown
  }
}

func adapterStateName(_ state: CBManagerState) -> String {
  switch state {
  case .poweredOn:
    return "poweredOn"
  case .poweredOff:
    return "poweredOff"
  case .resetting:
    return "resetting"
  case .unauthorized:
    return "unauthorized"
  case .unsupported:
    return "unavailable"
  case .unknown:
    return "unknown"
  @unknown default:
    return "unknown"
  }
}

func adapterStateWithoutManager(_ authorization: Authorization) -> String {
  switch authorization {
  case .denied, .restricted:
    return "unauthorized"
  default:
    return "unknown"
  }
}

func permissionOutcome(_ authorization: Authorization) -> String {
  switch authorization {
  case .allowedAlways:
    return "granted"
  case .notDetermined:
    return "promptable"
  case .denied:
    return "deniedPermanently"
  case .restricted:
    return "restricted"
  case .unknown:
    return "unknown"
  }
}
