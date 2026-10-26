use super::{Candidate, Job, Tool, storage};
use crate::AppState;

fn candidate(id: &str) -> Candidate {
    Candidate {
        id: id.into(),
        tool: "streamlink".into(),
        channel: "stable".into(),
        version: "8.6.2".into(),
        platform: "linux-x86_64".into(),
        notes_url: String::new(),
        source: String::new(),
        license: String::new(),
        artifacts: vec![],
        python_abi: None,
    }
}

async fn streamlink(state: &AppState) -> Tool {
    storage::inventory(&state.db)
        .await
        .unwrap()
        .into_iter()
        .find(|tool| tool.id == "streamlink")
        .unwrap()
}

async fn job(state: &AppState, id: &str) -> Job {
    storage::jobs(&state.db)
        .await
        .unwrap()
        .into_iter()
        .find(|job| job.id == id)
        .unwrap()
}

async fn install(state: &AppState, id: &str, manual: bool) -> anyhow::Result<String> {
    storage::enqueue(
        &state.db,
        streamlink(state).await,
        "install".into(),
        Some(candidate(id)),
        manual,
        None,
    )
    .await
}

async fn discover(state: &AppState, found: Option<&str>) {
    let error = found
        .is_none()
        .then(|| "Release service unavailable".into());
    storage::discover(
        &state.db,
        streamlink(state).await,
        found.map(candidate),
        error,
        None,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn a_requested_install_replaces_an_update_frozen_on_an_older_candidate() {
    let (_temp, state, _) = crate::test_support::fixture().await;
    let frozen = install(&state, "old", false).await.unwrap();
    let scheduled = install(&state, "new", false).await.unwrap_err();
    let repair = storage::enqueue(
        &state.db,
        streamlink(&state).await,
        "repair".into(),
        Some(candidate("new")),
        true,
        None,
    )
    .await
    .unwrap_err();
    for error in [scheduled, repair] {
        assert_eq!(
            error.to_string(),
            "Another operation is pending for this tool"
        );
    }
    assert_eq!(job(&state, &frozen).await.stage, "queued");

    let requested = install(&state, "new", true).await.unwrap();
    assert_ne!(requested, frozen);
    let replaced = job(&state, &frozen).await;
    assert_eq!(replaced.stage, "canceled");
    assert_eq!(
        replaced.reason.as_deref(),
        Some("Replaced by the latest release")
    );
    let requested = job(&state, &requested).await;
    assert_eq!(requested.stage, "queued");
    assert!(requested.manual);
    assert_eq!(requested.candidate.unwrap().id, "new");
}

#[tokio::test]
async fn a_check_finding_another_candidate_cancels_only_automatic_updates_frozen_on_the_old_one() {
    let (_temp, state, _) = crate::test_support::fixture().await;
    let automatic = install(&state, "old", false).await.unwrap();
    discover(&state, Some("old")).await;
    discover(&state, None).await;
    assert_eq!(job(&state, &automatic).await.stage, "queued");

    discover(&state, Some("new")).await;
    let replaced = job(&state, &automatic).await;
    assert_eq!(replaced.stage, "canceled");
    assert_eq!(
        replaced.reason.as_deref(),
        Some("Replaced by the latest release")
    );

    // An administrator's install of a specific release is left to them.
    let manual = install(&state, "new", true).await.unwrap();
    discover(&state, Some("newer")).await;
    assert_eq!(job(&state, &manual).await.stage, "queued");
}
