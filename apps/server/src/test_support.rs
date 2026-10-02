#[path = "../../../tests/helpers/auth-session.rs"]
mod auth_session;
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
