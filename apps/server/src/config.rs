use anyhow::{Result, bail};
use ipnet::IpNet;
use std::{net::SocketAddr, path::PathBuf};

#[derive(Clone)]
pub struct Config {
    pub state: PathBuf,
    pub cache: PathBuf,
    pub web: PathBuf,
    pub media: PathBuf,
    pub bind: SocketAddr,
    pub public_url: Option<url::Url>,
    pub trusted_proxies: Vec<IpNet>,
    pub cors_origins: Vec<String>,
    pub controller_socket: PathBuf,
}
impl Config {
    pub fn from_env() -> Result<Self> {
        let public_url = std::env::var("THELXINOE_PUBLIC_URL")
            .ok()
            .filter(|s| !s.is_empty())
            .map(|s| url::Url::parse(&s))
            .transpose()?;
        if let Some(url) = &public_url
            && (!matches!(url.scheme(), "http" | "https")
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.path() != "/"
                || url.query().is_some()
                || url.fragment().is_some())
        {
            bail!("Public URL must be an HTTP(S) origin without a path or credentials");
        }
        let trusted_proxies = std::env::var("THELXINOE_TRUSTED_PROXIES")
            .unwrap_or_default()
            .split(',')
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim().parse())
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(Self {
            state: env_path("THELXINOE_STATE", ".local/server"),
            cache: env_path("THELXINOE_CACHE", ".local/cache"),
            web: env_path("THELXINOE_WEB", "frontend/dist"),
            media: env_path("THELXINOE_MEDIA", ".local/media"),
            bind: std::env::var("THELXINOE_BIND")
                .unwrap_or("127.0.0.1:8484".into())
                .parse()?,
            public_url,
            trusted_proxies,
            cors_origins: std::env::var("THELXINOE_CORS_ORIGINS")
                .unwrap_or_default()
                .split(',')
                .filter(|s| !s.trim().is_empty())
                .map(|s| {
                    let value = s.trim();
                    let url = url::Url::parse(value)?;
                    anyhow::ensure!(
                        matches!(url.scheme(), "http" | "https")
                            && url.host_str().is_some()
                            && url.username().is_empty()
                            && url.password().is_none()
                            && url.path() == "/"
                            && url.query().is_none()
                            && url.fragment().is_none(),
                        "CORS origins must be explicit HTTP(S) origins"
                    );
                    Ok(url.origin().ascii_serialization())
                })
                .collect::<Result<Vec<_>>>()?,
            controller_socket: env_path(
                "THELXINOE_CONTROLLER_SOCKET",
                "/run/thelxinoe/controller.sock",
            ),
        })
    }
}
fn env_path(name: &str, default: &str) -> PathBuf {
    std::env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| default.into())
}
