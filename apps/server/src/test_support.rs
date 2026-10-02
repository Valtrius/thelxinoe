use crate::{AppState, config::Config};
use axum::{
    body::Body,
    http::{HeaderMap, Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;
#[path = "../../../tests/helpers/auth-session.rs"]
mod auth_session;
#[path = "../../../tests/helpers/server-config.rs"]
mod configuration;
pub(crate) use auth_session::issue_session;

#[derive(Default)]
pub(crate) struct FaultControl {
    pub calls: std::sync::atomic::AtomicUsize,
    pub pause: tokio::sync::Mutex<
        Option<(
            tokio::sync::oneshot::Sender<()>,
            tokio::sync::oneshot::Receiver<()>,
        )>,
    >,
}
impl FaultControl {
    pub async fn reached(&self) {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let Some((entered, resume)) = self.pause.lock().await.take() {
            let _ = entered.send(());
            let _ = resume.await;
        }
    }
}
tokio::task_local! { pub(crate) static MEDIA_VALIDATION: std::sync::Arc<FaultControl>; }
tokio::task_local! { pub(crate) static CONTROLLER_FAILURE: (&'static str, &'static str); }

pub(crate) async fn call(
    state: &AppState,
    path: &str,
    method: &str,
    body: Value,
    cookie: &str,
) -> (StatusCode, HeaderMap, Value) {
    let mut request = Request::builder()
        .uri(path)
        .method(method)
        .header("host", "internal:8484")
        .header("x-forwarded-proto", "https")
        .header("x-forwarded-host", "media.test")
        .header("x-thelxinoe-client", "1")
        .header("cookie", cookie)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    request.extensions_mut().insert(axum::extract::ConnectInfo(
        "127.0.0.1:12345".parse::<std::net::SocketAddr>().unwrap(),
    ));
    let response = crate::router(state.clone()).oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        headers,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
pub(crate) async fn fixture() -> (tempfile::TempDir, AppState, String) {
    let temp = tempfile::tempdir().unwrap();
    let mut config = configuration::config(temp.path());
    config.public_url = Some("https://media.test".parse().unwrap());
    config.trusted_proxies = vec!["127.0.0.1/32".parse().unwrap()];
    let state = AppState::open(config).await.unwrap();
    state
        .db
        .write("test.fixture", |db| {
            for name in ["alice", "bob"] {
                db.execute(
                    "INSERT INTO users(id,username,password_hash,role,created_at) VALUES (?1,?1,'unused','user',1)",
                    [name],
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();
    let token = crate::test_support::issue_session(
        &state.db,
        "alice".into(),
        "web".into(),
        "test browser".into(),
    )
    .await
    .unwrap();
    (temp, state, format!("thelxinoe_session={token}"))
}
