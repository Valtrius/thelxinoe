use super::*;

/// Alice and Bob, a show and a movie. The show has Specials, an episode without a file (`e3`)
/// and a file holding two episodes (`e4` and `e5`); the movie has two editions. Every file
/// lasts 100 seconds.
async fn catalog() -> (tempfile::TempDir, Database) {
    let temp = tempfile::tempdir().unwrap();
    let db = Database::open(temp.path().join("db.sqlite3")).unwrap();
    db.write("test.catalog", |db| {
        db.execute_batch(
            "INSERT INTO users(id,username,role,created_at) VALUES ('alice','alice','user',1),('bob','bob','user',1);
INSERT INTO library_roots(id,name,kind,path) VALUES ('shows','Shows','shows','/shows'),('movies','Movies','movies','/movies');
INSERT INTO media(id,root_id,kind,parent_id,evidence_key,title,sort_number,created_at) VALUES
 ('show','shows','show',NULL,'show','Show',NULL,1),
 ('specials','shows','season','show','specials','Specials',0,1),
 ('sp1','shows','episode','specials','sp1','Special 1',1,1),
 ('sp2','shows','episode','specials','sp2','Special 2',2,1),
 ('s1','shows','season','show','s1','Season 1',1,1),
 ('e1','shows','episode','s1','e1','One',1,1),
 ('e2','shows','episode','s1','e2','Two',2,1),
 ('e3','shows','episode','s1','e3','Three',3,1),
 ('s2','shows','season','show','s2','Season 2',2,1),
 ('e4','shows','episode','s2','e4','Four',1,1),
 ('e5','shows','episode','s2','e5','Five',2,1),
 ('movie','movies','movie',NULL,'movie','Movie',NULL,1);",
        )?;
        for (file, media, root, edition) in [
            ("fsp1", "sp1", "shows", ""),
            ("fsp2", "sp2", "shows", ""),
            ("f1", "e1", "shows", ""),
            ("f2", "e2", "shows", ""),
            ("f45", "e4", "shows", ""),
            ("f45", "e5", "shows", ""),
            ("theatrical", "movie", "movies", ""),
            ("cut", "movie", "movies", "Director's Cut"),
        ] {
            db.execute("INSERT OR IGNORE INTO media_files(id,root_id,path,generation,size,modified,fingerprint,probe,edition,scanned_at) VALUES (?1,?2,?1,'g',1,'m',?1,'{\"format\":{\"duration\":\"100\"}}',?3,1)", params![file, root, edition])?;
            db.execute("INSERT INTO media_sources(media_id,file_id) VALUES (?1,?2)", [media, file])?;
        }
        Ok(())
    })
    .await
    .unwrap();
    (temp, db)
}

async fn write<T: Send + 'static>(
    db: &Database,
    f: impl FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
) -> T {
    db.write("test.write", move |db| {
        let tx = db.transaction()?;
        let value = f(&tx)?;
        tx.commit()?;
        Ok(value)
    })
    .await
    .unwrap()
}

async fn read<T: Send + 'static>(
    db: &Database,
    f: impl FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
) -> T {
    db.read("test.read", move |db| Ok(f(db)?)).await.unwrap()
}

async fn mark_as(
    db: &Database,
    user: &'static str,
    media: &'static str,
    watched: bool,
    scope: Scope<'static>,
) -> Marked {
    write(db, move |tx| mark(tx, user, media, watched, scope, 10)).await
}

/// Whether the user watched each item, and its resume points as (edition, position).
async fn state(
    db: &Database,
    user: &'static str,
    media: &'static str,
) -> (bool, Vec<(String, f64)>) {
    let (watched, mut points) = read(db, move |db| item(db, user, media)).await;
    points.sort_by(|a, b| a.edition.cmp(&b.edition));
    (
        watched,
        points
            .into_iter()
            .map(|p| (p.edition, p.position))
            .collect(),
    )
}

fn items(marked: Marked) -> Option<usize> {
    match marked {
        Marked::Items(count) => Some(count),
        Marked::Missing | Marked::Unsupported => None,
    }
}

#[tokio::test]
async fn marking_fills_every_resume_point_and_unmarking_rewinds_them() {
    let (_temp, db) = catalog().await;
    write(&db, |tx| {
        set_resume_point(tx, "alice", "movie", 40.0, Edition::LastResumed, 1)?;
        set_resume_point(
            tx,
            "alice",
            "movie",
            300.0,
            Edition::Named("Director's Cut"),
            2,
        )
    })
    .await;
    // The first edition when none was resumed; any position stops at the edition's end.
    assert_eq!(
        state(&db, "alice", "movie").await,
        (
            false,
            vec![("".into(), 40.0), ("Director's Cut".into(), 100.0)]
        )
    );

    assert_eq!(
        items(mark_as(&db, "alice", "movie", true, Scope::Item).await),
        Some(1)
    );
    assert_eq!(
        state(&db, "alice", "movie").await,
        (
            true,
            vec![("".into(), 100.0), ("Director's Cut".into(), 100.0)]
        )
    );
    mark_as(&db, "alice", "movie", false, Scope::Item).await;
    assert_eq!(
        state(&db, "alice", "movie").await,
        (
            false,
            vec![("".into(), 0.0), ("Director's Cut".into(), 0.0)]
        )
    );
    assert_eq!(state(&db, "bob", "movie").await, (false, vec![]));
}

#[tokio::test]
async fn a_change_that_sets_a_resume_point_keeps_it_while_unmarking() {
    let (_temp, db) = catalog().await;
    let change_as = |media: &'static str, change: Change| {
        let db = db.clone();
        async move { write(&db, move |tx| super::change(tx, "alice", media, change, 10)).await }
    };
    change_as(
        "movie",
        Change {
            resume: Some(20.0),
            ..Change::default()
        },
    )
    .await;
    change_as(
        "movie",
        Change {
            resume: Some(70.0),
            ..Change::default()
        },
    )
    .await;
    change_as(
        "movie",
        Change {
            watched: Some(true),
            ..Change::default()
        },
    )
    .await;
    assert_eq!(
        state(&db, "alice", "movie").await,
        (true, vec![("".into(), 100.0)])
    );
    let changed = change_as(
        "movie",
        Change {
            watched: Some(false),
            resume: Some(25.0),
            ..Change::default()
        },
    )
    .await;
    assert!(matches!(changed, Changed::Done));
    assert_eq!(
        state(&db, "alice", "movie").await,
        (false, vec![("".into(), 25.0)])
    );

    // A resume point needs a file to measure it against.
    let no_file = change_as(
        "e3",
        Change {
            resume: Some(5.0),
            ..Change::default()
        },
    )
    .await;
    assert!(matches!(no_file, Changed::NoEdition));
    // Watched and resume points belong to playable items, Watch Later to titles too.
    let season = change_as(
        "s1",
        Change {
            watched: Some(true),
            ..Change::default()
        },
    )
    .await;
    assert!(matches!(season, Changed::Unsupported));
    let later = change_as(
        "s1",
        Change {
            watch_later: Some(true),
            ..Change::default()
        },
    )
    .await;
    assert!(matches!(later, Changed::Done));
    let missing = change_as(
        "nothing",
        Change {
            favorite: Some(true),
            ..Change::default()
        },
    )
    .await;
    assert!(matches!(missing, Changed::Missing));
}

#[tokio::test]
async fn marks_cover_a_season_or_every_episode_through_one() {
    let (_temp, db) = catalog().await;
    let watched = |user: &'static str| {
        let db = db.clone();
        async move {
            read(&db, move |db| {
                ids(db, "SELECT media_id FROM media_state WHERE user_id=?1 AND watched=1 ORDER BY media_id", [user])
            })
            .await
        }
    };
    assert_eq!(
        items(mark_as(&db, "alice", "s1", true, Scope::Episodes).await),
        Some(3)
    );
    assert_eq!(watched("alice").await, ["e1", "e2", "e3"]);

    // Earlier seasons and episodes, but not Specials nor what shares the episode's file.
    assert_eq!(
        items(mark_as(&db, "bob", "e4", true, Scope::Through).await),
        Some(4)
    );
    assert_eq!(watched("bob").await, ["e1", "e2", "e3", "e4"]);
    // Specials only among Specials.
    assert_eq!(
        items(mark_as(&db, "bob", "sp2", true, Scope::Through).await),
        Some(2)
    );
    assert_eq!(watched("bob").await, ["e1", "e2", "e3", "e4", "sp1", "sp2"]);
    assert_eq!(
        items(mark_as(&db, "bob", "s2", false, Scope::Episodes).await),
        Some(2)
    );
    assert_eq!(watched("bob").await, ["e1", "e2", "e3", "sp1", "sp2"]);

    for (media, scope) in [
        ("s1", Scope::Item),
        ("s1", Scope::Through),
        ("movie", Scope::Episodes),
        ("show", Scope::Episodes),
    ] {
        assert!(
            matches!(
                mark_as(&db, "alice", media, true, scope).await,
                Marked::Unsupported
            ),
            "{media}"
        );
    }
    assert!(matches!(
        mark_as(&db, "alice", "nothing", true, Scope::Item).await,
        Marked::Missing
    ));
}

#[tokio::test]
async fn finished_playback_marks_the_item_and_the_other_episodes_of_its_file() {
    let (_temp, db) = catalog().await;
    let play = |position: f64| {
        let db = db.clone();
        async move {
            write(&db, move |tx| {
                record_playback(
                    tx,
                    "alice",
                    Played {
                        media: "e4",
                        edition: "",
                        file: Some("f45"),
                        position,
                        duration: 100.0,
                    },
                    10,
                )
            })
            .await
        }
    };
    write(&db, |tx| {
        set_resume_point(tx, "alice", "e5", 20.0, Edition::LastResumed, 1)
    })
    .await;
    play(50.0).await;
    assert_eq!(
        state(&db, "alice", "e4").await,
        (false, vec![("".into(), 50.0)])
    );
    assert_eq!(
        state(&db, "alice", "e5").await,
        (false, vec![("".into(), 20.0)])
    );

    play(90.0).await;
    // The item keeps the position it reached; the episodes sharing its file are finished.
    assert_eq!(
        state(&db, "alice", "e4").await,
        (true, vec![("".into(), 90.0)])
    );
    assert_eq!(
        state(&db, "alice", "e5").await,
        (true, vec![("".into(), 100.0)])
    );

    // Playing it again never unmarks it.
    play(10.0).await;
    assert_eq!(
        state(&db, "alice", "e4").await,
        (true, vec![("".into(), 10.0)])
    );
}

#[tokio::test]
async fn a_show_is_watched_once_every_episode_is_while_jellyfin_counts_only_playable_ones() {
    let (_temp, db) = catalog().await;
    let facts = || {
        let db = db.clone();
        async move {
            read(&db, |db| {
                let sql = format!(
                    "SELECT m.id,{},{} FROM media m LEFT JOIN media_state u ON u.media_id=m.id AND u.user_id=?1 WHERE m.id IN ('show','specials','s1','e1') ORDER BY m.id",
                    watched_sql("m", "u", "?1"),
                    unwatched_count_sql("m", "?1")
                );
                db.prepare(&sql)?
                    .query_map(["alice"], |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?, r.get::<_, Option<i64>>(2)?)))?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
        }
    };
    let row = |id: &str, watched: bool, unwatched: Option<i64>| (id.to_owned(), watched, unwatched);
    assert_eq!(
        facts().await,
        [
            row("e1", false, None),
            row("s1", false, Some(2)),
            row("show", false, Some(4)),
            row("specials", false, Some(2))
        ]
    );
    for season in ["s1", "s2"] {
        mark_as(&db, "alice", season, true, Scope::Episodes).await;
    }
    // Specials keep the show unwatched, though Jellyfin counts nothing left in it.
    assert_eq!(
        facts().await,
        [
            row("e1", true, None),
            row("s1", true, Some(0)),
            row("show", false, Some(0)),
            row("specials", false, Some(2))
        ]
    );
    mark_as(&db, "alice", "specials", true, Scope::Episodes).await;
    assert_eq!(
        facts().await,
        [
            row("e1", true, None),
            row("s1", true, Some(0)),
            row("show", true, Some(0)),
            row("specials", true, Some(0))
        ]
    );
    // So does an episode without a file.
    mark_as(&db, "alice", "e3", false, Scope::Item).await;
    assert_eq!(
        facts().await,
        [
            row("e1", true, None),
            row("s1", false, Some(0)),
            row("show", false, Some(0)),
            row("specials", true, Some(0))
        ]
    );

    // The web app's cards: started once anything in them is watched or resumed.
    write(&db, |tx| {
        set_resume_point(tx, "bob", "e1", 30.0, Edition::LastResumed, 1)
    })
    .await;
    let cards = |user: &'static str| {
        let db = db.clone();
        async move {
            let facts = read(&db, move |db| cards(db, user, r#"["show","s2","movie"]"#)).await;
            let mut facts = facts
                .into_iter()
                .map(|(id, f)| (id, f.watched, f.started))
                .collect::<Vec<_>>();
            facts.sort();
            facts
        }
    };
    assert_eq!(
        cards("alice").await,
        [
            ("movie".into(), false, false),
            ("s2".into(), true, true),
            ("show".into(), false, true)
        ]
    );
    assert_eq!(
        cards("bob").await,
        [
            ("movie".into(), false, false),
            ("s2".into(), false, false),
            ("show".into(), false, true)
        ]
    );
}

#[tokio::test]
async fn continue_watching_needs_a_file_of_the_resume_points_own_edition() {
    let (_temp, db) = catalog().await;
    let shelf = || {
        let db = db.clone();
        async move {
            read(&db, |db| continue_watching(db, "alice"))
                .await
                .into_iter()
                .map(|e| (e.id, e.edition, e.file, e.position))
                .collect::<Vec<_>>()
        }
    };
    write(&db, |tx| {
        set_resume_point(
            tx,
            "alice",
            "movie",
            40.0,
            Edition::Named("Director's Cut"),
            1,
        )?;
        set_resume_point(tx, "alice", "e1", 95.0, Edition::LastResumed, 2)
    })
    .await;
    // A finished resume point is not in progress.
    assert_eq!(
        shelf().await,
        [("movie".into(), "Director's Cut".into(), "cut".into(), 40.0)]
    );
    write(&db, |tx| {
        tx.execute("UPDATE media_files SET present=0 WHERE id='cut'", [])
    })
    .await;
    assert!(shelf().await.is_empty());
}

#[tokio::test]
async fn next_up_skips_episodes_without_a_file() {
    let (_temp, db) = catalog().await;
    let shelf = || {
        let db = db.clone();
        async move {
            read(&db, |db| home_next_up(db, "alice"))
                .await
                .into_iter()
                .map(|e| e.id)
                .collect::<Vec<_>>()
        }
    };
    assert!(shelf().await.is_empty());
    mark_as(&db, "alice", "e1", true, Scope::Item).await;
    assert_eq!(shelf().await, ["e2"]);
    mark_as(&db, "alice", "e2", true, Scope::Item).await;
    assert_eq!(shelf().await, ["e4"]);
}

/// No query reads every episode of the library to rank them.
#[tokio::test]
async fn only_the_series_asked_for_are_ranked() {
    let (_temp, db) = catalog().await;
    let plans = read(&db, |db| {
        [
            home_next_up_sql(),
            next_up_sql(false, true),
            next_up_sql(true, true),
            up_next_sql(),
        ]
        .iter()
        .map(|sql| crate::test_support::query_plan(db, sql))
        .collect::<rusqlite::Result<Vec<_>>>()
    })
    .await;
    for plan in plans {
        // Episodes are found through their show's seasons.
        assert!(
            plan.iter()
                .any(|step| step == "SEARCH e USING INDEX media_parent (parent_id=?)"),
            "{plan:#?}"
        );
        // Media is never read by its kind or in full.
        assert!(
            !plan.iter().any(|step| step.contains("media_by_title")
                || ["SCAN e", "SCAN s", "SCAN m", "SCAN sh", "SCAN c"].contains(&step.as_str())),
            "{plan:#?}"
        );
    }
}
