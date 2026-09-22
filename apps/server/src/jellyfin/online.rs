//! Private online libraries translated to the standard Jellyfin video/playlist API.

#[path = "../storage/jellyfin/online.rs"]
mod storage;

use super::{Query, canonical, catalog};
use crate::{
    AppState,
    error::{ApiError, Result},
    online::watchlists,
};
use axum::{
    http::header,
    response::{IntoResponse, Response},
};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use thelxinoe_core::{Principal, id};

pub const ROOTS: [(&str, &str, &str); 3] = [
    ("d7df0f21-caa6-45a0-9798-267150600001", "youtube", "YouTube"),
    ("d7df0f21-caa6-45a0-9798-267150600002", "twitch", "Twitch"),
    ("d7df0f21-caa6-45a0-9798-267150600003", "kick", "Kick"),
];
pub fn views(server: &str) -> Vec<Value> {
    ROOTS.iter().map(|(id, _, name)| json!({"Id":id,"Name":name,"Type":"CollectionFolder","CollectionType":"homevideos","IsFolder":true,"ServerId":server,"ImageTags":{}})).collect()
}
fn column(kind: &str) -> &'static str {
    match kind {
        "youtube" => "youtube_video_id",
        "twitch" => "twitch_channel_id",
        "kick" => "kick_slug",
        "watchlist" => "watchlist_id",
        _ => unreachable!(),
    }
}

#[derive(Clone)]
pub struct Identity {
    pub id: String,
    pub kind: String,
    pub key: String,
}
impl Identity {
    pub fn media(&self) -> String {
        format!("{}:{}", self.kind, self.key)
    }
    pub fn list(&self) -> Result<i64> {
        if self.kind != "watchlist" {
            return Err(ApiError::not_found());
        }
        self.key.parse().map_err(|_| ApiError::not_found())
    }
}
pub async fn resolve(state: &AppState, p: &Principal, item: &str) -> Result<Option<Identity>> {
    let item = canonical(item);
    let user = p.user.id.clone();
    Ok(storage::resolve(item, user, &state.db).await?)
}
pub async fn ensure_lists(state: &AppState, p: &Principal) -> Result<()> {
    let user = p.user.id.clone();
    storage::ensure_lists(user, &state.db).await?;
    Ok(())
}

// Query the provider's own user rows. UUIDs cannot confer access to another user's feed.

pub async fn browse(
    state: &AppState,
    p: &Principal,
    query: &Query,
    item: Option<&str>,
) -> Result<Option<Value>> {
    let single = if let Some(item) = item {
        let key = canonical(item);
        if let Some(root) = views(&state.server_id).into_iter().find(|r| r["Id"] == key) {
            return Ok(Some(root));
        }
        resolve(state, p, &key).await?
    } else {
        None
    };
    if item.is_some() && single.is_none() {
        return Ok(None);
    }
    if let Some(identity) = &single
        && identity.kind == "watchlist"
    {
        return Ok(Some(
            catalog::playlists(state, p, Some(&identity.id), query).await?,
        ));
    }
    let parent = query.get("parentid").map(|s| canonical(s));
    let provider = parent
        .as_ref()
        .and_then(|id| ROOTS.iter().find(|r| r.0 == id).map(|r| r.1));
    let list = if let Some(parent) = &parent {
        resolve(state, p, parent)
            .await?
            .filter(|i| i.kind == "watchlist")
    } else {
        None
    };
    if single.is_none() && parent.is_some() && provider.is_none() && list.is_none() {
        return Ok(None);
    }
    let video_only = query
        .get("includeitemtypes")
        .is_some_and(|types| types.split(',').all(|t| t.eq_ignore_ascii_case("Video")));
    let mapped_ids = if let Some(ids) = query.get("ids") {
        let mut resolved = Vec::new();
        for id in ids.split(',').take(500) {
            if let Some(item) = resolve(state, p, id).await? {
                resolved.push(item);
            }
        }
        resolved
    } else {
        Vec::new()
    };
    if single.is_none()
        && provider.is_none()
        && list.is_none()
        && !video_only
        && mapped_ids.is_empty()
    {
        return Ok(None);
    }
    let list_id = list.as_ref().map(Identity::list).transpose()?;
    let q = query.clone();
    let user = p.user.id.clone();
    let server = state.server_id.to_string();
    let is_single = single.is_some();
    let start = if is_single {
        0
    } else {
        q.get("startindex")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(0)
            .min(1_000_000)
    };
    let limit = if is_single {
        1
    } else {
        q.get("limit")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(100)
            .min(500)
    };
    let (mut items, total) = storage::browse(
        &state.db,
        storage::BrowseQuery {
            single,
            provider,
            mapped_ids,
            list_id,
            q,
            user,
            server,
            start,
            limit,
        },
    )
    .await?;
    if items.iter().any(|i| i["ImageTags"]["Primary"].is_string()) {
        let grant = crate::grants::issue(state, p, "artwork", 300).await?;
        for item in &mut items {
            if item["ImageTags"]["Primary"].is_string() {
                item["ImageTags"]["Primary"] = json!(grant);
            }
        }
    }
    if is_single {
        Ok(Some(
            items.into_iter().next().ok_or_else(ApiError::not_found)?,
        ))
    } else {
        Ok(Some(catalog::result(items, total, start)))
    }
}

pub async fn image(
    state: &AppState,
    p: &Principal,
    identity: &Identity,
    head: bool,
) -> Result<Response> {
    let key = identity.key.clone();
    let video = (identity.kind == "youtube").then(|| key.clone());
    let kind = identity.kind.clone();
    let user = p.user.id.clone();
    let address = storage::image(key, kind, user, &state.db)
        .await?
        .ok_or_else(ApiError::not_found)?;
    let (mime, bytes) = if let Some(video) = video {
        crate::online::youtube_artwork_bytes(state, &video).await?
    } else {
        crate::online::artwork_bytes(state, &address).await?
    };
    Ok((
        [
            (header::CONTENT_TYPE, mime),
            (header::CACHE_CONTROL, "private, max-age=300"),
        ],
        if head { Vec::new() } else { bytes },
    )
        .into_response())
}

pub async fn set_flag(
    state: &AppState,
    p: &Principal,
    item: &Identity,
    favorite: bool,
    value: bool,
) -> Result<()> {
    if item.kind == "youtube" {
        let _ = crate::online::feed::edit_for(
            state,
            p,
            item.key.clone(),
            crate::online::feed::Edit {
                watchlist: None,
                pinned: favorite.then_some(value),
                watched: (!favorite).then_some(value),
            },
        )
        .await?;
    } else if favorite {
        let user = p.user.id.clone();
        let key = item.id.clone();
        storage::set_flag(user, key, &state.db, value).await?;
    }
    Ok(())
}

pub async fn playlist_create(state: &AppState, p: &Principal, input: Value) -> Result<Value> {
    if input["IsPublic"] == true
        || input["UserId"]
            .as_str()
            .is_some_and(|id| canonical(id) != p.user.id)
        || input["Users"].as_array().into_iter().flatten().any(|u| {
            u["UserId"]
                .as_str()
                .is_none_or(|id| canonical(id) != p.user.id)
        })
    {
        return Err(ApiError::forbidden());
    }
    let ids = input["Ids"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let videos = playlist_videos(state, p, &ids).await?;
    let name = input["Name"].as_str().unwrap_or_default().to_owned();
    let list = watchlists::create_for(state, p, watchlists::Create { name })
        .await?
        .0
        .as_i64()
        .unwrap();
    for (index, video) in videos.into_iter().enumerate() {
        let _ = watchlists::add_for(
            state,
            p,
            list,
            watchlists::Add {
                video_id: video,
                manual_position: index as f64,
            },
        )
        .await?;
    }
    ensure_lists(state, p).await?;
    let user = p.user.id.clone();
    let id = storage::playlist_create(list, user, &state.db).await?;
    Ok(json!({"Id":id}))
}
async fn playlist_videos(state: &AppState, p: &Principal, ids: &[String]) -> Result<Vec<String>> {
    if ids.len() > 1000 {
        return Err(ApiError::bad("Playlist exceeds the saved video limit"));
    }
    let mut videos = Vec::new();
    for id in ids {
        let item = resolve(state, p, id)
            .await?
            .ok_or_else(ApiError::not_found)?;
        if item.kind != "youtube" {
            return Err(ApiError::bad(
                "YouTube watchlists accept YouTube videos only",
            ));
        }
        videos.push(item.key);
    }
    Ok(videos)
}
pub async fn playlist_change(
    state: &AppState,
    p: &Principal,
    item: &Identity,
    query: &Query,
    add: bool,
) -> Result<()> {
    let list = item.list()?;
    let ids = query
        .get(if add { "ids" } else { "entryids" })
        .map(String::as_str)
        .unwrap_or_default()
        .split(',')
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let videos = playlist_videos(state, p, &ids).await?;
    let user = p.user.id.clone();
    let position = storage::playlist_change(list, user, &state.db).await?;
    for (index, video) in videos.into_iter().enumerate() {
        if add {
            let _ = watchlists::add_for(
                state,
                p,
                list,
                watchlists::Add {
                    video_id: video,
                    manual_position: position + index as f64,
                },
            )
            .await?;
        } else {
            let _ = watchlists::remove_for(state, p, list, video).await?;
        }
    }
    Ok(())
}

pub async fn playlist_move(
    state: &AppState,
    p: &Principal,
    list: &Identity,
    entry: &str,
    index: usize,
) -> Result<()> {
    let list = list.list()?;
    let entry = resolve(state, p, entry)
        .await?
        .ok_or_else(ApiError::not_found)?;
    if entry.kind != "youtube" {
        return Err(ApiError::not_found());
    }
    let user = p.user.id.clone();
    let moved = storage::playlist_move(list, entry, user, &state.db, index).await?;
    if !moved {
        return Err(ApiError::not_found());
    }
    state
        .emit(Some(p.user.id.clone()), "youtube.changed", json!({}))
        .await?;
    Ok(())
}

pub async fn sources(state: &AppState, p: &Principal, item: &Identity) -> Result<Vec<Value>> {
    if item.kind == "watchlist" {
        return Ok(Vec::new());
    }
    let dto = browse(state, p, &Query::new(), Some(&item.id))
        .await?
        .ok_or_else(ApiError::not_found)?;
    Ok(vec![source_dto(&dto)])
}
fn source_dto(dto: &Value) -> Value {
    let mut source = json!({"Id":dto["Id"],"Name":"Public stream","Protocol":"Http","Type":"Default","IsRemote":true,"Container":"hls","RunTimeTicks":dto["RunTimeTicks"],"Bitrate":8192000,"IsInfiniteStream":dto["IsLive"],"SupportsTranscoding":true,"TranscodingSubProtocol":"hls","DefaultAudioStreamIndex":1,"DefaultSubtitleStreamIndex":-1,"MediaStreams":[{"Type":"Video","Index":0,"Codec":"h264","IsDefault":true},{"Type":"Audio","Index":1,"Codec":"aac","Channels":2,"SampleRate":48000,"IsDefault":true}],"MediaAttachments":[],"Formats":[]});
    for field in [
        "ReadAtNativeFramerate",
        "IgnoreDts",
        "IgnoreIndex",
        "GenPtsInput",
        "RequiresLooping",
        "HasSegments",
        "RequiresOpening",
        "RequiresClosing",
        "SupportsProbing",
        "SupportsDirectPlay",
        "SupportsDirectStream",
    ] {
        source[field] = json!(false);
    }
    for stream in source["MediaStreams"].as_array_mut().unwrap() {
        for field in [
            "IsExternal",
            "IsInterlaced",
            "IsForced",
            "IsHearingImpaired",
            "IsTextSubtitleStream",
            "SupportsExternalStream",
        ] {
            stream[field] = json!(false);
        }
    }
    source
}
pub async fn playback_info(
    state: &AppState,
    p: &Principal,
    item: &Identity,
    query: &Query,
    mut input: Value,
) -> Result<Value> {
    if item.kind == "watchlist" {
        return Err(ApiError::bad("Select a video in this watchlist"));
    }
    if input["UserId"]
        .as_str()
        .is_some_and(|u| canonical(u) != p.user.id)
    {
        return Err(ApiError::forbidden());
    }
    let file = input["MediaSourceId"]
        .as_str()
        .or(query.get("mediasourceid").map(String::as_str))
        .map(canonical);
    let media = item.media();
    if item.kind == "youtube" {
        crate::online::downloads::authorize(state, p, &item.key).await?;
        match crate::online::downloads::source(state, &item.key).await {
            Ok(source) => {
                if file
                    .as_ref()
                    .is_some_and(|f| f != &item.id && f != &source.id)
                {
                    return Err(ApiError::not_found());
                }
                // Standard compatibility HLS gives downloaded videos a complete
                // seekable timeline, with the same progress and retention checks.
                input["EnableDirectPlay"] = json!(false);
                input["EnableDirectStream"] = json!(false);
                return super::playback::info_source(state, p, &media, query, input, source).await;
            }
            Err(e) if e.0 == axum::http::StatusCode::CONFLICT => {}
            Err(e) => return Err(e),
        }
    } else {
        crate::online::live::authorize(state, p, &media).await?;
    }
    if file.as_ref().is_some_and(|f| f != &item.id) {
        return Err(ApiError::not_found());
    }
    if input["EnableTranscoding"] == false {
        return Err(ApiError::bad("Public streams require HLS conversion"));
    }
    let profile = &input["DeviceProfile"];
    let max = [
        input["MaxStreamingBitrate"].as_i64(),
        profile["MaxStreamingBitrate"].as_i64(),
    ]
    .into_iter()
    .flatten()
    .filter(|v| *v > 0)
    .min()
    .unwrap_or(8_192_000);
    let quality = if max >= 8_192_000 {
        "8mbps"
    } else if max >= 4_192_000 {
        "4mbps"
    } else if max >= 2_192_000 {
        "2mbps"
    } else {
        return Err(ApiError::bad(
            "This client bitrate limit is below the supported conversion quality",
        ));
    };
    // Negotiate a bounded output envelope, then fit the actual upstream aspect
    // ratio into it. Source URLs and extractor metadata stay inside the server.
    let envelope = thelxinoe_playback::Source {
        id: String::new(),
        media_id: String::new(),
        generation: String::new(),
        edition: String::new(),
        path: "public.ts".into(),
        root: std::path::PathBuf::new(),
        size: 0,
        modified: String::new(),
        probe: json!({"format":{"duration":"1"},"streams":[{"codec_type":"video","codec_name":"h264","width":1920,"height":1080,"avg_frame_rate":"30/1","pix_fmt":"yuv420p","field_order":"progressive"},{"codec_type":"audio","codec_name":"aac","channels":2,"sample_rate":"48000"}]}),
    };
    let mut capabilities = super::profile::negotiate(
        &envelope,
        None,
        profile,
        false,
        false,
        thelxinoe_playback::bitrate(quality)?.unwrap(),
    );
    let conversion = capabilities
        .conversion
        .as_mut()
        .ok_or_else(|| ApiError::bad("The client does not support H.264/AAC HLS playback"))?;
    conversion.fit = true;
    capabilities.video = vec!["h264".into()];
    capabilities.audio = vec!["aac".into()];
    let options = thelxinoe_playback::Options {
        quality: quality.into(),
        audio: None,
        subtitle: Some("off".into()),
        capabilities,
    };
    let mut dto = sources(state, p, item).await?.remove(0);
    let playback = crate::playback::create_with_delivery(
        state,
        p,
        crate::playback::Create {
            queue: None,
            media_id: media,
            file_id: None,
            position: Some(0.0),
            options,
        },
        true,
    )
    .await?;
    let sid = playback["id"].as_str().unwrap().to_owned();
    let key = sid.clone();
    storage::playback_info(key, &state.db).await?;
    dto["TranscodingUrl"] = playback["url"].clone();
    dto["TranscodingContainer"] = json!("ts");
    dto["ETag"] = playback["grant"].clone();
    dto["RunTimeTicks"] = json!(catalog::ticks(playback["duration"].as_f64().unwrap_or(0.0)));
    dto["IsInfiniteStream"] = playback["live"].clone();
    Ok(json!({"MediaSources":[dto],"PlaySessionId":sid}))
}
