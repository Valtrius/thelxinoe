pub async fn issue_session(
    db: &thelxinoe_database::Database,
    user_id: String,
    transport: String,
    name: String,
) -> anyhow::Result<String> {
    let user = user_id.clone();
    let expected_hash = db
        .read("test.session_credential", move |db| {
            Ok(db.query_row(
                "SELECT password_hash FROM users WHERE id=?1",
                [user],
                |row| row.get(0),
            )?)
        })
        .await?;
    thelxinoe_auth::issue_session(
        db,
        thelxinoe_auth::SessionAuthorization::Password {
            user_id,
            expected_hash,
        },
        transport,
        name,
    )
    .await?
    .ok_or_else(|| anyhow::anyhow!("Fixture credential changed"))
}
