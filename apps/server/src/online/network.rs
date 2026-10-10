//! YouTube signs media addresses for the IP address that extracted them, and
//! it may block one address family while serving the other. Each attempt pins
//! one family from extraction through every later media connection.
use crate::{
    AppState,
    error::{ApiError, Result},
};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    future::Future,
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket},
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Family {
    Ipv4,
    Ipv6,
}
impl Family {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ipv4 => "ipv4",
            Self::Ipv6 => "ipv6",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Ipv4 => "IPv4",
            Self::Ipv6 => "IPv6",
        }
    }
    fn other(self) -> Self {
        match self {
            Self::Ipv4 => Self::Ipv6,
            Self::Ipv6 => Self::Ipv4,
        }
    }
    /// The yt-dlp option selecting this family.
    pub fn flag(self) -> &'static str {
        match self {
            Self::Ipv4 => "--force-ipv4",
            Self::Ipv6 => "--force-ipv6",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Mode {
    #[default]
    Auto,
    Ipv4,
    Ipv6,
}
impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Ipv4 => "ipv4",
            Self::Ipv6 => "ipv6",
        }
    }
    pub fn parse(value: &str) -> Self {
        match value {
            "ipv4" => Self::Ipv4,
            "ipv6" => Self::Ipv6,
            _ => Self::Auto,
        }
    }
}

#[derive(Default)]
pub(crate) struct Runtime {
    /// Automatic selection starts with IPv4 and keeps the family that last worked.
    ipv6_first: AtomicBool,
    pub(super) egress: super::egress::Egress,
}

/// Connecting a UDP socket only selects a route; it sends no packets.
fn reachable(family: Family) -> bool {
    let (local, remote): (SocketAddr, SocketAddr) = match family {
        Family::Ipv4 => (
            (Ipv4Addr::UNSPECIFIED, 0).into(),
            (Ipv4Addr::new(8, 8, 8, 8), 53).into(),
        ),
        Family::Ipv6 => (
            (Ipv6Addr::UNSPECIFIED, 0).into(),
            (
                Ipv6Addr::new(0x2001, 0x4860, 0x4860, 0, 0, 0, 0, 0x8888),
                53,
            )
                .into(),
        ),
    };
    UdpSocket::bind(local)
        .and_then(|socket| socket.connect(remote))
        .is_ok()
}

/// The families to attempt in order, or the configured family without a route.
fn order(
    mode: Mode,
    ipv6_first: bool,
    reachable: impl Fn(Family) -> bool,
) -> std::result::Result<Vec<Family>, Family> {
    let fixed = match mode {
        Mode::Ipv4 => Family::Ipv4,
        Mode::Ipv6 => Family::Ipv6,
        Mode::Auto => {
            let first = if ipv6_first {
                Family::Ipv6
            } else {
                Family::Ipv4
            };
            let families: Vec<_> = [first, first.other()]
                .into_iter()
                .filter(|f| reachable(*f))
                .collect();
            // Without any route, report the ordinary connection failure.
            return Ok(if families.is_empty() {
                vec![Family::Ipv4]
            } else {
                families
            });
        }
    };
    if reachable(fixed) {
        Ok(vec![fixed])
    } else {
        Err(fixed)
    }
}

pub(crate) async fn families(state: &AppState) -> Result<Vec<Family>> {
    let mode = super::storage::configuration(&state.db).await?.2;
    order(
        mode,
        state.online.network.ipv6_first.load(Ordering::Relaxed),
        reachable,
    )
    .map_err(|family| {
        ApiError(
            StatusCode::CONFLICT,
            "network_unavailable",
            format!(
                "{} is not available to this server; choose another YouTube network in settings",
                family.label()
            ),
        )
    })
}

pub(crate) fn worked(state: &AppState, family: Family) {
    state
        .online
        .network
        .ipv6_first
        .store(family == Family::Ipv6, Ordering::Relaxed);
}

/// Runs one attempt per family. Only YouTube's network block moves on to the
/// next family; other failures would repeat, so they are returned at once.
pub(crate) async fn attempt<T, F, Fut>(state: &AppState, mut run: F) -> Result<(T, Family)>
where
    F: FnMut(Family) -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let mut blocked = None;
    for family in families(state).await? {
        match run(family).await {
            Ok(value) => {
                worked(state, family);
                return Ok((value, family));
            }
            Err(error) if error.1 == "network_blocked" => blocked = Some(error),
            // A routed family can still lack internet access; keep the block
            // unless the fallback reached a verdict about the video itself.
            Err(error)
                if blocked.is_some()
                    && !matches!(error.1, "unavailable" | "extractor_authentication_required") =>
            {
                break;
            }
            Err(error) => return Err(error),
        }
    }
    Err(blocked.expect("at least one family is attempted"))
}

pub(crate) fn status(state: &AppState, mode: Mode) -> Value {
    let current = order(
        mode,
        state.online.network.ipv6_first.load(Ordering::Relaxed),
        reachable,
    )
    .ok()
    .map(|families| families[0].as_str());
    json!({"ipv4":reachable(Family::Ipv4),"ipv6":reachable(Family::Ipv6),"current":current})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_selection_falls_back_only_to_routed_families() {
        let both = |_| true;
        let ipv4 = |f| f == Family::Ipv4;
        assert_eq!(
            order(Mode::Auto, false, both),
            Ok(vec![Family::Ipv4, Family::Ipv6])
        );
        assert_eq!(
            order(Mode::Auto, true, both),
            Ok(vec![Family::Ipv6, Family::Ipv4])
        );
        assert_eq!(order(Mode::Auto, true, ipv4), Ok(vec![Family::Ipv4]));
        assert_eq!(order(Mode::Auto, false, |_| false), Ok(vec![Family::Ipv4]));
    }

    #[test]
    fn a_configured_family_never_falls_back() {
        let ipv4 = |f| f == Family::Ipv4;
        assert_eq!(order(Mode::Ipv4, true, |_| true), Ok(vec![Family::Ipv4]));
        assert_eq!(order(Mode::Ipv6, false, |_| true), Ok(vec![Family::Ipv6]));
        assert_eq!(order(Mode::Ipv6, false, ipv4), Err(Family::Ipv6));
        assert_eq!(Mode::parse("ipv6"), Mode::Ipv6);
        assert_eq!(Mode::parse("unknown"), Mode::Auto);
    }
}
