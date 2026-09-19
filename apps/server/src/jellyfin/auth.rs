use super::Query;
use crate::{
    AppState, accounts,
    error::{ApiError, Result},
};
use axum::http::{HeaderMap, header};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use thelxinoe_core::Principal;

#[derive(Clone, Default)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub client: String,
    pub version: String,
}
pub fn authorization(headers: &HeaderMap) -> Result<std::collections::BTreeMap<String, String>> {
    let mut fields = std::collections::BTreeMap::new();
    for name in ["authorization", "x-emby-authorization"] {
        let Some(value) = headers.get(name).and_then(|v| v.to_str().ok()) else {
            continue;
        };
        if value.len() > 4096 {
            return Err(ApiError::bad("Authorization header is too long"));
        }
        let Some((scheme, parameters)) = value.split_once(' ') else {
            continue;
        };
        if !["mediabrowser", "emby"].contains(&scheme.to_ascii_lowercase().as_str()) {
            continue;
        }
        let mut parts = Vec::new();
        let mut quoted = false;
        let mut escaped = false;
        let mut start = 0;
        for (index, ch) in parameters.char_indices() {
            if escaped {
                escaped = false;
                continue;
            }
            if quoted && ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == '"' {
                quoted = !quoted;
            } else if ch == ',' && !quoted {
                parts.push(&parameters[start..index]);
                start = index + 1;
            }
        }
        if quoted || escaped {
            return Err(ApiError::unauthorized());
        }
        parts.push(&parameters[start..]);
        for pair in parts {
            let Some((key, value)) = pair.trim().split_once('=') else {
                return Err(ApiError::unauthorized());
            };
            let raw = value
                .trim()
                .trim_matches('"')
                .replace("\\\"", "\"")
                .replace("\\\\", "\\");
            let encoded = format!("value={}", raw.replace('&', "%26"));
            let value = url::form_urlencoded::parse(encoded.as_bytes())
                .next()
                .ok_or_else(ApiError::unauthorized)?
                .1
                .into_owned();
            if value.chars().any(char::is_control) {
                return Err(ApiError::unauthorized());
            }
            let key = key.to_ascii_lowercase();
            if let Some(old) = fields.insert(key, value.clone())
                && old != value
            {
                return Err(ApiError::unauthorized());
            }
        }
    }
    Ok(fields)
}
pub fn device(headers: &HeaderMap) -> Result<Device> {
    let fields = authorization(headers)?;
    if fields.get("deviceid").is_some_and(|s| s.len() > 120) {
        return Err(ApiError::bad("Device identifier is too long"));
    }
    let read = |key: &str, default: &str| {
        fields
            .get(key)
            .map(String::as_str)
            .unwrap_or(default)
            .chars()
            .take(120)
            .collect::<String>()
    };
    let device = Device {
        id: read("deviceid", ""),
        name: read("device", "Media client"),
        client: read("client", "Jellyfin client"),
        version: read("version", ""),
    };
    if device.id.is_empty() {
        return Err(ApiError::bad("The client must provide its device identity"));
    }
    Ok(device)
}
pub async fn principal(state: &AppState, headers: &HeaderMap, query: &Query) -> Result<Principal> {
    let fields = authorization(headers)?;
    let mut candidates = Vec::new();
    if let Some(token) = fields.get("token") {
        candidates.push(token.as_str());
    }
    for name in ["x-emby-token", "x-mediabrowser-token"] {
        if let Some(value) = headers.get(name).and_then(|v| v.to_str().ok()) {
            candidates.push(value);
        }
    }
    if let Some(value) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    {
        candidates.push(value);
    }
    for name in ["api_key", "apikey", "access_token"] {
        if let Some(value) = query.get(name) {
            candidates.push(value);
        }
    }
    let Some(token) = candidates.first().copied() else {
        return Err(ApiError::unauthorized());
    };
    if token.len() != 64 || candidates.iter().any(|other| *other != token) {
        return Err(ApiError::unauthorized());
    }
    let principal = thelxinoe_auth::resolve(&state.db, token, "jellyfin")
        .await?
        .ok_or_else(ApiError::unauthorized)?;
    if let Some(user) = query.get("userid")
        && super::canonical(user) != principal.user.id
    {
        return Err(ApiError::forbidden());
    }
    Ok(principal)
}
pub async fn login(
    state: &AppState,
    headers: &HeaderMap,
    address: std::net::IpAddr,
    body: Value,
) -> Result<Value> {
    let device = device(headers)?;
    let user = accounts::check_credentials(
        state,
        address,
        body["Username"].as_str().unwrap_or_default(),
        body["Pw"].as_str().unwrap_or_default(),
    )
    .await?;
    issue(state, user, device).await
}
pub async fn issue(state: &AppState, user: String, device: Device) -> Result<Value> {
    let raw = thelxinoe_auth::issue_session(
        &state.db,
        user.clone(),
        "jellyfin".into(),
        format!("{} · {}", device.client, device.name)
            .chars()
            .take(100)
            .collect(),
    )
    .await?;
    let hash = thelxinoe_auth::digest(&raw);
    let (id,date)=state.db.call(move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let session:String=tx.query_row("SELECT id FROM sessions WHERE token_hash=?1",[hash],|r|r.get(0))?;
        tx.execute("DELETE FROM sessions WHERE user_id=?1 AND id IN (SELECT session_id FROM compat_devices WHERE device_id=?2 AND client=?3)",params![user,device.id,device.client])?;
        tx.execute("INSERT INTO compat_devices VALUES (?1,?2,?3,?4)",params![session,device.id,device.client,device.version])?;
        let date:String=tx.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%SZ','now')",[],|r|r.get(0))?;tx.commit()?;Ok((session,date))
    }).await?;
    let p = thelxinoe_auth::resolve(&state.db, &raw, "jellyfin")
        .await?
        .ok_or_else(ApiError::unauthorized)?;
    Ok(
        json!({"User":user_dto(state,&p).await?,"AccessToken":raw,"ServerId":state.server_id.as_str(),"SessionInfo":{"Id":id,"UserId":p.user.id,"UserName":p.user.username,"ServerId":state.server_id.as_str(),"SupportsRemoteControl":false,"PlayableMediaTypes":["Video","Audio"],"LastActivityDate":date,"LastPlaybackCheckIn":date,"IsActive":true,"SupportsMediaControl":false,"HasCustomDeviceName":false,"SupportedCommands":[]}}),
    )
}
pub async fn user_dto(state: &AppState, p: &Principal) -> Result<Value> {
    let user = p.user.id.clone();
    let prefs = state
        .db
        .call(move |db| {
            Ok(db
                .query_row(
                    "SELECT value FROM playback_preferences WHERE user_id=?1",
                    [user],
                    |r| r.get::<_, String>(0),
                )
                .optional()?)
        })
        .await?
        .and_then(|v| serde_json::from_str::<Value>(&v).ok())
        .unwrap_or_else(|| json!({}));
    let configuration = json!({"AudioLanguagePreference":prefs["audio_language"].as_str().unwrap_or(""),"PlayDefaultAudioTrack":true,"SubtitleLanguagePreference":prefs["subtitle_language"].as_str().unwrap_or(""),"SubtitleMode":if prefs["subtitles"]==true{"Default"}else{"None"},"GroupedFolders":[],"OrderedViews":[],"LatestItemsExcludes":[],"MyMediaExcludes":[],"HidePlayedInLatest":false,"RememberAudioSelections":true,"RememberSubtitleSelections":true,"EnableNextEpisodeAutoPlay":true,"DisplayMissingEpisodes":false,"DisplayCollectionsView":false,"EnableLocalPassword":false});
    let policy = json!({"EnableUserPreferenceAccess":true,"ForceRemoteSourceTranscoding":false,"EnableMediaConversion":false,"EnableAllChannels":false,"InvalidLoginAttemptCount":0,"LoginAttemptsBeforeLockout":20,"MaxActiveSessions":0,"RemoteClientBitrateLimit":0,"SyncPlayAccess":"None","IsAdministrator":false,"IsHidden":true,"IsDisabled":false,"EnableAllFolders":true,"EnabledFolders":[],"EnableAllDevices":true,"EnabledDevices":[],"EnableMediaPlayback":true,"EnableAudioPlaybackTranscoding":true,"EnableVideoPlaybackTranscoding":true,"EnablePlaybackRemuxing":true,"EnableContentDeletion":false,"EnableContentDownloading":false,"EnableSyncTranscoding":false,"EnableRemoteAccess":true,"EnableRemoteControlOfOtherUsers":false,"EnableSharedDeviceControl":false,"EnableCollectionManagement":false,"EnableSubtitleManagement":false,"EnableLyricManagement":false,"EnablePublicSharing":false,"AuthenticationProviderId":"Thelxinoe","PasswordResetProviderId":"Thelxinoe","AccessSchedules":[],"BlockedTags":[],"BlockedMediaFolders":[],"BlockedChannels":[],"EnableLiveTvAccess":false,"EnableLiveTvManagement":false});
    Ok(
        json!({"Id":p.user.id,"Name":p.user.username,"ServerId":state.server_id.as_str(),"ServerName":"Thelxinoe","HasPassword":true,"HasConfiguredPassword":true,"HasConfiguredEasyPassword":false,"EnableAutoLogin":false,"Configuration":configuration,"Policy":policy}),
    )
}
