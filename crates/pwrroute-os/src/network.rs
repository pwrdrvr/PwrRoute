//! A recovery hint after a failed connection, never a preflight gate.

/// Whether macOS currently has a usable default network route. This reads
/// local configuration only; a true result does not promise Internet access.
/// Other platforms, and failed reads, return `None`: callers must still try
/// their real request under a bounded retry policy.
pub fn network_connected() -> Option<bool> {
    #[cfg(target_os = "macos")]
    {
        use std::net::{Ipv4Addr, SocketAddr};
        use system_configuration::network_reachability::{
            ReachabilityFlags, SCNetworkReachability,
        };

        let route = SCNetworkReachability::from(SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)));
        let flags = route.reachability().ok()?;
        Some(
            flags.contains(ReachabilityFlags::REACHABLE)
                && !flags.intersects(
                    ReachabilityFlags::CONNECTION_REQUIRED
                        | ReachabilityFlags::INTERVENTION_REQUIRED,
                ),
        )
    }
    #[cfg(not(target_os = "macos"))]
    None
}
