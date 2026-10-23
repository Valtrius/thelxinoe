use super::*;

#[tokio::test]
async fn youtube_urls_survive_repeated_requests_rotate_daily_and_follow_session_revocation() {
    let (_temp, state, cookie) = crate::test_support::fixture().await;
    let mut headers = HeaderMap::new();
    headers.insert(axum::http::header::COOKIE, cookie.parse().unwrap());
    let p = security::principal(&state, &headers).await.unwrap();
    let (a, b) = tokio::join!(youtube_artwork(&state, &p), youtube_artwork(&state, &p));
    let token = a.unwrap();
    assert_eq!(token, b.unwrap());
    // No process-local state is required to recover the token.
    assert_eq!(
        token,
        storage::youtube_artwork(&state.db, state.secrets.clone(), p.clone())
            .await
            .unwrap()
    );
    assert!(
        resolve(&state, &token, "youtube-artwork", false)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        resolve(&state, &token, "artwork", false)
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
    assert_ne!(token, youtube_artwork(&state, &other).await.unwrap());
    let scope = format!("youtube-artwork-grant:{}", p.session_id);
    state
        .secrets
        .put(
            &state.db,
            scope.clone(),
            &serde_json::to_vec(&(&token, now() - 86400)).unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(token, youtube_artwork(&state, &p).await.unwrap());
    assert!(
        resolve(&state, &token, "youtube-artwork", false)
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
        resolve(&state, &token, "youtube-artwork", false)
            .await
            .unwrap()
            .is_none()
    );
    // Rotation also collects the encrypted tokens of removed sessions.
    let other_scope = format!("youtube-artwork-grant:{}", other.session_id);
    let other_token = youtube_artwork(&state, &other).await.unwrap();
    state
        .secrets
        .put(
            &state.db,
            other_scope,
            &serde_json::to_vec(&(other_token, now() - 86400)).unwrap(),
        )
        .await
        .unwrap();
    youtube_artwork(&state, &other).await.unwrap();
    assert!(
        state
            .secrets
            .get(&state.db, &scope)
            .await
            .unwrap()
            .is_none()
    );
}
