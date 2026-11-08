use super::*;

const HELD: &str = "11111111-1111-4111-8111-111111111111";
const MISSING: &str = "22222222-2222-4222-8222-222222222222";
const ASKED: &str = "33333333-3333-4333-8333-333333333333";

#[test]
fn trending_lists_each_release_group_once() {
    let body = json!({"payload":{"release_groups":[
        {"release_group_mbid":HELD,"release_group_name":"First","artist_name":"Band"},
        {"release_group_mbid":HELD,"release_group_name":"First","artist_name":"Band"},
        {"release_group_mbid":"not-an-id","release_group_name":"Broken"},
        {"release_group_mbid":MISSING,"release_group_name":"Second","artist_name":null},
    ]}});
    let items = trending(&body);
    assert_eq!(
        items
            .iter()
            .map(|item| item["title"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["First", "Second"]
    );
    assert_eq!(items[0]["artist"], "Band");
    assert_eq!(items[0]["artist_id"], Value::Null);
    assert_eq!(
        items[0]["cover"],
        format!("/api/v1/acquisition/music/covers/{HELD}")
    );
}

const ARTIST: &str = "44444444-4444-4444-8444-444444444444";
const TODAY: &str = "2026-11-08T00:00:00Z";

fn standard() -> Profile {
    Profile::parse(&json!({
        "name": "Standard",
        "primaryAlbumTypes": [
            {"albumType":{"name":"Album"},"allowed":true},
            {"albumType":{"name":"EP"},"allowed":false},
            {"albumType":{"name":"Single"},"allowed":false},
        ],
        "secondaryAlbumTypes": [
            {"albumType":{"name":"Studio"},"allowed":true},
            {"albumType":{"name":"Live"},"allowed":false},
            {"albumType":{"name":"Soundtrack"},"allowed":true},
        ],
    }))
}

#[test]
fn a_profile_keeps_allowed_primary_types_with_an_allowed_secondary_type() {
    let profile = standard();
    assert_eq!(profile.name, "Standard");
    assert!(
        profile.keeps("Album", &[]),
        "No secondary type is a studio album"
    );
    assert!(profile.keeps("album", &["Soundtrack".into()]));
    assert!(!profile.keeps("Album", &["Live".into()]));
    assert!(profile.keeps("Album", &["Soundtrack".into(), "Live".into()]));
    assert!(!profile.keeps("EP", &[]));
}

#[test]
fn a_discography_splits_what_lidarr_keeps_newest_first_with_what_lidarr_holds() {
    let groups = [
        json!({"id":HELD,"title":"Debut","primary-type":"Album","secondary-types":[],"first-release-date":"2001-05-01"}),
        json!({"id":MISSING,"title":"Later","primary-type":"Album","secondary-types":[],"first-release-date":"2010"}),
        json!({"id":ASKED,"title":"On Stage","primary-type":"Album","secondary-types":["Live"],"first-release-date":"2005-01-01"}),
        json!({"id":"not-an-id","title":"Broken","primary-type":"Album"}),
    ];
    // Lidarr holds the live album and one MusicBrainz no longer lists by default.
    let lidarr = [
        json!({"foreignAlbumId":ASKED,"title":"On Stage","albumType":"Album","secondaryTypes":["Live"],"releaseDate":"2005-01-01T00:00:00Z","monitored":true,"statistics":{"trackFileCount":0,"totalTrackCount":14}}),
        json!({"foreignAlbumId":ARTIST,"title":"Rarities","albumType":"Album","secondaryTypes":[],"releaseDate":"2030-01-01T00:00:00Z","monitored":true,"statistics":{"trackFileCount":0,"totalTrackCount":3}}),
    ];
    let (kept, left) = discography(&groups, &lidarr, &standard(), ("Band", ARTIST), TODAY);
    assert_eq!(
        kept.iter()
            .map(|album| album["title"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["Rarities", "Later", "On Stage", "Debut"]
    );
    assert!(left.is_empty());
    let live = &kept[2];
    assert_eq!(live["type"], "Live");
    assert_eq!(live["track_count"], 14);
    assert_eq!(live["missing"], true);
    assert_eq!(
        kept[0]["missing"], false,
        "An unreleased album is not missing yet"
    );
    assert_eq!(kept[3]["artist_id"], ARTIST);
    assert_eq!(kept[3]["date"], "2001-05-01");
    assert_eq!(kept[1]["year"], 2010);

    let (kept, left) = discography(&groups, &[], &standard(), ("Band", ARTIST), TODAY);
    assert_eq!(kept.len(), 2);
    assert_eq!(left[0]["title"], "On Stage");
    assert_eq!(left[0]["requestable"], false);
}

#[test]
fn missing_albums_are_released_without_files_newest_first() {
    let rows = serde_json::from_value::<Vec<HeldAlbum>>(json!([
        {"foreignAlbumId":HELD,"title":"Older","releaseDate":"2008-01-01T00:00:00Z","monitored":false,"artist":{"artistName":"Band","foreignArtistId":ARTIST},"statistics":{"trackFileCount":0,"totalTrackCount":10}},
        {"foreignAlbumId":MISSING,"title":"Newer","releaseDate":"2012-01-01T00:00:00Z","monitored":true,"artist":{"artistName":"Band","foreignArtistId":ARTIST}},
        {"foreignAlbumId":ASKED,"title":"Owned","releaseDate":"2014-01-01T00:00:00Z","statistics":{"trackFileCount":3,"totalTrackCount":10}},
        {"foreignAlbumId":ARTIST,"title":"Announced","releaseDate":"2030-01-01T00:00:00Z"},
    ]))
    .unwrap();
    let items = missing(rows, TODAY);
    assert_eq!(
        items
            .iter()
            .map(|item| item["title"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["Newer", "Older"]
    );
    assert_eq!(items[0]["missing"], true);
    assert_eq!(items[1]["missing"], false);
    assert_eq!(items[1]["track_count"], 10);
    assert_eq!(items[1]["artist_id"], ARTIST);
}

#[test]
fn similar_artists_leave_out_lidarrs_and_take_each_ones_top_album() {
    let body = json!([
        {"artist_mbid":HELD,"name":"Held"},
        {"artist_mbid":"broken"},
        {"artist_mbid":MISSING,"name":"New"},
    ]);
    assert_eq!(similar_artists(&body, &HashSet::from([HELD])), [MISSING]);
    let top = json!([
        {"release_group_mbid":HELD,"release_group":{"name":"Hit","type":"Single","date":"2020-01-01"},"artist":{"name":"New"}},
        {"release_group_mbid":ASKED,"release_group":{"name":"Record","type":"Album","date":"2019-04-02"},
         "artist":{"name":"New","artists":[{"artist_mbid":MISSING}]}},
    ]);
    let album = top_album(&top).unwrap();
    assert_eq!(album["title"], "Record");
    assert_eq!(album["year"], 2019);
    assert_eq!(album["artist_id"], MISSING);
    assert!(top_album(&json!([])).is_none());
}

#[test]
fn a_discography_reads_every_page_musicbrainz_lists() {
    let page = |rows: usize, count: u64| json!({"release-group-count": count, "release-groups": vec![json!({}); rows]});
    assert_eq!(next_offset(&page(100, 141), 0), Some(100));
    assert_eq!(next_offset(&page(41, 141), 100), None);
    assert_eq!(next_offset(&page(100, 100), 0), None);
    assert_eq!(next_offset(&page(0, 141), 100), None);
}

#[test]
fn a_tracklist_numbers_tracks_within_their_disc() {
    let release = json!({"media":[
        {"position":1,"tracks":[{"position":1,"title":"Intro","length":61_500},{"position":2,"title":"Song","length":null}]},
        {"position":2,"tracks":[{"position":1,"title":"Encore","length":200_000}]},
    ]});
    assert_eq!(
        tracklist(&release),
        [
            json!({"disc":1,"number":1,"title":"Intro","length":61}),
            json!({"disc":1,"number":2,"title":"Song","length":null}),
            json!({"disc":2,"number":1,"title":"Encore","length":200}),
        ]
    );
}

#[test]
fn an_album_shows_the_release_lidarr_monitors() {
    let row = json!({"releases":[
        {"monitored":false,"label":["Other"],"format":"Vinyl","trackCount":9,"duration":1},
        {"monitored":true,"label":["Label"],"format":"CD","disambiguation":"Deluxe","trackCount":12,"duration":2_580_000},
    ]});
    let mut value = json!({});
    album_facts(&mut value, &row, Vec::new());
    assert_eq!(value["label"], "Label");
    assert_eq!(value["format"], "CD");
    assert_eq!(value["edition"], "Deluxe");
    assert_eq!(value["track_count"], 12);
    assert_eq!(value["duration"], 2580);
}

#[test]
fn artist_names_match_regardless_of_case_and_punctuation() {
    assert!(same_name("AC/DC", "acdc"));
    assert!(same_name("Radiohead", " radiohead"));
    assert!(!same_name("Radiohead", "radio"));
    assert!(!same_name("!!!", "?"));
}

#[test]
fn only_canonical_musicbrainz_ids_are_accepted() {
    assert_eq!(mbid(Some(HELD)), Some(HELD));
    assert_eq!(mbid(Some(&HELD.replace('-', ""))), None);
    assert_eq!(mbid(Some(&format!("urn:uuid:{HELD}"))), None);
}

#[test]
fn artist_pictures_come_only_from_lidarrs_image_proxy() {
    let row = |url: &str| {
        json!({"images":[
            {"coverType":"fanart","remoteUrl":"https://images.lidarr.audio/a.jpg"},
            {"coverType":"poster","remoteUrl":url},
        ]})
    };
    assert_eq!(
        poster(&row("https://images.lidarr.audio/cache/b.jpg")).as_deref(),
        Some("https://images.lidarr.audio/cache/b.jpg")
    );
    assert_eq!(poster(&row("https://example.com/b.jpg")), None);
}

#[tokio::test]
async fn similar_artists_start_from_the_librarys_identified_artists() {
    let (_temp, state, _) = crate::test_support::fixture().await;
    state
        .db
        .write("test.fixture", |db| {
            db.execute("INSERT INTO library_roots(id,name,kind,path) VALUES ('music','Music','music','/media/music')", [])?;
            for (id, title, artist) in [("b", "Beta", Some(HELD)), ("a", "Alpha", Some(MISSING)), ("c", "Beta again", Some(HELD)), ("d", "Unknown", None)] {
                let metadata = artist.map_or(json!({}), |artist| json!({"musicbrainz_artist_id": artist}));
                db.execute(
                    "INSERT INTO media(id,root_id,kind,evidence_key,title,metadata,created_at) VALUES (?1,'music','artist',?1,?2,?3,1)",
                    params![id, title, metadata.to_string()],
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(
        super::super::requests::library_artists(&state)
            .await
            .unwrap(),
        [
            ("Alpha".into(), MISSING.into()),
            ("Beta".into(), HELD.into())
        ]
    );
}

#[tokio::test]
async fn albums_show_library_files_and_the_viewers_latest_request() {
    let (_temp, state, _) = crate::test_support::fixture().await;
    state
        .db
        .write("test.fixture", |db| {
            db.execute("INSERT INTO manager_services(id,name,kind,container_id,port,generation,credential,media_source,defaults,version,enabled,checked_at) VALUES ('lidarr','Lidarr','lidarr','c',8686,'g',X'00','','{}','1',1,1)", [])?;
            db.execute("INSERT INTO library_roots(id,name,kind,path) VALUES ('music','Music','music','/media/music')", [])?;
            db.execute("INSERT INTO media_files(id,root_id,path,generation,size,modified,fingerprint,probe,scanned_at) VALUES ('track-file','music','/media/music/a.flac','g',1,'1','f','{}',1)", [])?;
            let album = |id: &str, group: &str| {
                db.execute(
                    "INSERT INTO media(id,root_id,kind,evidence_key,title,metadata,created_at) VALUES (?1,'music','album',?1,?1,?2,1)",
                    params![id, json!({"musicbrainz_release_group_id": group}).to_string()],
                )
            };
            album("held", HELD)?;
            album("missing", MISSING)?;
            db.execute("INSERT INTO media(id,root_id,kind,parent_id,evidence_key,title,created_at) VALUES ('track','music','track','held','track','Track',1)", [])?;
            db.execute("INSERT INTO media_sources(media_id,file_id) VALUES ('track','track-file')", [])?;
            for (id, user, state, at) in [("old", "alice", "denied", 1), ("new", "alice", "pending", 2), ("other", "bob", "requested", 3)] {
                db.execute(
                    "INSERT INTO acquisition_requests(id,user_id,service_id,generation,external_id,title,state,created_at,updated_at) VALUES (?1,?2,'lidarr','g',?3,'Asked',?4,?5,?5)",
                    params![id, user, ASKED, state, at],
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();
    let mut items = [HELD, MISSING, ASKED].map(|id| album(id, "Album", None, None));
    super::super::requests::decorate(&state, "alice", "lidarr", &mut items)
        .await
        .unwrap();
    assert_eq!(items[0]["in_library"], true);
    assert_eq!(
        items[1]["in_library"], false,
        "An album without files is not held"
    );
    assert_eq!(items[1]["request"], Value::Null);
    assert_eq!(items[2]["request"], json!({"id":"new","state":"pending"}));
}
