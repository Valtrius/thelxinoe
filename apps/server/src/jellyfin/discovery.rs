//! A container cannot infer its externally reachable LAN origin.
use crate::AppState;
use anyhow::{Result, bail};
use serde_json::json;
use std::{
    collections::HashMap,
    net::IpAddr,
    time::{Duration, Instant},
};
use tokio::net::UdpSocket;

fn origin(value: &str) -> Result<String> {
    let url = url::Url::parse(value)?;
    if value.len() > 512
        || !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!("Discovery URL must be an HTTP(S) origin without a path or credentials");
    }
    Ok(url.origin().ascii_serialization())
}
pub async fn run(state: AppState) -> Result<()> {
    let enabled = std::env::var("THELXINOE_DISCOVERY").unwrap_or_else(|_| "true".into());
    if !["true", "false"].contains(&enabled.as_str()) {
        bail!("THELXINOE_DISCOVERY must be true or false");
    }
    let advertised = std::env::var("THELXINOE_DISCOVERY_URL")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| state.config.public_url.as_ref().map(ToString::to_string));
    if enabled == "false" || advertised.is_none() {
        return std::future::pending().await;
    }
    let address = origin(&advertised.unwrap())?;
    let bind = std::env::var("THELXINOE_DISCOVERY_BIND").unwrap_or_else(|_| "0.0.0.0:7359".into());
    let socket = UdpSocket::bind(bind).await?;
    let reply = serde_json::to_vec(
        &json!({"Address":address,"Id":state.server_id.as_str(),"Name":format!("Thelxinoe {}",thelxinoe_core::VERSION)}),
    )?;
    tracing::info!("LAN discovery enabled");
    serve(socket, reply).await
}
fn local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_private() || ip.is_link_local() || ip.is_loopback(),
        IpAddr::V6(ip) => ip
            .to_ipv4_mapped()
            .map(|ip| local(ip.into()))
            .unwrap_or_else(|| {
                ip.is_unique_local() || ip.is_unicast_link_local() || ip.is_loopback()
            }),
    }
}
async fn serve(socket: UdpSocket, reply: Vec<u8>) -> Result<()> {
    let mut buffer = [0; 128];
    let mut peers: HashMap<IpAddr, (Instant, u8)> = HashMap::new();
    let mut window = Instant::now();
    let mut total = 0;
    loop {
        let (size, peer) = socket.recv_from(&mut buffer).await?;
        if !local(peer.ip()) || !buffer[..size].eq_ignore_ascii_case(b"who is JellyfinServer?") {
            continue;
        }
        let now = Instant::now();
        if now.duration_since(window) >= Duration::from_secs(1) {
            window = now;
            total = 0;
        }
        if total >= 64 {
            continue;
        }
        peers.retain(|_, (start, _)| now.duration_since(*start) < Duration::from_secs(10));
        if peers.len() >= 1024 && !peers.contains_key(&peer.ip()) {
            continue;
        }
        let (_, count) = peers.entry(peer.ip()).or_insert((now, 0));
        if *count >= 4 {
            continue;
        }
        *count += 1;
        total += 1;
        let _ = socket.send_to(&reply, peer).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn discovery_replies_to_the_sdk_request_and_bounds_untrusted_input() {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let address = socket.local_addr().unwrap();
        let response = serde_json::to_vec(
            &json!({"Address":"http://localhost:8484","Id":"test","Name":"Thelxinoe"}),
        )
        .unwrap();
        let task = tokio::spawn(serve(socket, response.clone()));
        let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut buf = [0; 1024];
        client.send_to(b"unrelated packet", address).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(50), client.recv_from(&mut buf))
                .await
                .is_err()
        );
        for _ in 0..4 {
            client
                .send_to(b"who is JellyfinServer?", address)
                .await
                .unwrap();
            let (size, _) =
                tokio::time::timeout(Duration::from_secs(1), client.recv_from(&mut buf))
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(&buf[..size], response);
        }
        client
            .send_to(b"who is JellyfinServer?", address)
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(50), client.recv_from(&mut buf))
                .await
                .is_err()
        );
        task.abort();
        assert!(!local("8.8.8.8".parse().unwrap()));
        for invalid in [
            "http://user:secret@server",
            "https://server/path",
            "https://server?key=secret",
        ] {
            assert!(origin(invalid).is_err());
        }
    }
}
