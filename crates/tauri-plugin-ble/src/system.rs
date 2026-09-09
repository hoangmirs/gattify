use async_trait::async_trait;
use ble_core::{
    AdapterState, Backend, BleError, BleResult, Capabilities, Command, OperationContext,
    PermissionOutcome, PermissionState, Reply, Support,
};

/// Production backend gate.
///
/// Native GATT adapters replace this type platform by platform. Until a backend
/// is wired and verified, capability queries are honest and radio operations
/// fail explicitly.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemBackend;

#[async_trait]
impl Backend for SystemBackend {
    async fn execute(&self, _context: OperationContext, command: Command) -> BleResult<Reply> {
        match command {
            Command::GetState => Ok(Reply::State(AdapterState::Unknown)),
            Command::GetCapabilities => Ok(Reply::Capabilities(unverified_capabilities())),
            Command::CheckPermissions => Ok(Reply::Permissions(PermissionState {
                scan: PermissionOutcome::Unknown,
                connect: PermissionOutcome::Unknown,
                advertise: PermissionOutcome::Unknown,
            })),
            Command::CloseOwner | Command::Cancel { .. } => Ok(Reply::Empty),
            #[cfg(not(feature = "peer"))]
            _ => Err(BleError::unsupported(
                "native backend is not implemented; peer feature is disabled",
            )),
            #[cfg(feature = "peer")]
            _ => Err(BleError::unsupported(
                "native backend is not implemented for this target build",
            )),
        }
    }
}

fn unverified_capabilities() -> Capabilities {
    let platform = std::env::consts::OS;
    let unknown =
        |capability: &str| Support::unknown(format!("{platform}:{capability}:backendNotVerified"));
    Capabilities {
        central: unknown("central"),
        peripheral: unknown("peripheral"),
        advertising: unknown("advertising"),
        targeted_notify: unknown("targetedNotify"),
        simultaneous_roles: unknown("simultaneousRoles"),
        background: Support::unsupported("foregroundOnlyContract"),
        max_connections: None,
        max_advertising_data_length: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_backend_never_reports_supported() {
        let capabilities = unverified_capabilities();
        assert!(matches!(
            capabilities.central.level,
            ble_core::SupportLevel::Unknown
        ));
        assert!(matches!(
            capabilities.targeted_notify.level,
            ble_core::SupportLevel::Unknown
        ));
    }
}
