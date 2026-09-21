use crate::{
    AppState,
    error::{ApiError, Result},
    security,
};
use axum::{Json, extract::State, http::HeaderMap};
use base64::{Engine, engine::general_purpose::STANDARD};
use image::{ImageFormat, ImageReader, Limits, codecs::jpeg::JpegEncoder};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::Cursor;
use thelxinoe_core::{User, now};

#[derive(Serialize)]
pub struct Profile {
    #[serde(flatten)]
    user: User,
    avatar: Option<String>,
}

// Pictures belong to the profile response, not to authentication on every request.
pub async fn profile(state: &AppState, user: User) -> Result<Profile> {
    let id = user.id.clone();
    let avatar = state
        .db
        .call(move |db| {
            Ok(db
                .query_row(
                    "SELECT image FROM user_avatars WHERE user_id=?1",
                    [id],
                    |row| row.get(0),
                )
                .optional()?)
        })
        .await?;
    Ok(Profile { user, avatar })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AvatarInput {
    image: Option<String>,
}

fn normalize(image: String) -> Result<String> {
    let encoded = image
        .strip_prefix("data:image/jpeg;base64,")
        .ok_or_else(|| ApiError::bad("Save the cropped picture as a JPEG"))?;
    if encoded.len() > 500_000 {
        return Err(ApiError::bad("The cropped picture is too large"));
    }
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| ApiError::bad("Invalid picture encoding"))?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(512);
    limits.max_image_height = Some(512);
    limits.max_alloc = Some(8 * 1024 * 1024);
    let mut reader = ImageReader::with_format(Cursor::new(bytes), ImageFormat::Jpeg);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|_| ApiError::bad("Use a square picture up to 512 pixels wide"))?;
    if decoded.width() != decoded.height() || decoded.width() < 32 {
        return Err(ApiError::bad(
            "Use a square picture between 32 and 512 pixels wide",
        ));
    }
    // Re-encoding also strips source metadata and rejects malformed payloads.
    let mut output = Vec::new();
    JpegEncoder::new_with_quality(&mut output, 88)
        .encode_image(&decoded.to_rgb8())
        .map_err(anyhow::Error::from)?;
    Ok(format!(
        "data:image/jpeg;base64,{}",
        STANDARD.encode(output)
    ))
}

pub async fn save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<AvatarInput>,
) -> Result<Json<Value>> {
    let principal = security::principal(&state, &headers).await?;
    let image = tokio::task::spawn_blocking(move || input.image.map(normalize).transpose())
        .await
        .map_err(anyhow::Error::from)??;
    let saved = image.clone();
    let user_id = principal.user.id.clone();
    state.db.call(move |db| {
        if let Some(image) = image {
            db.execute("INSERT INTO user_avatars(user_id,image,updated_at) VALUES (?1,?2,?3)
                ON CONFLICT(user_id) DO UPDATE SET image=excluded.image,updated_at=excluded.updated_at",
                params![user_id, image, now()])?;
        } else {
            db.execute("DELETE FROM user_avatars WHERE user_id=?1", [user_id])?;
        }
        Ok(())
    }).await?;
    state
        .emit(
            Some(principal.user.id),
            "account.profile.changed",
            json!({}),
        )
        .await?;
    Ok(Json(json!({"avatar":saved})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::online::oauth::tests::{call, fixture};
    use axum::http::StatusCode;

    fn picture(width: u32, height: u32) -> String {
        let mut bytes = Vec::new();
        JpegEncoder::new(&mut bytes)
            .encode_image(&image::RgbImage::new(width, height))
            .unwrap();
        format!("data:image/jpeg;base64,{}", STANDARD.encode(bytes))
    }

    #[tokio::test]
    async fn profile_picture_is_validated_private_persistent_and_removable() {
        let (_temp, state, cookie) = fixture().await;
        for image in [
            "data:image/svg+xml,<svg/>".into(),
            picture(1024, 1024),
            picture(128, 64),
        ] {
            assert_eq!(
                call(
                    &state,
                    "/api/v1/me/avatar",
                    "PUT",
                    json!({"image":image}),
                    &cookie
                )
                .await
                .0,
                StatusCode::BAD_REQUEST
            );
        }
        let (status, _, saved) = call(
            &state,
            "/api/v1/me/avatar",
            "PUT",
            json!({"image":picture(128,128)}),
            &cookie,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let fresh = AppState::open(state.config.as_ref().clone()).await.unwrap();
        assert_eq!(
            call(&fresh, "/api/v1/auth/me", "GET", json!({}), &cookie)
                .await
                .2["user"]["avatar"],
            saved["avatar"]
        );
        let bob =
            thelxinoe_auth::issue_session(&state.db, "bob".into(), "web".into(), "Bob".into())
                .await
                .unwrap();
        assert!(
            call(
                &state,
                "/api/v1/auth/me",
                "GET",
                json!({}),
                &format!("thelxinoe_session={bob}")
            )
            .await
            .2["user"]["avatar"]
                .is_null()
        );
        assert_eq!(
            call(
                &state,
                "/api/v1/me/avatar",
                "PUT",
                json!({"image":null}),
                ""
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            call(
                &state,
                "/api/v1/me/avatar",
                "PUT",
                json!({"image":null}),
                &cookie
            )
            .await
            .0,
            StatusCode::OK
        );
        assert!(
            call(&state, "/api/v1/auth/me", "GET", json!({}), &cookie)
                .await
                .2["user"]["avatar"]
                .is_null()
        );
    }
}
