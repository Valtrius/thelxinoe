use crate::docker::Result;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum RollbackPhase {
    Copying,
    Activating,
}

pub(crate) enum RollbackAction {
    CopyRecovery,
    RestartOriginalOnly,
}

pub(crate) fn validate(phase: Option<RollbackPhase>, stage: &str) -> Result<()> {
    let valid = match (phase, stage) {
        (Some(RollbackPhase::Copying), "rollback-copying")
        | (Some(RollbackPhase::Activating), "rollback-activating") => true,
        (_, "rollback-copying" | "rollback-activating") => false,
        (Some(RollbackPhase::Activating), stage) => matches!(
            stage,
            "recovery-required" | "restore-failed" | "rolled-back" | "failed"
        ),
        _ => true,
    };
    if !valid {
        return Err((
            axum::http::StatusCode::CONFLICT,
            "Recovery journal phase and payload disagree; explicit recovery is required",
        ));
    }
    Ok(())
}

pub(crate) fn plan(phase: Option<RollbackPhase>, recovery_ready: bool) -> Result<RollbackAction> {
    match (phase, recovery_ready) {
        (Some(RollbackPhase::Activating), _) | (_, false) => {
            Ok(RollbackAction::RestartOriginalOnly)
        }
        (Some(RollbackPhase::Copying), true) => Ok(RollbackAction::CopyRecovery),
        (None, true) => Err((
            axum::http::StatusCode::CONFLICT,
            "Recovery journal cannot prove whether original services resumed; explicit recovery is required",
        )),
    }
}

pub(crate) fn spawn<F>(future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    #[cfg(test)]
    if let Ok(context) = crate::test_support::DOCKER.try_with(Clone::clone) {
        return tokio::spawn(crate::test_support::DOCKER.scope(context, future));
    }
    tokio::spawn(future)
}
