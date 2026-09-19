use super::{Query, canonical, catalog::ticks};
use crate::{
    AppState,
    error::{ApiError, Result},
    playback as core,
};
use axum::{
    extract::{Path, Request, State},
    response::Response,
};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use thelxinoe_core::Principal;
use thelxinoe_playback::{Capabilities, Options, Source, Track};

fn track_index(track: &Track) -> i64 {
    track.index.unwrap_or_else(|| {
        // Sidecars retain the same index if another subtitle is added or removed.
        let hash = thelxinoe_auth::digest(&track.id);
        1_000_000 + i64::from_str_radix(&hash[..7], 16).expect("hex digest")
    })
}
pub async fn source_dto(source: &Source) -> Result<Value> {
    let tracks = source.tracks().await?;
    let mut streams = Vec::new();
    for stream in source.probe["streams"].as_array().into_iter().flatten() {
        if stream["codec_type"] != "video" {
            continue;
        }
        let rate = stream["avg_frame_rate"]
            .as_str()
            .and_then(|v| v.split_once('/'))
            .and_then(|(a, b)| Some(a.parse::<f64>().ok()? / b.parse::<f64>().ok()?))
            .filter(|v| v.is_finite())
            .unwrap_or(0.0);
        streams.push(json!({"Type":"Video","Index":stream["index"],"Codec":stream["codec_name"],"Width":stream["width"],"Height":stream["height"],"BitDepth":stream["bits_per_raw_sample"].as_str().and_then(|v|v.parse::<u32>().ok()),"VideoRange":"SDR","VideoRangeType":"SDR","RealFrameRate":rate,"AverageFrameRate":rate,"IsDefault":true,"IsExternal":false,"IsInterlaced":false,"IsForced":false,"IsHearingImpaired":false,"IsTextSubtitleStream":false,"SupportsExternalStream":false}));
    }
    let mut indexes = std::collections::HashSet::new();
    for track in &tracks {
        let index = track_index(track);
        if !indexes.insert(index) {
            return Err(ApiError::conflict(
                "Subtitle index collision; rename one of the sidecar files",
            ));
        }
        let probe = source.probe["streams"]
            .as_array()
            .and_then(|v| v.iter().find(|s| s["index"].as_i64() == track.index));
        let mut item = json!({"Type":if track.kind=="audio"{"Audio"}else{"Subtitle"},"Index":index,"Codec":track.codec,"Language":track.language,"DisplayTitle":if track.title.is_empty(){&track.language}else{&track.title},"Title":track.title,"IsDefault":track.default,"IsForced":false,"IsInterlaced":false,"IsHearingImpaired":false,"IsExternal":track.path.is_some(),"SupportsExternalStream":track.kind=="subtitle" && track.supported,"IsTextSubtitleStream":track.kind=="subtitle" && track.supported});
        if track.kind == "audio" {
            item["Channels"] = probe.map(|v| v["channels"].clone()).unwrap_or(json!(2));
            item["SampleRate"] = json!(
                probe
                    .and_then(|v| v["sample_rate"].as_str())
                    .and_then(|v| v.parse::<u32>().ok())
            );
        }
        streams.push(item);
    }
    let bitrate = source.probe["format"]["bit_rate"]
        .as_str()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0);
    Ok(
        json!({"Id":source.id,"Name":if source.edition.is_empty(){"Original"}else{&source.edition},"Protocol":"File","Type":"Default","IsRemote":false,"Container":source.path.extension().and_then(|v|v.to_str()).unwrap_or(""),"Size":source.size,"RunTimeTicks":ticks(source.duration()),"Bitrate":bitrate,"ETag":source.generation,"ReadAtNativeFramerate":false,"IgnoreDts":false,"IgnoreIndex":false,"GenPtsInput":false,"IsInfiniteStream":false,"RequiresLooping":false,"SupportsProbing":false,"TranscodingSubProtocol":"hls","HasSegments":false,"RequiresOpening":false,"RequiresClosing":false,"SupportsDirectPlay":true,"SupportsDirectStream":true,"SupportsTranscoding":true,"MediaStreams":streams,"MediaAttachments":[],"Formats":[],"DefaultAudioStreamIndex":tracks.iter().find(|t|t.kind=="audio" && t.default).or_else(||tracks.iter().find(|t|t.kind=="audio")).and_then(|t|t.index),"DefaultSubtitleStreamIndex":-1}),
    )
}
pub async fn sources(state: &AppState, media: &str) -> Result<Vec<Value>> {
    let mid = media.to_owned();
    let files=state.db.call(move|db|Ok(db.prepare("SELECT f.id FROM media_sources s JOIN media_files f ON f.id=s.file_id WHERE s.media_id=?1 AND f.present=1 ORDER BY f.edition,f.id")?.query_map([mid],|r|r.get::<_,String>(0))?.collect::<std::result::Result<Vec<_>,_>>()?)).await?;
    let mut result = Vec::new();
    for file in files {
        result.push(source_dto(&core::source(state, media, Some(&file)).await?).await?);
    }
    Ok(result)
}
fn csv_matches(value: &Value, wanted: &str) -> bool {
    value
        .as_str()
        .is_none_or(|v| v.is_empty() || v.split(',').any(|s| s.trim().eq_ignore_ascii_case(wanted)))
}
pub async fn info(
    state: &AppState,
    p: &Principal,
    media: &str,
    query: &Query,
    input: Value,
) -> Result<Value> {
    if let Some(user) = input["UserId"].as_str()
        && canonical(user) != p.user.id
    {
        return Err(ApiError::forbidden());
    }
    let file = input["MediaSourceId"]
        .as_str()
        .or(query.get("mediasourceid").map(String::as_str))
        .map(canonical);
    let source = core::source(state, media, file.as_deref()).await?;
    let mut dto = source_dto(&source).await?;
    let tracks = source.tracks().await?;
    let field = |name: &str| {
        input.get(name).cloned().or_else(|| {
            query
                .get(&name.to_ascii_lowercase())
                .map(|v| serde_json::from_str(v).unwrap_or_else(|_| json!(v)))
        })
    };
    let audio = field("AudioStreamIndex")
        .and_then(|v| v.as_i64())
        .filter(|v| *v >= 0);
    let sub = field("SubtitleStreamIndex").and_then(|v| v.as_i64());
    let subtitle = match sub {
        Some(-1) => Some("off".into()),
        Some(index) => Some(
            tracks
                .iter()
                .find(|t| t.kind == "subtitle" && track_index(t) == index)
                .ok_or_else(|| ApiError::bad("Unknown subtitle index"))?
                .id
                .clone(),
        ),
        None => None,
    };
    let codec_audio = tracks
        .iter()
        .find(|t| t.kind == "audio" && audio.is_none_or(|i| Some(i) == t.index))
        .map(|t| t.codec.as_str())
        .unwrap_or("");
    let video = source.video_codec().unwrap_or_default();
    let container = dto["Container"].as_str().unwrap_or_default();
    let profile = &input["DeviceProfile"];
    let direct = field("EnableDirectPlay").as_ref() != Some(&json!(false))
        && profile["DirectPlayProfiles"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|item| {
                csv_matches(&item["Container"], container)
                    && csv_matches(&item["VideoCodec"], video)
                    && csv_matches(&item["AudioCodec"], codec_audio)
            });
    let hls = profile["TranscodingProfiles"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|v| {
            v["Protocol"]
                .as_str()
                .is_some_and(|s| s.eq_ignore_ascii_case("hls"))
        });
    let mut caps = Capabilities {
        containers: if direct {
            vec![container.to_owned()]
        } else {
            Vec::new()
        },
        video: vec![video.to_owned()],
        audio: vec![codec_audio.into()],
        hls,
        native_tracks: true,
    };
    if field("EnableDirectStream").as_ref() == Some(&json!(false)) {
        caps.video.clear();
        caps.audio.clear();
    }
    let max = field("MaxStreamingBitrate").and_then(|v| v.as_i64());
    let quality = if max.is_some_and(|v| v > 0 && v < dto["Bitrate"].as_i64().unwrap_or(0)) {
        let max = max.unwrap();
        if max < 2_192_000 {
            return Err(ApiError::bad(
                "This client bitrate limit is below the supported conversion quality",
            ));
        }
        if max >= 20_192_000 {
            "20mbps"
        } else if max >= 8_192_000 {
            "8mbps"
        } else if max >= 4_192_000 {
            "4mbps"
        } else {
            "2mbps"
        }
    } else {
        "auto"
    };
    let options = Options {
        quality: quality.into(),
        audio,
        subtitle,
        capabilities: caps,
    };
    let planned =
        thelxinoe_playback::plan(&source, &options).map_err(|e| ApiError::bad(e.to_string()))?;
    if planned == "transcode" && field("EnableTranscoding").as_ref() == Some(&json!(false)) {
        return Err(ApiError::bad("The client disabled the required conversion"));
    }
    let playback = core::create_for(
        state,
        p,
        core::Create {
            queue: None,
            media_id: media.into(),
            file_id: Some(source.id.clone()),
            position: Some(0.0),
            options,
        },
    )
    .await?;
    let id = playback["id"].as_str().unwrap().to_owned();
    let key = id.clone();
    state
        .db
        .call(move |db| {
            db.execute(
                "INSERT INTO compat_playbacks(playback_id) VALUES (?1)",
                [key],
            )?;
            Ok(())
        })
        .await?;
    dto["SupportsDirectPlay"] = json!(playback["mode"] == "direct");
    dto["SupportsDirectStream"] = json!(playback["mode"] == "remux");
    dto["SupportsTranscoding"] = json!(playback["mode"] == "transcode");
    dto["DirectStreamUrl"] = playback["url"].clone();
    // The video backend preserves ETag as the stream URL's tag parameter.
    // Use that field for a scoped grant when it omits the device login header.
    dto["ETag"] = playback["grant"].clone();
    if playback["mode"] != "direct" {
        dto["TranscodingUrl"] = playback["url"].clone();
        dto["TranscodingSubProtocol"] = json!("hls");
        dto["TranscodingContainer"] = json!("ts");
    }
    if !playback["options"]["audio"].is_null() {
        dto["DefaultAudioStreamIndex"] = playback["options"]["audio"].clone();
    }
    dto["DefaultSubtitleStreamIndex"] = json!(
        tracks
            .iter()
            .find(|t| Some(t.id.as_str()) == playback["selected_subtitle"].as_str())
            .map(track_index)
            .unwrap_or(-1)
    );
    if let Some(streams) = dto["MediaStreams"].as_array_mut() {
        for stream in streams {
            if stream["Type"] == "Subtitle"
                && stream["IsTextSubtitleStream"] == true
                && let Some(track) = tracks
                    .iter()
                    .find(|t| Some(track_index(t)) == stream["Index"].as_i64())
                && let Some(sub) = playback["subtitles"]
                    .as_array()
                    .and_then(|a| a.iter().find(|s| s["id"] == track.id))
            {
                stream["DeliveryUrl"] = sub["url"].clone();
                stream["DeliveryMethod"] = json!("External");
                stream["IsExternal"] = json!(true);
                stream["Codec"] = json!("webvtt");
            }
        }
    }
    Ok(json!({"MediaSources":[dto],"PlaySessionId":id}))
}
pub async fn report(state: &AppState, p: &Principal, input: Value, stopped: bool) -> Result<()> {
    let id = canonical(
        input["PlaySessionId"]
            .as_str()
            .ok_or_else(|| ApiError::bad("Missing play session"))?,
    );
    let media = canonical(input["ItemId"].as_str().unwrap_or_default());
    let key = id.clone();
    let user = p.user.id.clone();
    let auth = p.session_id.clone();
    let row=state.db.call(move|db|{
        let tx=db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let row=tx.query_row("SELECT c.sequence,s.position FROM compat_playbacks c JOIN playback_sessions s ON s.id=c.playback_id WHERE s.id=?1 AND s.media_id=?2 AND s.user_id=?3 AND s.auth_session_id=?4",params![key,media,user,auth],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,f64>(1)?))).optional()?;
        if row.is_some(){tx.execute("UPDATE compat_playbacks SET sequence=sequence+1 WHERE playback_id=?1",[key])?;}
        tx.commit()?;Ok(row)
    }).await?.ok_or_else(ApiError::not_found)?;
    core::report(
        state,
        p,
        &id,
        core::Progress {
            sequence: row.0,
            position: input["PositionTicks"]
                .as_f64()
                .map(|v| v / 10_000_000.0)
                .unwrap_or(row.1),
            state: if stopped {
                "stopped"
            } else if input["IsPaused"] == true {
                "paused"
            } else {
                "playing"
            }
            .into(),
        },
    )
    .await?;
    Ok(())
}
pub async fn stream(
    state: AppState,
    p: &Principal,
    media: &str,
    q: &Query,
    request: Request,
) -> Result<Response> {
    let id = canonical(
        q.get("playsessionid")
            .ok_or_else(|| ApiError::bad("Missing play session"))?,
    );
    let key = id.clone();
    let uid = p.user.id.clone();
    let auth = p.session_id.clone();
    let media = media.to_owned();
    let valid=state.db.call(move|db|Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM playback_sessions WHERE id=?1 AND user_id=?2 AND auth_session_id=?3 AND media_id=?4 AND mode='direct')",params![key,uid,auth,media],|r|r.get::<_,bool>(0))?)).await?;
    if !valid {
        return Err(ApiError::not_found());
    }
    let grant = crate::grants::issue(&state, p, &format!("playback:{id}"), 120).await?;
    core::stream(
        State(state),
        Path(id),
        axum::extract::Query(core::Grant { grant }),
        request,
    )
    .await
}
