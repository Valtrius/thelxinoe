//! Connected services are validation-only. Normal work needs an operational Owned service.
//!
//! Every upstream mutation is authorized here immediately before it is sent. Reads are
//! always permitted; their callers decide which services they may observe.

#[path = "../storage/managers/ownership.rs"]
mod storage;

use super::*;

/// What a loaded service connection may change upstream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Scope {
    /// Connected, or loaded only to check health and identity.
    Validation,
    /// Owned, committed, set up and not held; rechecked with its generation before mutations.
    Operational,
    /// Fresh installation setup under its `setup` hold.
    Setup { provision: String },
    /// One reviewed setup action of a committed transfer, against its exact replacement.
    Transfer { operation: String, action: String },
}

/// Setup or transfer authority of the running coordinator for one service.
#[derive(Clone)]
pub(super) struct Authority {
    pub(super) service: String,
    pub(super) scope: Scope,
}

tokio::task_local! {
    static AUTHORITY: Authority;
}

/// Run coordinator work for one service under setup or transfer authority. Loaders give that
/// service the authority's scope; every mutation still rechecks it against durable state.
pub(super) async fn with_authority<F: std::future::Future>(
    authority: Authority,
    work: F,
) -> F::Output {
    AUTHORITY.scope(authority, work).await
}

/// The coordinator scope for `service`, when the current task holds one.
pub(super) fn authority_for(service: &str) -> Option<Scope> {
    AUTHORITY
        .try_with(|authority| (authority.service == service).then(|| authority.scope.clone()))
        .ok()
        .flatten()
}

/// The identity a connection was opened for.
#[derive(Clone)]
pub(super) struct Grant {
    pub(super) scope: Scope,
    pub(super) service: String,
    pub(super) generation: String,
    pub(super) container: String,
}
impl Grant {
    pub(super) fn validation() -> Self {
        Self {
            scope: Scope::Validation,
            service: String::new(),
            generation: String::new(),
            container: String::new(),
        }
    }
}

fn validation_only(kind: &str) -> ApiError {
    ApiError(
        axum::http::StatusCode::CONFLICT,
        "validation_only",
        format!(
            "{} is connected for read-only validation. Take ownership before Thelxinoe changes it",
            failures::label(kind)
        ),
    )
}

pub(super) fn held(kind: &str) -> ApiError {
    ApiError(
        axum::http::StatusCode::CONFLICT,
        "operational_hold",
        format!(
            "{} isn't available for normal operations while it is connected only, being set up, transferred or recovered",
            failures::label(kind)
        ),
    )
}

/// Allow one upstream mutation by a connection opened with `grant`.
pub(super) async fn authorize(state: &AppState, kind: &str, grant: &Grant) -> Result<()> {
    let allowed = match &grant.scope {
        Scope::Validation => return Err(validation_only(kind)),
        Scope::Operational => {
            storage::operational(&state.db, grant.service.clone(), grant.generation.clone()).await?
        }
        Scope::Setup { provision } => {
            storage::setup(
                &state.db,
                provision.clone(),
                grant.service.clone(),
                grant.container.clone(),
            )
            .await?
        }
        Scope::Transfer { operation, action } => {
            storage::transfer(
                &state.db,
                operation.clone(),
                action.clone(),
                grant.service.clone(),
                grant.container.clone(),
            )
            .await?
        }
    };
    if allowed { Ok(()) } else { Err(held(kind)) }
}

/// NZBGet methods that only read state. Every other method is a mutation.
pub(super) fn nzbget_read_only(method: &str) -> bool {
    matches!(
        method,
        "version"
            | "status"
            | "listgroups"
            | "history"
            | "config"
            | "loadconfig"
            | "configtemplates"
            | "log"
            | "listfiles"
            | "servervolumes"
            | "sysinfo"
            | "systemhealth"
    )
}

/// Whether `service` may be used by normal workers right now.
pub(super) async fn operational(state: &AppState, service: &str) -> Result<bool> {
    Ok(storage::operational_any(&state.db, service.to_owned()).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn authority_applies_only_to_its_service_and_task() {
        let authority = Authority {
            service: "service".into(),
            scope: Scope::Setup {
                provision: "provision".into(),
            },
        };
        assert_eq!(authority_for("service"), None);
        with_authority(authority, async {
            assert!(matches!(
                authority_for("service"),
                Some(Scope::Setup { .. })
            ));
            assert_eq!(authority_for("peer"), None);
            // Spawned work does not inherit coordinator authority.
            assert_eq!(
                tokio::spawn(async { authority_for("service") })
                    .await
                    .unwrap(),
                None
            );
        })
        .await;
    }

    #[test]
    fn only_documented_nzbget_reads_skip_authorization() {
        for method in ["status", "listgroups", "history", "config", "version"] {
            assert!(nzbget_read_only(method), "{method}");
        }
        for method in [
            "editqueue",
            "saveconfig",
            "pausedownload",
            "rate",
            "append",
            "reload",
        ] {
            assert!(!nzbget_read_only(method), "{method}");
        }
    }
}
