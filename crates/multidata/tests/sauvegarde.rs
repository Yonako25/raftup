//! Tests du format de sauvegarde `.raftup`.

#![allow(clippy::unwrap_used)]

use std::path::PathBuf;

use ap_protocol::{Hint, HintStatus, ItemFlags, NetworkItem};
use multidata::raftup::{
    diff, seed_sha256, AwayInfo, FeedEntry, FeedKind, Record, SaveError, SaveFile, SaveHeader,
    Snapshot, FEED_MAX, FORMAT_VERSION,
};

/// Chemin unique par test, sous le dossier temporaire de cargo.
fn path(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("raftup-save");
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(format!("{name}.raftup"));
    let _ = std::fs::remove_file(&p);
    let _ = std::fs::remove_file(format!("{}.tmp", p.display()));
    p
}

fn header() -> SaveHeader {
    SaveHeader::new("0.1.0", "AP_TEST", &seed_sha256(b"seed"), 1_760_000_000.5)
}

fn item(n: i64) -> NetworkItem {
    NetworkItem {
        item: n,
        location: n + 1000,
        player: 1,
        flags: ItemFlags::PROGRESSION,
    }
}

fn hint(n: i64) -> Hint {
    Hint {
        receiving_player: 1,
        finding_player: 2,
        location: n,
        item: n,
        found: false,
        entrance: String::new(),
        item_flags: ItemFlags::NONE,
        status: HintStatus::UNSPECIFIED,
    }
}

fn sample_records() -> Vec<Record> {
    vec![
        Record::Checks {
            team: 0,
            slot: 1,
            locations: vec![3, 1],
        },
        Record::Items {
            team: 0,
            slot: 1,
            remote: false,
            items: vec![item(1)],
        },
        Record::Status {
            team: 0,
            slot: 1,
            status: 20,
        },
        Record::Alias {
            team: 0,
            slot: 1,
            alias: Some("Zoé".into()),
        },
        Record::Store {
            key: "k".into(),
            value: serde_json::json!({"a": [1, 2]}),
        },
    ]
}

#[test]
fn seed_sha256_is_lowercase_hex() {
    assert_eq!(
        seed_sha256(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn roundtrip_create_append_open() {
    let p = path("roundtrip");
    let mut f = SaveFile::create(&p, header()).unwrap();
    f.append(&sample_records()).unwrap();
    assert_eq!(f.lines_since_compact(), 5);
    drop(f);
    let (f2, snap) = SaveFile::open(&p, &seed_sha256(b"seed")).unwrap();
    assert!(!f2.ignored_tail());
    assert_eq!(f2.lines_since_compact(), 5);
    let mut expected = Snapshot::default();
    for r in sample_records() {
        expected.apply(&r);
    }
    assert_eq!(snap, expected);
    assert_eq!(snap.location_checks[0].locations, vec![1, 3]);
}

#[test]
fn apply_each_record() {
    let mut s = Snapshot::default();
    s.apply(&Record::Checks {
        team: 0,
        slot: 1,
        locations: vec![5, 2],
    });
    s.apply(&Record::Checks {
        team: 0,
        slot: 1,
        locations: vec![2, 9],
    });
    assert_eq!(s.location_checks[0].locations, vec![2, 5, 9]);

    for n in [1, 2] {
        s.apply(&Record::Items {
            team: 0,
            slot: 1,
            remote: true,
            items: vec![item(n)],
        });
    }
    assert_eq!(s.received_items[0].items, vec![item(1), item(2)]);

    s.apply(&Record::Status {
        team: 0,
        slot: 1,
        status: 10,
    });
    s.apply(&Record::Status {
        team: 0,
        slot: 1,
        status: 30,
    });
    assert_eq!(s.client_status.len(), 1);
    assert_eq!(s.client_status[0].status, 30);

    s.apply(&Record::Hints {
        team: 0,
        slot: 1,
        hints: vec![hint(1), hint(2)],
    });
    s.apply(&Record::Hints {
        team: 0,
        slot: 1,
        hints: vec![hint(3)],
    });
    assert_eq!(s.hints[0].hints, vec![hint(3)]);

    s.apply(&Record::HintsUsed {
        team: 0,
        slot: 1,
        total: 2,
    });
    s.apply(&Record::HintsUsed {
        team: 0,
        slot: 1,
        total: 4,
    });
    assert_eq!(s.hints_used[0].count, 4);

    s.apply(&Record::Alias {
        team: 0,
        slot: 1,
        alias: Some("a".into()),
    });
    assert_eq!(s.aliases[0].alias, "a");
    s.apply(&Record::Alias {
        team: 0,
        slot: 1,
        alias: None,
    });
    assert!(s.aliases.is_empty());

    for _ in 0..2 {
        s.apply(&Record::GroupCollected {
            group: 5,
            member: 1,
        });
    }
    assert_eq!(s.group_collected.len(), 1);

    s.apply(&Record::Store {
        key: "k".into(),
        value: serde_json::json!(1),
    });
    s.apply(&Record::Store {
        key: "k".into(),
        value: serde_json::json!(2),
    });
    assert_eq!(s.store.len(), 1);
    assert_eq!(s.store[0].value, serde_json::json!(2));
}

#[test]
fn truncated_last_line_is_ignored_and_file_stays_usable() {
    let p = path("truncated");
    let mut f = SaveFile::create(&p, header()).unwrap();
    f.append(&sample_records()).unwrap();
    drop(f);
    let mut text = std::fs::read_to_string(&p).unwrap();
    text.push_str("{\"type\":\"checks\",\"team\":0,\"sl");
    std::fs::write(&p, text).unwrap();

    let (mut f, snap) = SaveFile::open(&p, &seed_sha256(b"seed")).unwrap();
    assert!(f.ignored_tail());
    assert_eq!(f.lines_since_compact(), 5);
    assert_eq!(snap.location_checks[0].locations, vec![1, 3]);
    // Un ajout après réouverture ne doit pas être collé à la ligne tronquée.
    f.append(&[Record::GroupCollected {
        group: 1,
        member: 2,
    }])
    .unwrap();
    drop(f);
    let (f, snap) = SaveFile::open(&p, &seed_sha256(b"seed")).unwrap();
    assert!(!f.ignored_tail());
    assert_eq!(snap.group_collected.len(), 1);
}

#[test]
fn corrupted_middle_line_is_rejected() {
    let p = path("corrupted");
    let mut f = SaveFile::create(&p, header()).unwrap();
    f.append(&sample_records()).unwrap();
    drop(f);
    let text = std::fs::read_to_string(&p).unwrap();
    let mut lines: Vec<&str> = text.lines().collect();
    lines[3] = "{pas du json";
    std::fs::write(&p, lines.join("\n") + "\n").unwrap();
    match SaveFile::open(&p, &seed_sha256(b"seed")) {
        Err(SaveError::Corrupted { line }) => assert_eq!(line, 4),
        other => panic!("attendu Corrupted, obtenu {other:?}"),
    }
}

#[test]
fn future_format_version_is_rejected() {
    let p = path("v3");
    drop(SaveFile::create(&p, header()).unwrap());
    let text = std::fs::read_to_string(&p).unwrap();
    std::fs::write(
        &p,
        text.replacen("\"format_version\":2", "\"format_version\":3", 1),
    )
    .unwrap();
    assert!(matches!(
        SaveFile::open(&p, &seed_sha256(b"seed")),
        Err(SaveError::UnsupportedVersion { found: 3, .. })
    ));
}

#[test]
fn wrong_seed_hash_is_rejected() {
    let p = path("seed");
    drop(SaveFile::create(&p, header()).unwrap());
    assert!(matches!(
        SaveFile::open(&p, &seed_sha256(b"autre")),
        Err(SaveError::SeedMismatch)
    ));
}

#[test]
fn not_a_save_is_rejected() {
    let p = path("notasave");
    std::fs::write(&p, "hello\n").unwrap();
    assert!(matches!(SaveFile::open(&p, "x"), Err(SaveError::NotASave)));
}

#[test]
fn big_integer_in_store_is_preserved() {
    let p = path("bigint");
    let big: serde_json::Value = serde_json::from_str("123456789012345678901234567890").unwrap();
    let mut f = SaveFile::create(&p, header()).unwrap();
    f.append(&[Record::Store {
        key: "big".into(),
        value: big,
    }])
    .unwrap();
    drop(f);
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(text.contains("123456789012345678901234567890"));
    let (_, snap) = SaveFile::open(&p, &seed_sha256(b"seed")).unwrap();
    assert_eq!(
        snap.store[0].value.to_string(),
        "123456789012345678901234567890"
    );
}

#[test]
fn compact_then_reopen_is_identical() {
    let p = path("compact");
    let mut f = SaveFile::create(&p, header()).unwrap();
    f.append(&sample_records()).unwrap();
    let mut snap = Snapshot::default();
    for r in sample_records() {
        snap.apply(&r);
    }
    f.compact(&snap).unwrap();
    assert_eq!(f.lines_since_compact(), 0);
    // Le fichier compacté reste utilisable en ajout.
    let extra = Record::GroupCollected {
        group: 9,
        member: 3,
    };
    f.append(std::slice::from_ref(&extra)).unwrap();
    snap.apply(&extra);
    drop(f);
    let (f, reopened) = SaveFile::open(&p, &seed_sha256(b"seed")).unwrap();
    assert_eq!(reopened, snap);
    assert_eq!(f.lines_since_compact(), 1);
    assert!(!std::path::Path::new(&format!("{}.tmp", p.display())).exists());
}

#[test]
fn leftover_tmp_does_not_prevent_opening() {
    let p = path("leftover");
    let mut f = SaveFile::create(&p, header()).unwrap();
    f.append(&sample_records()).unwrap();
    drop(f);
    // Compactage interrompu : un .tmp à moitié écrit traîne.
    std::fs::write(format!("{}.tmp", p.display()), "{\"format_ver").unwrap();
    let (_, snap) = SaveFile::open(&p, &seed_sha256(b"seed")).unwrap();
    assert_eq!(snap.location_checks[0].locations, vec![1, 3]);
}

#[test]
fn truncated_line_cutting_a_utf8_char_is_ignored() {
    let p = path("utf8-cut");
    let mut f = SaveFile::create(&p, header()).unwrap();
    f.append(&sample_records()).unwrap();
    drop(f);
    // Dernière ligne coupée entre les deux octets du « é » de « Zoé ».
    let mut bytes = std::fs::read(&p).unwrap();
    let line = "{\"type\":\"alias\",\"team\":0,\"slot\":1,\"alias\":\"Zoé\"}\n";
    let cut = line.find('é').unwrap() + 1;
    bytes.extend_from_slice(&line.as_bytes()[..cut]);
    std::fs::write(&p, bytes).unwrap();

    let (f, snap) = SaveFile::open(&p, &seed_sha256(b"seed")).unwrap();
    assert!(f.ignored_tail());
    assert_eq!(f.lines_since_compact(), 5);
    assert_eq!(snap.location_checks[0].locations, vec![1, 3]);
}

#[test]
fn format_version_zero_is_rejected() {
    let p = path("v0");
    drop(SaveFile::create(&p, header()).unwrap());
    let text = std::fs::read_to_string(&p).unwrap();
    std::fs::write(
        &p,
        text.replacen("\"format_version\":2", "\"format_version\":0", 1),
    )
    .unwrap();
    assert!(matches!(
        SaveFile::open(&p, &seed_sha256(b"seed")),
        Err(SaveError::UnsupportedVersion { found: 0, .. })
    ));
}

#[test]
fn failed_create_keeps_the_previous_save() {
    let p = path("create-keeps-old");
    let mut f = SaveFile::create(&p, header()).unwrap();
    f.append(&sample_records()).unwrap();
    drop(f);
    let before = std::fs::read(&p).unwrap();
    // Un dossier à la place du `.tmp` fait échouer l'écriture.
    let tmp = PathBuf::from(format!("{}.tmp", p.display()));
    std::fs::create_dir_all(&tmp).unwrap();
    let result = SaveFile::create(&p, header());
    std::fs::remove_dir(&tmp).unwrap();
    assert!(result.is_err());
    assert_eq!(std::fs::read(&p).unwrap(), before);
}

/// Une partie simulée : `sample_records` puis des changements de chaque sorte,
/// y compris un surnom retiré et une valeur du stockage remplacée.
fn evolution() -> Vec<Record> {
    let mut records = sample_records();
    records.extend([
        Record::Checks {
            team: 0,
            slot: 2,
            locations: vec![7],
        },
        Record::Items {
            team: 0,
            slot: 1,
            remote: false,
            items: vec![item(2), item(3)],
        },
        Record::Hints {
            team: 0,
            slot: 1,
            hints: vec![hint(1), hint(2)],
        },
        Record::HintsUsed {
            team: 0,
            slot: 1,
            total: 2,
        },
        Record::GroupCollected {
            group: 5,
            member: 1,
        },
        Record::Alias {
            team: 0,
            slot: 1,
            alias: None,
        },
        Record::Store {
            key: "k".into(),
            value: serde_json::json!(12345678901234567890u64),
        },
        Record::Status {
            team: 0,
            slot: 1,
            status: 30,
        },
    ]);
    records.extend(async_records());
    records
}

/// Deltas du suivi asynchrone (format 2, ADR 0031), dont une absence levée et
/// des votes effacés.
fn async_records() -> Vec<Record> {
    let feed = |seq: u64, event: FeedKind| Record::Feed {
        entry: FeedEntry {
            seq,
            time: 10.0 + seq as f64,
            team: 0,
            slot: 2,
            event,
        },
    };
    vec![
        Record::Away {
            team: 0,
            slot: 2,
            away: Some(AwayInfo {
                since: 5.0,
                message: "Vacances".into(),
            }),
        },
        feed(
            1,
            FeedKind::Away {
                message: "Vacances".into(),
            },
        ),
        Record::Seen {
            team: 0,
            slot: 1,
            items: 3,
            time: 9.5,
        },
        Record::ReleaseVotes {
            team: 0,
            target: 2,
            voters: vec![1],
        },
        feed(
            2,
            FeedKind::ReleaseVote {
                voter: 1,
                votes: 1,
                needed: 2,
            },
        ),
        Record::Seen {
            team: 0,
            slot: 1,
            items: 4,
            time: 12.0,
        },
        Record::Away {
            team: 0,
            slot: 2,
            away: None,
        },
        Record::ReleaseVotes {
            team: 0,
            target: 2,
            voters: Vec::new(),
        },
        feed(3, FeedKind::Back),
    ]
}

#[test]
fn version_1_file_still_opens() {
    let p = path("v1");
    drop(SaveFile::create(&p, header()).unwrap());
    let text = std::fs::read_to_string(&p).unwrap();
    // Un vrai fichier version 1 : en-tête 1, instantané sans les champs du format 2.
    let lines: Vec<&str> = text.lines().collect();
    let v1 = format!(
        "{}\n{}\n{}\n",
        lines[0].replacen("\"format_version\":2", "\"format_version\":1", 1),
        r#"{"type":"snapshot","location_checks":[],"received_items":[],"client_status":[],"hints":[],"hints_used":[],"aliases":[],"group_collected":[],"store":[],"activity":[]}"#,
        r#"{"type":"checks","team":0,"slot":1,"locations":[4]}"#,
    );
    std::fs::write(&p, v1).unwrap();
    let (mut file, snap) = SaveFile::open(&p, &seed_sha256(b"seed")).unwrap();
    assert_eq!(file.header().format_version, 1);
    assert_eq!(snap.location_checks[0].locations, vec![4]);
    assert!(snap.away.is_empty() && snap.feed.is_empty());
    // Le compactage le réécrit en version courante.
    file.compact(&snap).unwrap();
    drop(file);
    let (file, again) = SaveFile::open(&p, &seed_sha256(b"seed")).unwrap();
    assert_eq!(file.header().format_version, FORMAT_VERSION);
    assert_eq!(again, snap);
}

#[test]
fn feed_keeps_the_last_entries() {
    let mut snap = Snapshot::default();
    for seq in 0..(FEED_MAX as u64 + 5) {
        snap.apply(&Record::Feed {
            entry: FeedEntry {
                seq,
                time: 0.0,
                team: 0,
                slot: 1,
                event: FeedKind::Goal,
            },
        });
    }
    assert_eq!(snap.feed.len(), FEED_MAX);
    assert_eq!(snap.feed[0].seq, 5);
}

#[test]
fn diff_step_by_step_rebuilds_each_snapshot() {
    let mut before = Snapshot::default();
    for record in evolution() {
        let mut after = before.clone();
        after.apply(&record);
        let mut rebuilt = before.clone();
        for delta in diff(&before, &after) {
            rebuilt.apply(&delta);
        }
        assert_eq!(rebuilt, after, "après {record:?}");
        before = after;
    }
}

#[test]
fn diff_in_one_jump_rebuilds_the_end() {
    let start = Snapshot::default();
    let mut end = Snapshot::default();
    for record in evolution() {
        end.apply(&record);
    }
    let mut rebuilt = start.clone();
    for delta in diff(&start, &end) {
        rebuilt.apply(&delta);
    }
    assert_eq!(rebuilt, end);
    // Rien n'a changé : aucun delta.
    assert!(diff(&end, &end).is_empty());
}
