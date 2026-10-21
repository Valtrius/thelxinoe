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
pub const WATCHLIST_VIEW: &str = "d7df0f21-caa6-45a0-9798-267150600004";

fn format_viewers(viewers: i64) -> String {
    if viewers < 1_000 {
        return viewers.to_string();
    }
    let viewers = viewers as u64;
    let suffixes = ["k", "m", "b", "t"];
    let mut unit = 0;
    let mut divisor = 1_000;
    let mut tenths = (viewers + divisor / 20) / (divisor / 10);
    while tenths >= 10_000 && unit < suffixes.len() - 1 {
        divisor *= 1_000;
        unit += 1;
        tenths = (viewers + divisor / 20) / (divisor / 10);
    }
    let suffix = suffixes[unit];
    if tenths.is_multiple_of(10) {
        format!("{}{suffix}", tenths / 10)
    } else {
        format!("{}.{}{suffix}", tenths / 10, tenths % 10)
    }
}

pub async fn views(state: &AppState) -> Result<Vec<Value>> {
    let mut items = Vec::new();
    for (id, provider, name) in ROOTS {
        if crate::online::availability::enabled(state, provider).await? {
            items.push(json!({"Id":id,"DisplayPreferencesId":id,"Name":name,"Type":"CollectionFolder","CollectionType":"homevideos","IsFolder":true,"ServerId":state.server_id.as_str(),"ImageTags":{}}));
            if provider == "youtube" {
                items.push(json!({"Id":WATCHLIST_VIEW,"DisplayPreferencesId":WATCHLIST_VIEW,"Name":"Watchlists","Type":"CollectionFolder","CollectionType":"homevideos","IsFolder":true,"LocationType":"Remote","ServerId":state.server_id.as_str(),"ImageTags":{}}));
            }
        }
    }
    Ok(items)
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
    let identity = storage::resolve(item, user, &state.db).await?;
    if let Some(identity) = &identity {
        let provider = if identity.kind == "watchlist" {
            "youtube"
        } else {
            &identity.kind
        };
        crate::online::availability::require_enabled(state, provider).await?;
    }
    Ok(identity)
}
pub async fn ensure_lists(state: &AppState, p: &Principal) -> Result<()> {
    if !crate::online::availability::enabled(state, "youtube").await? {
        return Ok(());
    }
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
        if let Some(root) = views(state).await?.into_iter().find(|r| r["Id"] == key) {
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
    if let Some(provider) = provider {
        crate::online::availability::require_enabled(state, provider).await?;
    }
    let watchlist_view = parent.as_deref() == Some(WATCHLIST_VIEW);
    let list = if watchlist_view {
        crate::online::availability::require_enabled(state, "youtube").await?;
        ensure_lists(state, p).await?;
        let lists = storage::watchlists(p.user.id.clone(), &state.db).await?;
        if let [list] = lists.as_slice() {
            Some(list.clone())
        } else {
            let recursive = query
                .get("recursive")
                .is_some_and(|v| v.eq_ignore_ascii_case("true"));
            let folders = query
                .get("isfolder")
                .is_some_and(|v| v.eq_ignore_ascii_case("true"))
                || query.get("includeitemtypes").is_some_and(|types| {
                    !types.split(',').any(|t| t.eq_ignore_ascii_case("Video"))
                });
            if !recursive || folders {
                let start = query
                    .get("startindex")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                if query.get("includeitemtypes").is_some_and(|types| {
                    !types.split(',').any(|t| t.eq_ignore_ascii_case("Playlist"))
                }) || query.get("excludeitemtypes").is_some_and(|types| {
                    types.split(',').any(|t| t.eq_ignore_ascii_case("Playlist"))
                }) || query
                    .get("isfolder")
                    .is_some_and(|v| v.eq_ignore_ascii_case("false"))
                {
                    return Ok(Some(catalog::result(Vec::new(), 0, start)));
                }
                return Ok(Some(catalog::playlists(state, p, None, query).await?));
            }
            None
        }
    } else if let Some(parent) = &parent {
        resolve(state, p, parent)
            .await?
            .filter(|i| i.kind == "watchlist")
    } else {
        None
    };
    if single.is_none()
        && parent.is_some()
        && provider.is_none()
        && list.is_none()
        && !watchlist_view
    {
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
        && !watchlist_view
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
            watchlists_only: watchlist_view && list_id.is_none(),
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
    crate::online::availability::require_enabled(state, "youtube").await?;
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
    if item.kind == "youtube" {
        crate::online::downloads::authorize(state, p, &item.key).await?;
        match crate::online::downloads::source(state, &item.key).await {
            Ok(source) => return Ok(vec![super::playback::source_dto(&source).await?]),
            Err(error) if error.0 == axum::http::StatusCode::CONFLICT => {}
            Err(error) => return Err(error),
        }
    }
    let key = if item.kind == "youtube" {
        item.key.clone()
    } else {
        item.media()
    };
    Ok(crate::online::streams::variants(state, p, &key)
        .await?
        .iter()
        .map(|variant| variant_dto(&dto, item, variant))
        .collect())
}
fn variant_id(item: &Identity, variant: &Value) -> String {
    let digest = thelxinoe_auth::digest(&format!(
        "{}:{}:{}:{}",
        item.id, variant["quality"], variant["video"], variant["audio"]
    ));
    uuid::Uuid::parse_str(&digest[..32]).unwrap().to_string()
}
fn variant_dto(dto: &Value, item: &Identity, variant: &Value) -> Value {
    let mut source = source_dto(dto);
    source["Id"] = json!(variant_id(item, variant));
    source["Name"] = json!(format!(
        "{} · {}",
        variant["quality"].as_str().unwrap_or_default(),
        variant["video"]
            .as_str()
            .unwrap_or_default()
            .to_ascii_uppercase()
    ));
    source["Bitrate"] = variant["bitrate"].clone();
    for (name, key) in [
        ("Codec", "video"),
        ("Width", "width"),
        ("Height", "height"),
        ("AverageFrameRate", "fps"),
        ("RealFrameRate", "fps"),
    ] {
        source["MediaStreams"][0][name] = variant[key].clone();
    }
    source["MediaStreams"][1]["Codec"] = variant["audio"].clone();
    source
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
        stream_flags(stream);
    }
    source
}
fn stream_flags(stream: &mut Value) {
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
pub async fn playback_info(
    state: &AppState,
    p: &Principal,
    item: &Identity,
    query: &Query,
    input: Value,
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
                return super::playback::info_source(state, p, &media, query, input, source).await;
            }
            Err(e) if e.0 == axum::http::StatusCode::CONFLICT => {}
            Err(e) => return Err(e),
        }
    } else {
        crate::online::live::authorize(state, p, &media).await?;
    }
    let source_key = if item.kind == "youtube" {
        item.key.as_str()
    } else {
        media.as_str()
    };
    let profile = &input["DeviceProfile"];
    let variants = crate::online::streams::variants(state, p, source_key).await?;
    let selected = file
        .as_ref()
        .filter(|id| *id != &item.id)
        .map(|id| {
            variants
                .iter()
                .find(|variant| variant_id(item, variant) == *id)
                .ok_or_else(ApiError::not_found)
        })
        .transpose()?;
    let codecs: Vec<String> = profile["TranscodingProfiles"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| {
            p["Type"] == "Video"
                && p["Protocol"]
                    .as_str()
                    .is_some_and(|p| p.eq_ignore_ascii_case("hls"))
        })
        .flat_map(|p| {
            p["VideoCodec"]
                .as_str()
                .filter(|codec| !codec.trim().is_empty())
                .unwrap_or("h264,hevc,vp9,av1")
                .split(',')
                .map(|codec| codec.trim().to_ascii_lowercase())
                .collect::<Vec<_>>()
        })
        .collect();
    let max = [
        input["MaxStreamingBitrate"].as_i64(),
        profile["MaxStreamingBitrate"].as_i64(),
        query
            .get("maxstreamingbitrate")
            .and_then(|v| v.parse().ok()),
    ]
    .into_iter()
    .flatten()
    .filter(|v| *v > 0)
    .min();
    let (source, _, rendition) = crate::online::streams::negotiate_source(
        state,
        source_key,
        if input["EnableDirectStream"] == false {
            &[]
        } else {
            &codecs
        },
        max,
        selected,
        |source| {
            super::profile::negotiate_online(
                source,
                None,
                profile,
                false,
                input["EnableDirectStream"] != false,
                None,
            )
        },
    )
    .await?;
    let source_rate = source.probe["format"]["bit_rate"]
        .as_str()
        .and_then(|v| v.parse::<i64>().ok());
    let quality = if max.is_some_and(|max| source_rate.is_none_or(|rate| rate > max)) {
        match max.unwrap() {
            n if n >= 20_192_000 => "20mbps",
            n if n >= 8_192_000 => "8mbps",
            n if n >= 4_192_000 => "4mbps",
            n if n >= 2_192_000 => "2mbps",
            _ => {
                return Err(ApiError::bad(
                    "This client bitrate limit is below the supported conversion quality",
                ));
            }
        }
    } else {
        rendition.as_deref().unwrap_or("auto")
    };
    let mut capabilities = super::profile::negotiate_online(
        &source,
        None,
        profile,
        false,
        input["EnableDirectStream"] != false,
        thelxinoe_playback::bitrate(if rendition.as_deref() == Some(quality) {
            "auto"
        } else {
            quality
        })?,
    );
    capabilities.allow_conversion = Some(input["EnableTranscoding"] != false);
    let options = thelxinoe_playback::Options {
        quality: quality.into(),
        audio: None,
        subtitle: Some("off".into()),
        capabilities,
    };
    let delivered = selected
        .or_else(|| {
            variants.iter().find(|variant| {
                Some(variant["quality"].as_str().unwrap_or_default()) == rendition.as_deref()
                    && variant["video"] == source.video_codec().unwrap_or_default()
            })
        })
        .unwrap_or(&variants[0]);
    let item_dto = browse(state, p, &Query::new(), Some(&item.id))
        .await?
        .ok_or_else(ApiError::not_found)?;
    let mut dto = variant_dto(&item_dto, item, delivered);
    let playback = crate::playback::create_with_delivery(
        state,
        p,
        crate::playback::Create {
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
    dto["TranscodingContainer"] = playback["options"]["capabilities"]["hls_container"]
        .as_str()
        .map(|c| json!(c))
        .unwrap_or(json!("ts"));
    dto["ETag"] = playback["grant"].clone();
    dto["RunTimeTicks"] = json!(catalog::ticks(playback["duration"].as_f64().unwrap_or(0.0)));
    dto["IsInfiniteStream"] = playback["live"].clone();
    dto["MediaStreams"] = json!(
        playback["probe"]["streams"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|stream| {
                ["video", "audio"].contains(&stream["codec_type"].as_str().unwrap_or(""))
            })
            .map(|stream| {
                let video = stream["codec_type"] == "video";
                let encoded = playback["encoding"][if video { "video" } else { "audio" }] == true;
                let conversion = &playback["options"]["capabilities"]["conversion"];
                let rate = stream["avg_frame_rate"]
                    .as_str()
                    .and_then(|s| s.split_once('/'))
                    .and_then(|(a, b)| Some(a.parse::<f64>().ok()? / b.parse::<f64>().ok()?))
                    .filter(|r| r.is_finite());
                let sample_rate = if encoded && !video {
                    Some(48000)
                } else {
                    stream["sample_rate"].as_str().and_then(|r| r.parse::<u32>().ok())
                };
                let mut dto = json!({"Type": if video { "Video" } else { "Audio" }, "Index": stream["index"], "Codec": if encoded { json!(if video { "h264" } else { "aac" }) } else { stream["codec_name"].clone() }, "IsDefault": true, "Width": if encoded && video {conversion["width"].clone()}else{stream["width"].clone()}, "Height": if encoded && video {conversion["height"].clone()}else{stream["height"].clone()},"AverageFrameRate":if encoded && video {conversion["frame_rate"].clone()}else{json!(rate)},"RealFrameRate":if encoded && video {conversion["frame_rate"].clone()}else{json!(rate)}, "Channels": if encoded && !video { json!(2) } else { stream["channels"].clone() }, "SampleRate": sample_rate});
                stream_flags(&mut dto);
                dto["IsInterlaced"] = json!(video && !encoded && !["progressive", "unknown", ""].contains(&stream["field_order"].as_str().unwrap_or("")));
                dto
            })
            .collect::<Vec<_>>()
    );
    Ok(json!({"MediaSources":[dto],"PlaySessionId":sid}))
}
