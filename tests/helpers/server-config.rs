pub fn config(root: &std::path::Path) -> super::Config {
    super::Config {
        state: root.join("state"),
        cache: root.join("cache"),
        web: root.join("web"),
        media: root.join("media"),
        bind: "127.0.0.1:0".parse().unwrap(),
        public_url: None,
        trusted_proxies: vec![],
        cors_origins: vec![],
        controller_socket: root.join("socket"),
    }
}
