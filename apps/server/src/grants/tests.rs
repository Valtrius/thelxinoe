use super::*;

#[tokio::test]
async fn youtube_urls_survive_repeated_requests_rotate_daily_and_follow_session_revocation() {
    stable_artwork_urls("youtube-artwork", "artwork").await;
}

#[tokio::test]
async fn library_urls_survive_repeated_requests_rotate_daily_and_follow_session_revocation() {
    stable_artwork_urls("artwork", "youtube-artwork").await;
}

async fn stable_artwork_urls(resource: &'static str, other_resource: &'static str) {
    async fn issue(state: &AppState, p: &Principal, resource: &str) -> anyhow::Result<String> {
        if resource == "artwork" {
            artwork(state, p).await
        } else {
            youtube_artwork(state, p).await
        }
    }
    let (_temp, state, cookie) = crate::test_support::fixture().await;
    let mut headers = HeaderMap::new();
    headers.insert(axum::http::header::COOKIE, cookie.parse().unwrap());
    let p = security::principal(&state, &headers).await.unwrap();
    let (a, b) = tokio::join!(issue(&state, &p, resource), issue(&state, &p, resource));
    let token = a.unwrap();
    assert_eq!(token, b.unwrap());
    // No process-local state is required to recover the token.
    assert_eq!(
        token,
        storage::artwork(&state.db, state.secrets.clone(), p.clone(), resource)
            .await
            .unwrap()
    );
    assert!(
        resolve(&state, &token, resource, false)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        resolve(&state, &token, other_resource, false)
            .await
            .unwrap()
            .is_none()
    );
    let other_session = crate::test_support::issue_session(
        &state.db,
        "alice".into(),
        "web".into(),
        "Other browser".into(),
    )
    .await
    .unwrap();
    headers.insert(
        axum::http::header::COOKIE,
        format!("thelxinoe_session={other_session}")
            .parse()
            .unwrap(),
    );
    let other = security::principal(&state, &headers).await.unwrap();
    assert_ne!(token, issue(&state, &other, resource).await.unwrap());
    let scope = format!("{resource}-grant:{}", p.session_id);
    state
        .secrets
        .put(
            &state.db,
            scope.clone(),
            &serde_json::to_vec(&(&token, now() - 86400)).unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(token, issue(&state, &p, resource).await.unwrap());
    assert!(
        resolve(&state, &token, resource, false)
            .await
            .unwrap()
            .is_some()
    );
    let session = p.session_id.clone();
    state
        .db
        .write("test.revoke", move |db| {
            db.execute("DELETE FROM sessions WHERE id=?1", [session])?;
            Ok(())
        })
        .await
        .unwrap();
    assert!(
        resolve(&state, &token, resource, false)
            .await
            .unwrap()
            .is_none()
    );
    // Rotation also collects the encrypted tokens of removed sessions.
    let other_scope = format!("{resource}-grant:{}", other.session_id);
    let other_token = issue(&state, &other, resource).await.unwrap();
    state
        .secrets
        .put(
            &state.db,
            other_scope,
            &serde_json::to_vec(&(other_token, now() - 86400)).unwrap(),
        )
        .await
        .unwrap();
    issue(&state, &other, resource).await.unwrap();
    assert!(
        state
            .secrets
            .get(&state.db, &scope)
            .await
            .unwrap()
            .is_none()
    );
}
