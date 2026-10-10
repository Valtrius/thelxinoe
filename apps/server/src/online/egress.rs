//! Loopback CONNECT proxies keep YouTube media connections on the address
//! family that extracted their signed addresses, including the HLS segments
//! ffmpeg opens itself. Only HTTPS to YouTube's media CDN may pass.
use super::network::Family;
use anyhow::{Result, ensure};
use std::{net::Ipv4Addr, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{OnceCell, Semaphore},
};

#[derive(Default)]
pub(crate) struct Egress {
    proxies: [OnceCell<String>; 2],
}
impl Egress {
    pub async fn proxy(&self, family: Family) -> Result<String> {
        self.proxies[family as usize]
            .get_or_try_init(|| async {
                let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
                let address = format!("http://{}", listener.local_addr()?);
                tokio::spawn(serve(listener, family));
                Ok(address)
            })
            .await
            .cloned()
    }
}

async fn serve(listener: TcpListener, family: Family) {
    let slots = Arc::new(Semaphore::new(64));
    loop {
        match listener.accept().await {
            Ok((client, _)) => {
                let slots = slots.clone();
                tokio::spawn(async move {
                    let Ok(_slot) = slots.acquire_owned().await else {
                        return;
                    };
                    let _ = tunnel(client, family).await;
                });
            }
            // Wait out descriptor exhaustion instead of spinning.
            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
}

fn target(head: &[u8]) -> Option<String> {
    let line = std::str::from_utf8(head).ok()?.split("\r\n").next()?;
    let mut parts = line.split(' ');
    let (Some("CONNECT"), Some(authority), Some("HTTP/1.1" | "HTTP/1.0"), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    let host = authority.strip_suffix(":443")?.to_ascii_lowercase();
    (host
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
        && (host == "googlevideo.com" || host.ends_with(".googlevideo.com")))
    .then_some(host)
}

async fn tunnel(mut client: TcpStream, family: Family) -> Result<()> {
    let mut head = Vec::new();
    let mut buffer = [0; 1024];
    // Clients wait for the tunnel before sending TLS, so the head ends a read.
    tokio::time::timeout(Duration::from_secs(10), async {
        while !head.ends_with(b"\r\n\r\n") {
            let read = client.read(&mut buffer).await?;
            ensure!(
                read > 0 && head.len() + read <= 8192,
                "Invalid proxy request"
            );
            head.extend_from_slice(&buffer[..read]);
        }
        Ok(())
    })
    .await??;
    let Some(host) = target(&head) else {
        client
            .write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await?;
        return Ok(());
    };
    let Some(mut upstream) = connect(&host, family).await else {
        client
            .write_all(
                b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .await?;
        return Ok(());
    };
    client
        .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
        .await?;
    tokio::io::copy_bidirectional(&mut client, &mut upstream).await?;
    Ok(())
}

async fn connect(host: &str, family: Family) -> Option<TcpStream> {
    let addresses = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::net::lookup_host((host, 443)),
    )
    .await
    .ok()?
    .ok()?;
    for address in addresses.filter(|a| a.is_ipv6() == (family == Family::Ipv6)) {
        if let Ok(Ok(stream)) =
            tokio::time::timeout(Duration::from_secs(10), TcpStream::connect(address)).await
        {
            return Some(stream);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_tunnels_to_the_media_cdn_are_accepted() {
        assert_eq!(
            target(b"CONNECT rr1---sn-abc.googlevideo.com:443 HTTP/1.1\r\nHost: x\r\n\r\n"),
            Some("rr1---sn-abc.googlevideo.com".into())
        );
        assert_eq!(
            target(b"CONNECT Manifest.GoogleVideo.com:443 HTTP/1.0\r\n\r\n"),
            Some("manifest.googlevideo.com".into())
        );
        for head in [
            &b"GET http://r1.googlevideo.com/ HTTP/1.1\r\n\r\n"[..],
            b"CONNECT r1.googlevideo.com:80 HTTP/1.1\r\n\r\n",
            b"CONNECT r1.googlevideo.com HTTP/1.1\r\n\r\n",
            b"CONNECT googlevideo.com.example.test:443 HTTP/1.1\r\n\r\n",
            b"CONNECT evilgooglevideo.com:443 HTTP/1.1\r\n\r\n",
            b"CONNECT 127.0.0.1:443 HTTP/1.1\r\n\r\n",
            b"CONNECT r1.googlevideo.com:443 HTTP/1.1 extra\r\n\r\n",
            b"CONNECT r1.google video.com:443 HTTP/1.1\r\n\r\n",
        ] {
            assert_eq!(target(head), None, "{}", String::from_utf8_lossy(head));
        }
    }

    #[tokio::test]
    async fn the_proxy_listens_on_loopback_and_refuses_other_hosts() {
        let egress = Egress::default();
        let proxy = egress.proxy(Family::Ipv4).await.unwrap();
        assert!(proxy.starts_with("http://127.0.0.1:"));
        assert_eq!(egress.proxy(Family::Ipv4).await.unwrap(), proxy);
        assert_ne!(egress.proxy(Family::Ipv6).await.unwrap(), proxy);
        let mut client = TcpStream::connect(proxy.trim_start_matches("http://"))
            .await
            .unwrap();
        client
            .write_all(b"CONNECT example.com:443 HTTP/1.1\r\nHost: example.com:443\r\n\r\n")
            .await
            .unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).await.unwrap();
        assert!(response.starts_with("HTTP/1.1 403 "), "{response}");
    }
}
