//! Trusted transfer workers. They run without network or Docker access, with mounts fixed
//! by the controller, and report through `/progress`.
use anyhow::Context;
use rusqlite::{Connection, OpenFlags, types::ValueRef};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    os::unix::fs::MetadataExt,
    path::{Component, Path, PathBuf},
};

fn write(name: &str, value: &Value) {
    let path = Path::new("/progress").join(name);
    let temporary = path.with_extension("tmp");
    if fs::write(&temporary, value.to_string()).is_ok() {
        let _ = fs::rename(&temporary, path);
    }
}
fn phase(name: &str) {
    write("progress.json", &json!({"phase":name}));
}
/// Report a result or a failure the controller can show.
fn reported(work: impl FnOnce() -> anyhow::Result<Value>) -> anyhow::Result<()> {
    phase("running");
    match work() {
        Ok(value) => {
            write("report.json", &value);
            write("progress.json", &json!({"phase":"complete"}));
            Ok(())
        }
        Err(error) => {
            write(
                "progress.json",
                &json!({"phase":"failed","error":format!("{error:#}")}),
            );
            Err(error)
        }
    }
}
fn input() -> anyhow::Result<Value> {
    Ok(serde_json::from_slice(
        &fs::read("/progress/input.json").context("Missing worker input")?,
    )?)
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ---------- State inspection ----------

fn add(tables: &mut Map<String, Value>, name: &str, value: Option<Value>) {
    if let Some(value) = value {
        tables.insert(name.into(), value);
    }
}

fn open(path: &Path, review: bool) -> anyhow::Result<Option<Connection>> {
    if !path.is_file() {
        return Ok(None);
    }
    Ok(Some(if review {
        // The live source keeps running during review and is never written. Recent
        // changes, even its schema, may still live in its write-ahead log, so read a
        // private copy of both; the authoritative inspection follows the stop.
        let wal = PathBuf::from(format!("{}-wal", path.display()));
        let size = |p: &Path| fs::metadata(p).map(|m| m.len()).unwrap_or(0);
        if size(&wal) > 0 && size(path) + size(&wal) <= REVIEW_COPY_LIMIT {
            let copy = Path::new("/tmp").join(format!(
                "review-{}",
                path.file_name().and_then(|n| n.to_str()).unwrap_or("db")
            ));
            fs::copy(path, &copy)?;
            fs::copy(&wal, format!("{}-wal", copy.display()))?;
            Connection::open(&copy)?
        } else {
            Connection::open_with_flags(
                format!("file:{}?immutable=1", path.display()),
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
            )?
        }
    } else {
        Connection::open(path)?
    }))
}
/// Review copies live in the worker's temporary storage.
const REVIEW_COPY_LIMIT: u64 = 768 * 1024 * 1024;
fn json_value(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(v) => json!(v),
        ValueRef::Real(v) => json!(v),
        ValueRef::Text(v) => json!(String::from_utf8_lossy(v)),
        ValueRef::Blob(v) => json!(format!("blob:{}", hex(&Sha256::digest(v)))),
    }
}
fn exists(c: &Connection, table: &str) -> bool {
    c.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [table],
        |r| r.get::<_, bool>(0),
    )
    .unwrap_or(false)
}
/// Row identities (first column) and a digest of the selected stable columns.
fn records(c: &Connection, table: &str, sql: &str, strict: bool) -> anyhow::Result<Option<Value>> {
    if !exists(c, table) {
        return Ok(None);
    }
    let mut statement = c.prepare(sql)?;
    let columns = statement.column_count();
    let mut rows = statement.query([])?;
    let mut hasher = Sha256::new();
    let mut ids = Vec::new();
    let mut count = 0u64;
    while let Some(row) = rows.next()? {
        let values = (0..columns)
            .map(|i| row.get_ref(i).map(json_value))
            .collect::<rusqlite::Result<Vec<_>>>()?;
        hasher.update(serde_json::to_vec(&values)?);
        ids.push(values[0].clone());
        count += 1;
    }
    Ok(Some(
        json!({"count":count,"digest":hex(&hasher.finalize()),"strict":strict,
        "ids":if count <= 50_000 {json!(ids)} else {Value::Null}}),
    ))
}
fn strings(c: &Connection, table: &str, sql: &str) -> anyhow::Result<Vec<String>> {
    if !exists(c, table) {
        return Ok(vec![]);
    }
    let mut statement = c.prepare(sql)?;
    let rows = statement.query_map([], |r| r.get::<_, Option<String>>(0))?;
    Ok(rows.filter_map(|r| r.ok().flatten()).collect())
}

/// Hosts and ports configured in settings documents; used for egress sentinels and peers.
fn endpoints(settings: &Value, remotes: &mut Vec<Value>, peers: &mut Vec<String>) {
    match settings {
        Value::Object(map) => {
            let port = map
                .get("port")
                .and_then(|p| p.as_u64().or_else(|| p.as_str()?.parse().ok()));
            for (key, value) in map {
                let Some(text) = value.as_str().filter(|t| !t.is_empty()) else {
                    endpoints(value, remotes, peers);
                    continue;
                };
                let lowered = key.to_ascii_lowercase();
                if matches!(lowered.as_str(), "host" | "hostname" | "ip" | "server") {
                    remotes.push(json!({"host":text,"port":port.unwrap_or(80)}));
                    peers.push(text.to_owned());
                } else if (lowered.ends_with("url") || lowered == "baseurl")
                    && let Ok(url) = reqwest::Url::parse(text)
                    && let Some(host) = url.host_str()
                {
                    remotes.push(
                        json!({"host":host,"port":url.port_or_known_default().unwrap_or(80)}),
                    );
                    peers.push(host.to_owned());
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|i| endpoints(i, remotes, peers)),
        _ => {}
    }
}
fn settings_endpoints(
    c: &Connection,
    tables: &[&str],
    remotes: &mut Vec<Value>,
    peers: &mut Vec<String>,
) -> anyhow::Result<()> {
    for table in tables {
        for text in strings(c, table, &format!("SELECT Settings FROM {table}"))? {
            if let Ok(value) = serde_json::from_str::<Value>(&text) {
                endpoints(&value, remotes, peers);
            }
        }
    }
    Ok(())
}

fn xml_field(text: &str, field: &str) -> Value {
    text.split_once(&format!("<{field}>"))
        .and_then(|(_, tail)| tail.split_once(&format!("</{field}>")))
        .map(|(value, _)| json!(value))
        .unwrap_or(Value::Null)
}

pub fn inspect(kind: &str, mode: &str) -> anyhow::Result<()> {
    reported(|| {
        let root = Path::new("/state");
        let review = mode == "review";
        anyhow::ensure!(root.is_dir(), "Configuration mount missing");
        let mut report = Map::new();
        let mut blockers: Vec<String> = Vec::new();
        let mut references: Vec<Value> = Vec::new();
        let mut remotes: Vec<Value> = Vec::new();
        let mut peers: Vec<String> = Vec::new();
        let mut tables = Map::new();
        let mut files: Vec<Value> = Vec::new();
        let mut format = json!({});
        let mut config = json!({});
        match kind {
            "radarr" | "sonarr" | "lidarr" | "prowlarr" => {
                let xml =
                    fs::read_to_string(root.join("config.xml")).context("config.xml is missing")?;
                config = json!({"url_base":xml_field(&xml,"UrlBase"),"port":xml_field(&xml,"Port").as_str().and_then(|p| p.parse::<u16>().ok()),
                    "authentication":xml_field(&xml,"AuthenticationMethod"),"ssl":xml_field(&xml,"EnableSsl").as_str().map(|v| v.eq_ignore_ascii_case("true")),
                    "update_automatically":xml_field(&xml,"UpdateAutomatically").as_str().map(|v| v.eq_ignore_ascii_case("true")),
                    "allowed_hosts":xml_field(&xml,"AllowedHosts"),
                    "api_key":!xml_field(&xml,"ApiKey").as_str().unwrap_or("").is_empty()});
                if let Some(path) = xml_field(&xml, "SslCertPath")
                    .as_str()
                    .filter(|p| !p.is_empty())
                    && config["ssl"] == true
                {
                    references.push(
                        json!({"path":path,"kind":"file","access":"r","label":"HTTPS certificate"}),
                    );
                }
                let db = root.join(format!("{kind}.db"));
                let c = open(&db, review)?.with_context(|| format!("{kind}.db is missing"))?;
                format = json!({"database":format!("{kind}.db"),"schema":c.query_row("SELECT MAX(Version) FROM VersionInfo",[],|r| r.get::<_,Option<i64>>(0))?});
                for (name, table) in [
                    ("download_clients", "DownloadClients"),
                    ("indexers", "Indexers"),
                    ("notifications", "Notifications"),
                    ("import_lists", "ImportLists"),
                    ("applications", "Applications"),
                    ("indexer_proxies", "IndexerProxies"),
                ] {
                    add(
                        &mut tables,
                        name,
                        records(
                            &c,
                            table,
                            &format!(
                                "SELECT Id,Name,Implementation,Settings FROM {table} ORDER BY Id"
                            ),
                            true,
                        )?,
                    );
                }
                add(
                    &mut tables,
                    "tags",
                    records(&c, "Tags", "SELECT Id,Label FROM Tags ORDER BY Id", true)?,
                );
                add(
                    &mut tables,
                    "quality_profiles",
                    records(
                        &c,
                        "QualityProfiles",
                        "SELECT Id,Name,Cutoff,Items FROM QualityProfiles ORDER BY Id",
                        true,
                    )?,
                );
                add(
                    &mut tables,
                    "custom_formats",
                    records(
                        &c,
                        "CustomFormats",
                        "SELECT Id,Name,Specifications FROM CustomFormats ORDER BY Id",
                        true,
                    )?,
                );
                add(
                    &mut tables,
                    "history",
                    records(&c, "History", "SELECT Id FROM History ORDER BY Id", false)?,
                );
                settings_endpoints(
                    &c,
                    &[
                        "DownloadClients",
                        "Indexers",
                        "Notifications",
                        "ImportLists",
                        "Applications",
                        "IndexerProxies",
                    ],
                    &mut remotes,
                    &mut peers,
                )?;
                for path in strings(
                    &c,
                    "Notifications",
                    "SELECT json_extract(Settings,'$.path') FROM Notifications WHERE Implementation='CustomScript'",
                )? {
                    references.push(
                        json!({"path":path,"kind":"file","access":"x","label":"Custom script"}),
                    );
                }
                if kind != "prowlarr" {
                    for path in strings(&c, "RootFolders", "SELECT Path FROM RootFolders")? {
                        references.push(
                            json!({"path":path,"kind":"dir","access":"rw","label":"Root folder"}),
                        );
                    }
                    add(
                        &mut tables,
                        "root_folders",
                        records(
                            &c,
                            "RootFolders",
                            "SELECT Id,Path FROM RootFolders ORDER BY Id",
                            true,
                        )?,
                    );
                }
                let (items, item_sql, file_sql) = match kind {
                    "radarr" => (
                        (
                            "movies",
                            "Movies",
                            "SELECT m.Id,md.TmdbId,m.Path,m.Monitored,m.QualityProfileId,m.MovieFileId FROM Movies m LEFT JOIN MovieMetadata md ON md.Id=m.MovieMetadataId ORDER BY m.Id",
                        ),
                        Some((
                            "movie_files",
                            "MovieFiles",
                            // Radarr links one file per movie; a rescan race can leave unlinked rows.
                            "SELECT f.Id,f.MovieId,f.RelativePath,f.Size FROM MovieFiles f JOIN Movies m ON m.MovieFileId=f.Id ORDER BY f.Id",
                        )),
                        "SELECT m.Path||'/'||f.RelativePath,f.Size FROM MovieFiles f JOIN Movies m ON m.Id=f.MovieId",
                    ),
                    "sonarr" => (
                        (
                            "series",
                            "Series",
                            "SELECT Id,TvdbId,Path,Monitored,SeasonFolder,QualityProfileId,Seasons FROM Series ORDER BY Id",
                        ),
                        Some((
                            "episode_files",
                            "EpisodeFiles",
                            "SELECT Id,SeriesId,SeasonNumber,RelativePath,Size FROM EpisodeFiles ORDER BY Id",
                        )),
                        "SELECT s.Path||'/'||f.RelativePath,f.Size FROM EpisodeFiles f JOIN Series s ON s.Id=f.SeriesId",
                    ),
                    "lidarr" => (
                        (
                            "artists",
                            "Artists",
                            "SELECT a.Id,am.ForeignArtistId,a.Path,a.Monitored,a.QualityProfileId,a.MetadataProfileId FROM Artists a LEFT JOIN ArtistMetadata am ON am.Id=a.ArtistMetadataId ORDER BY a.Id",
                        ),
                        Some((
                            "track_files",
                            "TrackFiles",
                            "SELECT Id,AlbumId,Path,Size FROM TrackFiles ORDER BY Id",
                        )),
                        "SELECT Path,Size FROM TrackFiles",
                    ),
                    _ => (
                        (
                            "indexer_definitions",
                            "IndexerDefinitionVersions",
                            "SELECT Id FROM IndexerDefinitionVersions ORDER BY Id",
                        ),
                        None,
                        "",
                    ),
                };
                add(
                    &mut tables,
                    items.0,
                    records(&c, items.1, items.2, kind != "prowlarr")?,
                );
                if let Some((name, table, sql)) = item_sql {
                    add(&mut tables, name, records(&c, table, sql, true)?);
                    let mut statement = c.prepare(file_sql)?;
                    let rows = statement
                        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
                    for row in rows {
                        let (path, size) = row?;
                        files.push(json!([path, size]));
                    }
                }
                if kind == "sonarr" {
                    add(
                        &mut tables,
                        "episodes",
                        records(
                            &c,
                            "Episodes",
                            "SELECT Id,SeriesId,SeasonNumber,EpisodeNumber,EpisodeFileId,Monitored FROM Episodes ORDER BY Id",
                            true,
                        )?,
                    );
                }
                if kind == "lidarr" {
                    add(
                        &mut tables,
                        "albums",
                        records(
                            &c,
                            "Albums",
                            "SELECT Id,ForeignAlbumId,Monitored,ProfileId FROM Albums ORDER BY Id",
                            true,
                        )?,
                    );
                    add(
                        &mut tables,
                        "metadata_profiles",
                        records(
                            &c,
                            "MetadataProfiles",
                            "SELECT Id,Name FROM MetadataProfiles ORDER BY Id",
                            true,
                        )?,
                    );
                }
                if kind == "prowlarr" {
                    add(
                        &mut tables,
                        "app_profiles",
                        records(
                            &c,
                            "AppSyncProfiles",
                            "SELECT Id,Name FROM AppSyncProfiles ORDER BY Id",
                            true,
                        )?,
                    );
                }
            }
            "bazarr" => {
                let text = fs::read_to_string(root.join("config/config.yaml"))
                    .context("Bazarr's config.yaml is missing")?;
                let yaml: Value =
                    serde_yaml_ng::from_str(&text).context("Bazarr's config.yaml isn't valid")?;
                config = json!({"url_base":yaml["general"]["base_url"],"port":yaml["general"]["port"],
                    "authentication":yaml["auth"]["type"].as_str().unwrap_or("none")});
                for peer in ["radarr", "sonarr"] {
                    if yaml["general"][format!("use_{peer}")] == true
                        && let Some(host) = yaml[peer]["ip"].as_str()
                    {
                        remotes.push(
                            json!({"host":host,"port":yaml[peer]["port"].as_u64().unwrap_or(80)}),
                        );
                        peers.push(host.into());
                    }
                }
                tables.insert("configuration".into(), json!({"count":1,"strict":true,"ids":[1],
                    "digest":hex(&Sha256::digest(serde_json::to_vec(&json!([yaml["general"]["enabled_providers"],yaml["auth"]["apikey"]]))?))}));
                let c = open(&root.join("db/bazarr.db"), review)?
                    .context("Bazarr's database is missing")?;
                format = json!({"database":"db/bazarr.db","schema":c.query_row("SELECT version_num FROM alembic_version",[],|r| r.get::<_,String>(0)).ok()});
                add(
                    &mut tables,
                    "languages_profiles",
                    records(
                        &c,
                        "table_languages_profiles",
                        "SELECT profileId,name,items,cutoff FROM table_languages_profiles ORDER BY profileId",
                        true,
                    )?,
                );
                add(
                    &mut tables,
                    "history",
                    records(
                        &c,
                        "table_history",
                        "SELECT id,action,sonarrEpisodeId,subtitles_path FROM table_history ORDER BY id",
                        true,
                    )?,
                );
                add(
                    &mut tables,
                    "history_movies",
                    records(
                        &c,
                        "table_history_movie",
                        "SELECT id,action,radarrId,subtitles_path FROM table_history_movie ORDER BY id",
                        true,
                    )?,
                );
                add(
                    &mut tables,
                    "movies",
                    records(
                        &c,
                        "table_movies",
                        "SELECT radarrId,path FROM table_movies ORDER BY radarrId",
                        false,
                    )?,
                );
                add(
                    &mut tables,
                    "episodes",
                    records(
                        &c,
                        "table_episodes",
                        "SELECT sonarrEpisodeId,path FROM table_episodes ORDER BY sonarrEpisodeId",
                        false,
                    )?,
                );
                for path in strings(&c, "table_movies", "SELECT path FROM table_movies")?
                    .into_iter()
                    .chain(strings(
                        &c,
                        "table_episodes",
                        "SELECT path FROM table_episodes",
                    )?)
                {
                    let size = 0;
                    files.push(json!([path, size]));
                }
            }
            "nzbget" => {
                let text = fs::read_to_string(root.join("nzbget.conf"))
                    .context("nzbget.conf is missing")?;
                let values: Vec<(String, String)> = text
                    .lines()
                    .filter(|l| !l.trim_start().starts_with('#'))
                    .filter_map(|l| l.split_once('='))
                    .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
                    .collect();
                let get = |key: &str| {
                    values
                        .iter()
                        .rev()
                        .find(|(k, _)| k.eq_ignore_ascii_case(key))
                        .map(|(_, v)| v.clone())
                };
                let main = get("MainDir").unwrap_or_else(|| "/config".into());
                let resolve = |value: &str| {
                    value
                        .replace("${MainDir}", &main)
                        .replace("${AppDir}", "/app/nzbget")
                };
                config = json!({"port":get("ControlPort").and_then(|p| p.parse::<u16>().ok()),"address":get("ControlIP"),
                    "username":get("ControlUsername"),"secure":get("SecureControl")});
                for (key, kind, access) in [
                    ("MainDir", "dir", "rw"),
                    ("DestDir", "dir", "rw"),
                    ("InterDir", "dir", "rw"),
                    ("NzbDir", "dir", "rw"),
                    ("QueueDir", "dir", "rw"),
                    ("TempDir", "dir", "rw"),
                    ("ScriptDir", "dir", "r"),
                    ("SecureCert", "file", "r"),
                    ("SecureKey", "file", "r"),
                ] {
                    let Some(path) = get(key).map(|v| resolve(&v)).filter(|v| !v.is_empty()) else {
                        continue;
                    };
                    if key == "QueueDir" && !within(&path, "/config") {
                        blockers.push(format!(
                            "NZBGet keeps its queue and history at {path}, outside its configuration folder. Set QueueDir to ${{MainDir}}/queue with MainDir inside /config, then review again"
                        ));
                    }
                    if key.starts_with("Secure") && get("SecureControl").as_deref() != Some("yes") {
                        continue;
                    }
                    // NZBGet creates its writable directories on demand.
                    references.push(json!({"path":path,"kind":kind,"access":access,"label":format!("NZBGet {key}"),
                        "create":kind == "dir" && access == "rw"}));
                }
                let mut categories = Vec::new();
                let mut servers = Vec::new();
                for (key, value) in &values {
                    if let Some(rest) = key.strip_prefix("Category") {
                        categories.push(json!([key, value]));
                        if rest.ends_with(".DestDir") && !value.is_empty() {
                            references.push(json!({"path":resolve(value),"kind":"dir","access":"rw","label":"NZBGet category destination","create":true}));
                        }
                    }
                    if let Some(rest) = key.strip_prefix("Server") {
                        let secret = rest.ends_with(".Password") || rest.ends_with(".Username");
                        servers.push(json!([
                            key,
                            if secret {
                                json!(hex(&Sha256::digest(value.as_bytes())))
                            } else {
                                json!(value)
                            }
                        ]));
                        if rest.ends_with(".Host") {
                            let port = get(&key.replace(".Host", ".Port"))
                                .and_then(|p| p.parse::<u16>().ok())
                                .unwrap_or(563);
                            remotes.push(json!({"host":value,"port":port}));
                        }
                    }
                }
                let digest = |items: &Vec<Value>| {
                    hex(&Sha256::digest(
                        serde_json::to_vec(items).unwrap_or_default(),
                    ))
                };
                tables.insert("categories".into(), json!({"count":categories.len(),"digest":digest(&categories),"strict":true,"ids":categories.iter().map(|c| c[0].clone()).collect::<Vec<_>>()}));
                tables.insert("servers".into(), json!({"count":servers.len(),"digest":digest(&servers),"strict":true,"ids":servers.iter().map(|c| c[0].clone()).collect::<Vec<_>>()}));
                let queue = PathBuf::from(resolve(
                    &get("QueueDir").unwrap_or_else(|| "${MainDir}/queue".into()),
                ));
                let local = queue
                    .strip_prefix("/config")
                    .map(|q| root.join(q))
                    .unwrap_or_default();
                let version = fs::read_to_string(local.join("queue"))
                    .ok()
                    .and_then(|t| t.lines().next().map(str::to_owned));
                format = json!({"configuration":"nzbget.conf","queue":version});
            }
            "seerr" => {
                let settings: Value = serde_json::from_slice(
                    &fs::read(root.join("settings.json"))
                        .context("Seerr's settings.json is missing")?,
                )?;
                config = json!({"sign_in":if settings["main"]["localLogin"]==false && settings["main"]["mediaServerLogin"]==false {"Thelxinoe accounts only"} else {"Seerr and media-server accounts"},
                    "availability":match settings["main"]["mediaServerType"].as_u64() {Some(1)=>"Plex",Some(2)=>"Jellyfin",Some(3)=>"Emby",_=>"Radarr and Sonarr"},
                    "directory":"/app/config"});
                for kind in ["radarr", "sonarr"] {
                    for server in settings[kind].as_array().into_iter().flatten() {
                        if let Some(host) = server["hostname"].as_str() {
                            remotes.push(
                                json!({"host":host,"port":server["port"].as_u64().unwrap_or(80)}),
                            );
                            peers.push(host.into());
                        }
                    }
                }
                endpoints(&settings["notifications"], &mut remotes, &mut Vec::new());
                tables.insert("settings".into(), json!({"count":1,"strict":true,"ids":[1],
                    "digest":hex(&Sha256::digest(serde_json::to_vec(&json!([settings["notifications"],settings["radarr"],settings["sonarr"]]))?))}));
                let c = open(&root.join("db/db.sqlite3"), review)?
                    .context("Seerr's database is missing")?;
                format = json!({"database":"db/db.sqlite3","schema":c.query_row("SELECT name FROM migrations ORDER BY id DESC LIMIT 1",[],|r| r.get::<_,String>(0)).ok()});
                add(
                    &mut tables,
                    "users",
                    records(
                        &c,
                        "user",
                        "SELECT id,email,username,userType FROM user ORDER BY id",
                        true,
                    )?,
                );
                add(
                    &mut tables,
                    "requests",
                    records(
                        &c,
                        "media_request",
                        "SELECT id,type,mediaId,requestedById,is4k,serverId,profileId,rootFolder FROM media_request ORDER BY id",
                        true,
                    )?,
                );
                add(
                    &mut tables,
                    "season_requests",
                    records(
                        &c,
                        "season_request",
                        "SELECT id,seasonNumber,requestId FROM season_request ORDER BY id",
                        true,
                    )?,
                );
                add(
                    &mut tables,
                    "media",
                    records(
                        &c,
                        "media",
                        "SELECT id,mediaType,tmdbId FROM media ORDER BY id",
                        true,
                    )?,
                );
            }
            "recyclarr" => {
                let mut documents = Vec::new();
                for name in ["recyclarr.yml", "settings.yml"] {
                    if let Ok(text) = fs::read_to_string(root.join(name)) {
                        documents.push((name.to_owned(), text));
                    }
                }
                if let Ok(entries) = fs::read_dir(root.join("configs")) {
                    let mut entries = entries
                        .filter_map(|e| e.ok())
                        .map(|e| e.path())
                        .collect::<Vec<_>>();
                    entries.sort();
                    for path in entries {
                        if path.extension().is_some_and(|e| e == "yml" || e == "yaml") {
                            documents.push((
                                format!(
                                    "configs/{}",
                                    path.file_name().unwrap_or_default().to_string_lossy()
                                ),
                                fs::read_to_string(&path)?,
                            ));
                        }
                    }
                }
                anyhow::ensure!(
                    documents.iter().any(|(n, _)| n != "settings.yml"),
                    "Recyclarr has no configuration to import"
                );
                for (_, text) in &documents {
                    let yaml: Value = serde_yaml_ng::from_str(text)
                        .context("Recyclarr configuration isn't valid YAML")?;
                    for kind in ["radarr", "sonarr"] {
                        for (_, instance) in yaml[kind].as_object().into_iter().flatten() {
                            if let Some(url) = instance["base_url"]
                                .as_str()
                                .and_then(|u| reqwest::Url::parse(u).ok())
                                && let Some(host) = url.host_str()
                            {
                                remotes.push(json!({"host":host,"port":url.port_or_known_default().unwrap_or(80)}));
                                peers.push(host.into());
                            }
                        }
                    }
                }
                let names: Vec<Value> = documents.iter().map(|(n, _)| json!(n)).collect();
                tables.insert("configuration".into(), json!({"count":documents.len(),"strict":true,"ids":names,
                    "digest":hex(&Sha256::digest(serde_json::to_vec(&documents.iter().map(|(n,t)| json!([n,t])).collect::<Vec<_>>())?))}));
                let state =
                    tree(&root.join("state")).unwrap_or_else(|_| json!({"digest":null,"files":0}));
                tables.insert("state".into(), json!({"count":state["files"],"strict":false,"ids":[],"digest":state["digest"]}));
                format = json!({"configuration":"recyclarr.yml","files":names});
            }
            _ => anyhow::bail!("Unsupported service"),
        }
        peers.sort();
        peers.dedup();
        report.insert("kind".into(), json!(kind));
        report.insert("format".into(), format);
        report.insert("config".into(), config);
        report.insert("records".into(), Value::Object(tables));
        report.insert("references".into(), json!(references));
        report.insert("remotes".into(), json!(remotes));
        report.insert("peers".into(), json!(peers));
        report.insert("files".into(), json!(files));
        report.insert("blockers".into(), json!(blockers));
        report.insert("counts".into(), json!(files.len()));
        Ok(Value::Object(report))
    })
}

fn within(path: &str, root: &str) -> bool {
    path == root || path.starts_with(&format!("{}/", root.trim_end_matches('/')))
}

// ---------- Transformations ----------

fn set_xml(text: &str, field: &str, value: &str) -> String {
    let open = format!("<{field}>");
    let close = format!("</{field}>");
    if let Some((head, tail)) = text.split_once(&open)
        && let Some((_, rest)) = tail.split_once(&close)
    {
        return format!("{head}{open}{value}{close}{rest}");
    }
    let empty = format!("<{field} />");
    if text.contains(&empty) {
        return text.replacen(&empty, &format!("{open}{value}{close}"), 1);
    }
    text.replacen(
        "</Config>",
        &format!("  {open}{value}{close}\n</Config>"),
        1,
    )
}

/// Apply the reviewed changes to a copied configuration and give it to the target identity.
pub fn prepare(kind: &str, mode: &str) -> anyhow::Result<()> {
    reported(|| {
        let root = Path::new("/destination");
        anyhow::ensure!(root.is_dir(), "Destination mount missing");
        let plan: Value = serde_json::from_str(&std::env::var("THELXINOE_TRANSFER_PLAN")?)?;
        let mut applied = Vec::new();
        match kind {
            "radarr" | "sonarr" | "lidarr" | "prowlarr" => {
                let path = root.join("config.xml");
                let mut text = fs::read_to_string(&path).context("config.xml is missing")?;
                let port = plan["port"].as_u64().context("Missing port")?.to_string();
                for (field, value) in [
                    ("UrlBase", plan["url_base"].as_str().unwrap_or("")),
                    ("Port", port.as_str()),
                    ("BindAddress", "*"),
                    ("AuthenticationMethod", "External"),
                    ("AuthenticationRequired", "Enabled"),
                    ("EnableSsl", "False"),
                    ("LaunchBrowser", "False"),
                    ("UpdateAutomatically", "False"),
                ] {
                    text = set_xml(&text, field, value);
                    applied.push(field);
                }
                // A restricted host list must also admit the replacement's managed names.
                if let Some(hosts) = plan["allowed_hosts"].as_str() {
                    text = set_xml(&text, "AllowedHosts", hosts);
                    applied.push("AllowedHosts");
                }
                fs::write(&path, text)?;
            }
            "bazarr" => {
                let path = root.join("config/config.yaml");
                let mut yaml: serde_yaml_ng::Value =
                    serde_yaml_ng::from_str(&fs::read_to_string(&path)?)?;
                let general = yaml
                    .get_mut("general")
                    .and_then(|g| g.as_mapping_mut())
                    .context("Bazarr's general settings are missing")?;
                general.insert(
                    "base_url".into(),
                    plan["url_base"].as_str().unwrap_or("").into(),
                );
                general.insert("port".into(), plan["port"].as_u64().unwrap_or(6767).into());
                general.insert("ip".into(), "0.0.0.0".into());
                if let Some(auth) = yaml.get_mut("auth").and_then(|a| a.as_mapping_mut()) {
                    auth.insert("type".into(), serde_yaml_ng::Value::Null);
                }
                fs::write(&path, serde_yaml_ng::to_string(&yaml)?)?;
                applied.extend(["base_url", "port", "ip", "auth.type"]);
            }
            "nzbget" => {
                let path = root.join("nzbget.conf");
                let text = fs::read_to_string(&path)?;
                let port = plan["port"].as_u64().unwrap_or(6789);
                let mut seen = (false, false);
                let mut lines: Vec<String> = text
                    .lines()
                    .map(|line| match line.split_once('=') {
                        Some((key, _)) if key.trim().eq_ignore_ascii_case("ControlPort") => {
                            seen.0 = true;
                            format!("ControlPort={port}")
                        }
                        Some((key, _)) if key.trim().eq_ignore_ascii_case("ControlIP") => {
                            seen.1 = true;
                            "ControlIP=0.0.0.0".into()
                        }
                        _ => line.to_owned(),
                    })
                    .collect();
                if !seen.0 {
                    lines.push(format!("ControlPort={port}"));
                }
                if !seen.1 {
                    lines.push("ControlIP=0.0.0.0".into());
                }
                fs::write(&path, lines.join("\n") + "\n")?;
                applied.extend(["ControlPort", "ControlIP"]);
            }
            "seerr" => {}
            "recyclarr" => {
                let metadata = root.join(".thelxinoe");
                if metadata.exists() {
                    fs::remove_dir_all(&metadata)?;
                }
                let fixture = std::env::var("THELXINOE_TRANSFER_FIXTURE_KEY").ok();
                let mut port = 10000u16;
                let mut fixtures = Vec::new();
                for path in recyclarr_documents(root)? {
                    let mut config: serde_yaml_ng::Value =
                        serde_yaml_ng::from_str(&fs::read_to_string(&path)?)?;
                    for kind in ["radarr", "sonarr"] {
                        for (name, instance) in config
                            .get_mut(kind)
                            .and_then(|v| v.as_mapping_mut())
                            .into_iter()
                            .flatten()
                        {
                            let Some(map) = instance.as_mapping_mut() else {
                                continue;
                            };
                            if mode == "validation" {
                                // Validation targets are loopback fixtures; production keys are never used.
                                map.insert(
                                    "base_url".into(),
                                    format!("http://127.0.0.1:{port}").into(),
                                );
                                map.insert(
                                    "api_key".into(),
                                    fixture.clone().unwrap_or_default().into(),
                                );
                                fixtures.push(
                                    json!({"kind":kind,"instance":name.as_str(),"port":port}),
                                );
                                port += 1;
                            } else {
                                map.insert("api_key".into(), "REDACTED_REGENERATED_AT_SYNC".into());
                            }
                        }
                    }
                    fs::write(&path, serde_yaml_ng::to_string(&config)?)?;
                }
                applied.push("targets");
                if mode == "validation" {
                    if let Ok(settings) = std::env::var("THELXINOE_RECYCLARR_SETTINGS") {
                        fs::write(root.join("settings.yml"), settings)?;
                    }
                    let ids = plan["identity"].as_array().context("Missing identity")?;
                    chown_tree(
                        root,
                        ids[0].as_u64().unwrap_or(0) as u32,
                        ids[1].as_u64().unwrap_or(0) as u32,
                    )?;
                    return Ok(json!({"applied":applied,"fixtures":fixtures}));
                }
            }
            _ => anyhow::bail!("Unsupported service"),
        }
        let ids = plan["identity"].as_array().context("Missing identity")?;
        let (uid, gid) = (
            ids[0].as_u64().unwrap_or(0) as u32,
            ids[1].as_u64().unwrap_or(0) as u32,
        );
        anyhow::ensure!(
            uid > 0 && gid > 0,
            "Transfers require a non-root target identity"
        );
        chown_tree(root, uid, gid)?;
        Ok(json!({"applied":applied,"mode":mode}))
    })
}

pub fn recyclarr_documents(root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut documents = vec![];
    if root.join("recyclarr.yml").is_file() {
        documents.push(root.join("recyclarr.yml"));
    }
    if let Ok(entries) = fs::read_dir(root.join("configs")) {
        let mut entries: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        entries.sort();
        documents.extend(
            entries
                .into_iter()
                .filter(|p| p.extension().is_some_and(|e| e == "yml" || e == "yaml")),
        );
    }
    Ok(documents)
}

/// Only the copied configuration gets the target identity; links are never followed.
fn chown_tree(path: &Path, uid: u32, gid: u32) -> anyhow::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        return Ok(());
    }
    std::os::unix::fs::lchown(path, Some(uid), Some(gid))?;
    if meta.is_dir() {
        for entry in fs::read_dir(path)? {
            chown_tree(&entry?.path(), uid, gid)?;
        }
    }
    Ok(())
}

// ---------- Validation fixtures ----------

/// Synthetic media: sparse representations of every imported file at its own path.
pub fn fixture() -> anyhow::Result<()> {
    reported(|| {
        let input = input()?;
        let destinations: Vec<String> = serde_json::from_value(input["destinations"].clone())?;
        let (uid, gid) = (
            input["uid"].as_u64().unwrap_or(0) as u32,
            input["gid"].as_u64().unwrap_or(0) as u32,
        );
        let root = Path::new("/fixture");
        for index in 0..destinations.len() {
            fs::create_dir_all(root.join(index.to_string()))?;
        }
        let mut created = 0u64;
        for file in input["files"].as_array().into_iter().flatten() {
            let Some(path) = file[0].as_str() else {
                continue;
            };
            let size = file[1].as_u64().unwrap_or(0);
            let Some((index, destination)) = destinations
                .iter()
                .enumerate()
                .filter(|(_, d)| within(path, d))
                .max_by_key(|(_, d)| d.len())
            else {
                continue;
            };
            let relative = Path::new(path).strip_prefix(destination)?;
            anyhow::ensure!(
                relative
                    .components()
                    .all(|c| matches!(c, Component::Normal(_))),
                "Invalid imported path"
            );
            let target = root.join(index.to_string()).join(relative);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            if !target.exists() {
                fs::File::create(&target)?.set_len(size)?;
                created += 1;
            }
        }
        for index in 0..destinations.len() {
            chown_tree(&root.join(index.to_string()), uid, gid)?;
        }
        Ok(json!({"representations":created}))
    })
}

// ---------- Target-identity access ----------

/// Whether the kernel grants this process `mode` on `path`, ACLs included (a Synology share
/// can grant everything through ACLs while its mode bits grant nothing). Every mount is
/// read-only here, so a write the permissions allow reports a read-only file system.
fn permits(path: &Path, mode: libc::c_int) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: `path` is a NUL-terminated string that outlives the call.
    let granted = unsafe { libc::access(path.as_ptr(), mode) } == 0;
    granted || std::io::Error::last_os_error().raw_os_error() == Some(libc::EROFS)
}

/// Media and download access for the target identity, computed without writing anything.
/// The controller runs this worker as that identity.
pub fn access() -> anyhow::Result<()> {
    reported(|| {
        // SAFETY: geteuid has no preconditions.
        let uid = unsafe { libc::geteuid() };
        // Root passes every check, so its answers would be meaningless.
        anyhow::ensure!(uid != 0, "The access check must run as the target identity");
        let input = input()?;
        let mut problems = Vec::new();
        let mut missing = Vec::new();
        let mut roots = Vec::new();
        let mut checked = 0u64;
        for reference in input["references"].as_array().into_iter().flatten() {
            let Some(path) = reference["path"].as_str() else {
                continue;
            };
            let label = reference["label"].as_str().unwrap_or("Path");
            if within(path, "/config") {
                continue;
            }
            checked += 1;
            let mut target = Path::new(path);
            let mut found = fs::metadata(target);
            let denied = |found: &std::io::Result<fs::Metadata>| {
                found
                    .as_ref()
                    .is_err_and(|e| e.kind() == std::io::ErrorKind::PermissionDenied)
            };
            // A directory created on demand only needs a writable existing parent.
            if reference["create"] == true && found.is_err() && !denied(&found) {
                match target.ancestors().skip(1).find(|a| a.is_dir()) {
                    Some(parent) if parent != Path::new("/") => {
                        target = parent;
                        found = fs::metadata(target);
                    }
                    _ => {
                        missing.push(format!("{label} {path}"));
                        continue;
                    }
                }
            }
            // Resolving a path needs every ancestor searchable by the target identity.
            if denied(&found) {
                let mut ancestors = target.ancestors().skip(1).collect::<Vec<_>>();
                ancestors.reverse();
                let blocked = ancestors
                    .into_iter()
                    .find(|a| !permits(a, libc::X_OK))
                    .unwrap_or(target);
                problems.push(format!(
                    "{label} {} isn't reachable by {uid}: {} doesn't allow it to enter",
                    target.display(),
                    blocked.display()
                ));
                continue;
            }
            let Ok(meta) = found else {
                missing.push(format!("{label} {path}"));
                continue;
            };
            let want = match reference["access"].as_str() {
                Some("rw") if meta.is_dir() => libc::R_OK | libc::W_OK | libc::X_OK,
                Some("rw") => libc::R_OK | libc::W_OK,
                Some("x") => libc::R_OK | libc::X_OK,
                _ if meta.is_dir() => libc::R_OK | libc::X_OK,
                _ => libc::R_OK,
            };
            if !permits(target, want) {
                problems.push(format!(
                    "{label} {} isn't {} by the target identity {uid}; adjust its permissions without moving the files",
                    target.display(),
                    if want & libc::W_OK != 0 { "writable" } else { "readable" },
                ));
            }
            if reference["kind"] == "dir" {
                roots.push(json!({"path":path,"device":meta.dev(),"inode":meta.ino()}));
            }
        }
        if !missing.is_empty() {
            problems.insert(0, format!(
                "{} referenced paths don't resolve to existing files, for example {}. Fix or rescan them in the original service, then review again",
                missing.len(),
                missing.iter().take(3).cloned().collect::<Vec<_>>().join(", ")
            ));
        }
        let supplementary: Vec<Value> = input["supplementary"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(|destination| json!({"destination":destination,"entry":entry(Path::new(destination))}))
            .collect();
        Ok(
            json!({"checked":checked,"problems":problems,"roots":roots,"supplementary":supplementary}),
        )
    })
}

/// What a read-only supplementary mount holds: a folder, one regular file, or neither.
fn entry(path: &Path) -> &'static str {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => "directory",
        Ok(meta) if meta.is_file() => "file",
        _ => "unsupported",
    }
}

// ---------- Digests ----------

fn tree(root: &Path) -> anyhow::Result<Value> {
    let mut hasher = Sha256::new();
    let mut files = 0u64;
    let mut bytes = 0u64;
    let mut pending = vec![root.to_path_buf()];
    let mut entries = Vec::new();
    while let Some(directory) = pending.pop() {
        let mut children: Vec<PathBuf> = fs::read_dir(&directory)?
            .map(|e| e.map(|e| e.path()))
            .collect::<std::io::Result<_>>()?;
        children.sort();
        for child in children {
            let meta = fs::symlink_metadata(&child)?;
            if meta.is_dir() {
                pending.push(child.clone());
            }
            entries.push((child, meta));
        }
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    for (path, meta) in entries {
        let relative = path.strip_prefix(root)?.to_string_lossy().into_owned();
        // Copies materialize sibling links; digest their content the same way.
        let (meta, content) = if meta.file_type().is_symlink() {
            let target = path
                .parent()
                .context("Missing link parent")?
                .join(fs::read_link(&path)?);
            (fs::metadata(&target)?, Some(target))
        } else {
            (meta.clone(), meta.is_file().then(|| path.clone()))
        };
        // Copies keep only the permission bits (state_copy `owner`), so special bits such as
        // an inherited setgid must not make a faithful copy look incomplete.
        hasher.update(format!(
            "{relative}\0{}\0{}\0{}\0{}\0",
            meta.is_dir(),
            meta.mode() & 0o777,
            meta.uid(),
            meta.gid()
        ));
        if let Some(content) = content {
            let mut file = fs::File::open(content)?;
            let mut buffer = vec![0; 1 << 20];
            loop {
                let read = file.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                hasher.update(&buffer[..read]);
                bytes += read as u64;
            }
            files += 1;
            if files.is_multiple_of(200) {
                write(
                    "progress.json",
                    &json!({"phase":"digest","files":files,"bytes":bytes}),
                );
            }
        }
    }
    Ok(json!({"digest":hex(&hasher.finalize()),"files":files,"bytes":bytes}))
}

pub fn digest() -> anyhow::Result<()> {
    reported(|| tree(Path::new("/state")))
}

// ---------- Live effects after the production boundary ----------

pub fn effects(kind: &str) -> anyhow::Result<()> {
    reported(|| {
        let boundary: i64 = std::env::var("THELXINOE_TRANSFER_BOUNDARY")?.parse()?;
        let since = chrono_like(boundary);
        let root = Path::new("/state");
        let mut known = Vec::new();
        let mut uncertain = vec![
            "Notifications, webhooks and indexer queries are not recorded locally and may have been sent".to_owned(),
        ];
        let count = |c: &Connection, table: &str, sql: &str| -> i64 {
            if !exists(c, table) {
                return 0;
            }
            c.query_row(sql, [&since], |r| r.get::<_, i64>(0))
                .unwrap_or(0)
        };
        let database = match kind {
            "radarr" | "sonarr" | "lidarr" | "prowlarr" => Some(format!("{kind}.db")),
            "bazarr" => Some("db/bazarr.db".into()),
            "seerr" => Some("db/db.sqlite3".into()),
            _ => None,
        };
        match database.and_then(|d| open(&root.join(d), true).ok().flatten()) {
            Some(c) => {
                let checks: &[(&str, &str, &str)] = match kind {
                    "radarr" => &[
                        (
                            "History",
                            "SELECT COUNT(*) FROM History WHERE Date>?1",
                            "download, import or grab events",
                        ),
                        (
                            "MovieFiles",
                            "SELECT COUNT(*) FROM MovieFiles WHERE DateAdded>?1",
                            "imported movie files",
                        ),
                        (
                            "Movies",
                            "SELECT COUNT(*) FROM Movies WHERE Added>?1",
                            "movies added",
                        ),
                    ],
                    "sonarr" => &[
                        (
                            "History",
                            "SELECT COUNT(*) FROM History WHERE Date>?1",
                            "download, import or grab events",
                        ),
                        (
                            "EpisodeFiles",
                            "SELECT COUNT(*) FROM EpisodeFiles WHERE DateAdded>?1",
                            "imported episode files",
                        ),
                        (
                            "Series",
                            "SELECT COUNT(*) FROM Series WHERE Added>?1",
                            "series added",
                        ),
                    ],
                    "lidarr" => &[
                        (
                            "History",
                            "SELECT COUNT(*) FROM History WHERE Date>?1",
                            "download, import or grab events",
                        ),
                        (
                            "TrackFiles",
                            "SELECT COUNT(*) FROM TrackFiles WHERE DateAdded>?1",
                            "imported track files",
                        ),
                    ],
                    "prowlarr" => &[(
                        "History",
                        "SELECT COUNT(*) FROM History WHERE Date>?1",
                        "indexer searches or grabs",
                    )],
                    "bazarr" => &[
                        (
                            "table_history",
                            "SELECT COUNT(*) FROM table_history WHERE timestamp>?1",
                            "episode subtitle changes",
                        ),
                        (
                            "table_history_movie",
                            "SELECT COUNT(*) FROM table_history_movie WHERE timestamp>?1",
                            "movie subtitle changes",
                        ),
                    ],
                    "seerr" => &[(
                        "media_request",
                        "SELECT COUNT(*) FROM media_request WHERE createdAt>?1",
                        "requests submitted",
                    )],
                    _ => &[],
                };
                for (table, sql, label) in checks {
                    let n = count(&c, table, sql);
                    if n > 0 {
                        known.push(json!({"effect":label,"count":n}));
                    }
                }
            }
            None => uncertain.push(format!(
                "{kind} activity since activation couldn't be read from its state"
            )),
        }
        if kind == "nzbget" {
            uncertain.push("NZBGet downloads and post-processing since activation must be reviewed in its history".into());
        }
        Ok(json!({"since":boundary,"known":known,"uncertain":uncertain}))
    })
}

/// ISO-8601 UTC text, matching how the applications store dates.
fn chrono_like(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    // Civil-from-days (Howard Hinnant), valid for the proleptic Gregorian calendar.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_settings_are_replaced_or_added_without_touching_others() {
        let text =
            "<Config>\n  <UrlBase></UrlBase>\n  <ApiKey>keep</ApiKey>\n  <EnableSsl />\n</Config>";
        let text = set_xml(text, "UrlBase", "/services/radarr");
        let text = set_xml(&text, "EnableSsl", "False");
        let text = set_xml(&text, "LaunchBrowser", "False");
        assert!(text.contains("<UrlBase>/services/radarr</UrlBase>"));
        assert!(text.contains("<ApiKey>keep</ApiKey>"));
        assert!(text.contains("<EnableSsl>False</EnableSsl>"));
        assert!(text.contains("<LaunchBrowser>False</LaunchBrowser>\n</Config>"));
    }

    #[test]
    fn effect_boundaries_use_application_date_text() {
        assert_eq!(chrono_like(0), "1970-01-01 00:00:00");
        assert_eq!(chrono_like(1_790_000_000), "2026-09-21 14:13:20");
    }

    #[test]
    fn settings_documents_yield_hosts_without_secrets() {
        let mut remotes = Vec::new();
        let mut peers = Vec::new();
        endpoints(
            &json!({"host":"nzbget","port":6789,"password":"secret","nested":{"baseUrl":"https://indexer.example:8443/api"}}),
            &mut remotes,
            &mut peers,
        );
        assert!(remotes.contains(&json!({"host":"nzbget","port":6789})));
        assert!(remotes.contains(&json!({"host":"indexer.example","port":8443})));
        assert!(!serde_json::to_string(&remotes).unwrap().contains("secret"));
    }

    #[test]
    fn records_report_identities_and_detect_changes() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE Movies(Id INTEGER, Path TEXT); INSERT INTO Movies VALUES (1,'/movies/a'),(2,'/movies/b');").unwrap();
        let before = records(&c, "Movies", "SELECT Id,Path FROM Movies ORDER BY Id", true)
            .unwrap()
            .unwrap();
        assert_eq!(before["ids"], json!([1, 2]));
        c.execute("UPDATE Movies SET Path='/movies/c' WHERE Id=2", [])
            .unwrap();
        let after = records(&c, "Movies", "SELECT Id,Path FROM Movies ORDER BY Id", true)
            .unwrap()
            .unwrap();
        assert_ne!(before["digest"], after["digest"]);
        assert!(records(&c, "Missing", "SELECT 1", true).unwrap().is_none());
    }

    #[test]
    fn digests_ignore_the_special_bits_copies_drop() {
        use std::os::unix::fs::PermissionsExt;
        let source = tempfile::tempdir().unwrap();
        let copy = tempfile::tempdir().unwrap();
        for (root, mode) in [(source.path(), 0o2775), (copy.path(), 0o775)] {
            let data = root.join("data");
            fs::create_dir(&data).unwrap();
            fs::write(data.join("db"), "state").unwrap();
            fs::set_permissions(&data, fs::Permissions::from_mode(mode)).unwrap();
        }
        assert_eq!(
            tree(source.path()).unwrap()["digest"],
            tree(copy.path()).unwrap()["digest"]
        );
    }

    #[test]
    fn access_is_the_kernel_decision() {
        let directory = tempfile::tempdir().unwrap();
        use std::os::unix::fs::PermissionsExt;
        let file = directory.path().join("movie.mkv");
        fs::write(&file, "x").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o640)).unwrap();
        assert!(permits(&file, libc::R_OK | libc::W_OK));
        assert!(!permits(&directory.path().join("missing"), libc::R_OK));
    }
}
