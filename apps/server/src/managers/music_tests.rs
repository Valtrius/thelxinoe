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
    assert_eq!(
        items[0]["cover"],
        format!("/api/v1/acquisition/music/covers/{HELD}")
    );
}

#[test]
fn fresh_keeps_albums_most_listened_first() {
    let body = json!({"payload":{"releases":[
        {"release_group_mbid":HELD,"release_name":"Quiet","artist_credit_name":"A","release_date":"2026-10-20","release_group_primary_type":"Album","listen_count":3},
        {"release_group_mbid":MISSING,"release_name":"Single","artist_credit_name":"B","release_date":"2026-10-21","release_group_primary_type":"Single","listen_count":90},
        {"release_group_mbid":ASKED,"release_name":"Loud","artist_credit_name":"C","release_date":"2026-10-19","release_group_primary_type":"Album","listen_count":40},
    ]}});
    let items = fresh(&body);
    assert_eq!(
        items
            .iter()
            .map(|item| item["title"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["Loud", "Quiet"]
    );
    assert_eq!(items[1]["year"], 2026);
}

#[test]
fn lidarr_calendar_lists_newest_releases_first() {
    let rows = [
        json!({"foreignAlbumId":HELD,"title":"Older","releaseDate":"2026-08-01T00:00:00Z","artist":{"artistName":"Band"}}),
        json!({"foreignAlbumId":MISSING,"title":"Newer","releaseDate":"2026-10-01T00:00:00Z","artist":{"artistName":"Band"}}),
    ];
    let items = released(&rows);
    assert_eq!(items[0]["title"], "Newer");
    assert_eq!(items[1]["external_id"], HELD);
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
