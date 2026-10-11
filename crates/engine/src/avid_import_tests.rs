//! Combined-import regressions: use generated PCM and synthetic catalogs, never private fixtures.
use super::*;
use filmcraft_project::{BinEntry, ItemId};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Input {
    report: Vec<u8>,
    media: BTreeMap<String, Vec<u8>>,
    reads: AtomicUsize,
}
impl Services for Input {
    fn read_file(&self, path: &str) -> std::io::Result<Vec<u8>> {
        if path == "catalog.json" {
            self.reads.fetch_add(1, Ordering::Relaxed);
            Ok(self.report.clone())
        } else {
            self.media.get(path).cloned().ok_or_else(|| std::io::Error::other("unexpected read"))
        }
    }
    fn file_size(&self, path: &str) -> std::io::Result<u64> {
        if path == "catalog.json" {
            Ok(self.report.len() as u64)
        } else {
            self.media.get(path).map(|v| v.len() as u64).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "missing"))
        }
    }
    fn write_file(&self, _: &str, _: &[u8]) -> std::io::Result<()> {
        Err(std::io::Error::other("import must not write source files"))
    }
    fn reader(&self, path: &str) -> Option<std::io::Result<filmcraft_media::SharedReader>> {
        Some(self.read_file(path).map(|bytes| Arc::new(filmcraft_media::reader::MemReader(bytes.into())) as filmcraft_media::SharedReader))
    }
}
fn fixture() -> (Value, BTreeMap<String, Vec<u8>>) {
    let mut v = crate::avid_catalog_tests::fixture();
    let mut media = BTreeMap::new();
    let mut tracks = Vec::new();
    for i in 1..=2 {
        let ids = filmcraft_mxf::PackageIds::from_seed(&format!("combined-{i}"), "test");
        let raw: String = ids.file.0.iter().map(|b| format!("{b:02x}")).collect();
        let bytes = filmcraft_mxf::write_opatom_pcm(
            &filmcraft_mxf::OpAtomPcm { sample_rate: 48000, bits: 16, channels: 1, edit_rate: None, ids, timecode: None },
            &vec![if i == 1 { 4000 } else { -8000 }; 4800],
        )
        .unwrap();
        let filename = format!("A{i}.mxf");
        media.insert(format!("/combined/{filename}"), bytes);
        tracks.push(json!({"index":i,"status":"present","component":{"class":"SCLP","media_kind":2,"source_id":{"raw":raw,"byte_order":"big"},"source_track":i,"start":0,"length":4800,"rate":{"numerator":48000,"denominator":1}},"references":[{
            "status":"present","source_id":{"raw":raw,"byte_order":"big"},"source_track":i,"source_start":0,"length":4800,
            "rate":{"numerator":48000,"denominator":1},"locations":[{"folder":"/combined","filename":filename}]}]}));
    }
    let master = &mut v["bins"][0]["catalog"]["clips"][0];
    master["length"] = json!(4800);
    master["rate"] = json!({"numerator":48000,"denominator":1});
    master["workflow_evidence"] = json!("material_identity_in_managed_pmr");
    master["availability"] = json!("present");
    master["tracks"] = json!(tracks);
    let rate = json!({"numerator":24,"denominator":1});
    let clip = json!({"class":"SCLP","media_kind":2,"length":1,"start":1,"rate":rate,"source_id":master["id"],"source_track":2});
    let sequence = json!({"kind":"CompositionMob","name":"Trimmed","user_placed":true,"live":true,"id":{"raw":"02".repeat(32),"byte_order":"little"},
        "length":2,"rate":rate,"attributes":{"SEQUERNCE_FORMAT_STRING":"HD 1080p/24"},"tracks":[
        {"index":1,"component":{"class":"TCCP","media_kind":3,"length":2,"rate":rate,"start":86400,"fps":24,"flags":0}},
        {"index":2,"component":{"class":"SEQU","media_kind":2,"length":2,"rate":rate,"components":[
            {"class":"FILL","media_kind":2,"length":1,"rate":rate},clip]}}]});
    v["bins"][0]["mobs"] = json!([master.clone(), sequence]);
    (v, media)
}
fn session(v: &Value, media: BTreeMap<String, Vec<u8>>) -> (Session, Arc<Input>) {
    let input = Arc::new(Input { report: serde_json::to_vec(v).unwrap(), media, reads: AtomicUsize::new(0) });
    (Session::new(input.clone()), input)
}
fn params(mode: &str) -> Value {
    json!({"path":"catalog.json","mode":mode})
}

#[test]
fn avid_import_combined_preview_commit_undo_and_reopen() {
    let (v, media) = fixture();
    let (mut s, input) = session(&v, media);
    let before = s.project.clone();
    let state = serde_json::to_value(&s.state).unwrap();
    let revision = s.revision;
    let preview = s.execute("file.previewAvidCatalog", params("sequences")).unwrap();
    assert_eq!(s.project, before);
    assert_eq!(serde_json::to_value(&s.state).unwrap(), state);
    assert_eq!(s.revision, revision);
    assert!(s.history.undo.is_empty());
    assert_eq!(input.reads.load(Ordering::Relaxed), 1, "one snapshot per operation");
    assert_eq!(preview["bound"].as_array().unwrap().len(), 1);
    assert_eq!(preview["sequences"].as_array().unwrap().len(), 1);
    assert!(!preview["warnings"].to_string().contains("was not imported"));
    assert!(!preview["warnings"].to_string().contains("Metadata-only"));
    let out = s.execute("file.importAvidCatalog", params("sequences")).unwrap();
    assert_eq!(out, preview);
    assert_eq!(input.reads.load(Ordering::Relaxed), 2);
    assert_eq!(s.history.undo.len(), 1);
    let id = ItemId(out["sequences"][0]["item"].as_u64().unwrap());
    assert_eq!(s.state.active_sequence, Some(id));
    assert!(s.state.open_sequences.contains(&id));
    let seq = s.project.sequence(id).unwrap();
    assert_eq!(seq.duration(), Tick::from_units(2, 24));
    let clip = &seq.audio_tracks[1].items[0];
    assert_eq!((clip.start, clip.source_in), (Tick::from_units(1, 24), Tick::from_units(1, 24)));
    assert_eq!(clip.source_channels, vec![1]);
    let source = s.media.full_res_source(&s.project, clip.item, &*s.services).unwrap();
    assert_eq!(source.audio(2000, 8, 48000).unwrap().channels[1], vec![-8000.0 / 32768.0; 8]);
    let first_master = clip.item;
    let after = s.project.clone();
    assert_eq!(filmcraft_format::decode(&filmcraft_format::encode(&after, false)).unwrap().project, *after);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.project, before);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(s.project, after);
    let out = s.execute("file.importAvidCatalog", params("sequences")).unwrap();
    assert_eq!(out["bins"][0]["name"], "Sound Effects_2");
    let seq2 = s.project.sequence(ItemId(out["sequences"][0]["item"].as_u64().unwrap())).unwrap();
    assert_ne!(seq2.audio_tracks[1].items[0].item, first_master, "new sequence must use new bin's master");
}

#[test]
fn avid_import_combined_failures_leave_no_bins_media_or_history() {
    let (good, media) = fixture();
    for stage in ["second-bin-binding", "sequence"] {
        let mut v = good.clone();
        if stage == "sequence" {
            v["bins"][0]["mobs"][1]["tracks"][1]["component"]["components"][1]["class"] = json!("UNSUPPORTED");
        } else {
            let mut b = v["bins"][0].clone();
            b["summary"]["bin_uid"] = json!("second");
            b["catalog"]["clips"][0]["tracks"][1]["references"][0]["source_id"]["raw"] = json!("ff".repeat(32));
            b["catalog"]["clips"][0]["tracks"][1]["component"]["source_id"]["raw"] = json!("ff".repeat(32));
            b["mobs"][0] = b["catalog"]["clips"][0].clone();
            v["bins"].as_array_mut().unwrap().push(b);
        }
        let (mut s, _) = session(&v, media.clone());
        let before = s.project.clone();
        let state = serde_json::to_value(&s.state).unwrap();
        let revision = s.revision;
        for cmd in ["file.previewAvidCatalog", "file.importAvidCatalog"] {
            let error = s.execute(cmd, params("sequences")).unwrap_err().to_string();
            assert!(error.contains(if stage == "sequence" { "UNSUPPORTED" } else { "identity or single-track profile" }), "{stage}: {error}");
            assert_eq!(s.project, before);
            assert_eq!(serde_json::to_value(&s.state).unwrap(), state);
            assert_eq!(s.revision, revision);
            assert!(s.history.undo.is_empty());
            assert!(s.history.redo.is_empty());
        }
    }
    for mode in [json!(null), json!(true), json!(0), json!([]), json!("all"), json!({})] {
        let (mut s, _) = session(&good, media.clone());
        let before = s.project.clone();
        for cmd in ["file.previewAvidCatalog", "file.importAvidCatalog"] {
            assert!(s.execute(cmd, json!({"path":"catalog.json","mode":mode})).is_err());
            assert_eq!(s.project, before);
            assert!(s.history.undo.is_empty());
        }
    }
}

#[test]
fn avid_import_combined_modes_partial_channels_and_empty_bins() {
    let (mut v, media) = fixture();
    v["bins"][0]["catalog"]["clips"][0]["tracks"][0]["references"][0]["status"] = json!("absent");
    v["bins"][0]["mobs"][0] = v["bins"][0]["catalog"]["clips"][0].clone();
    let mut empty = v["bins"][0].clone();
    empty["summary"]["bin_uid"] = json!("empty");
    empty["mobs"] = json!([]);
    empty["catalog"]["clips"] = json!([]);
    empty["catalog"]["summary"]["master_count"] = json!(0);
    v["bins"].as_array_mut().unwrap().push(empty);
    for mode in ["metadata", "media", "sequences"] {
        let (mut s, _) = session(&v, media.clone());
        let out = s.execute("file.importAvidCatalog", params(mode)).unwrap();
        assert_eq!(out["bins"][1]["name"], "Sound Effects_2");
        assert_eq!(out["sequences"].as_array().unwrap().len(), usize::from(mode == "sequences"));
        let m = s.project.items.values().find_map(|i| i.as_media()).unwrap();
        if mode == "metadata" {
            assert!(m.offline);
            assert!(out["bound"].as_array().unwrap().is_empty());
        } else {
            let filmcraft_project::MediaRef::Components { channels, .. } = &m.media else { panic!("expected bound channels") };
            assert!(channels[0].path.is_none());
            assert!(channels[1].path.is_some());
            assert_eq!(out["bound"][0]["channels"], 2);
            assert!(!out["bound"][0]["warnings"].as_array().unwrap().is_empty());
        }
    }
}

#[test]
fn avid_import_combined_batches_more_than_64_masters_in_one_edit() {
    let (mut v, media) = fixture();
    let master = v["bins"][0]["catalog"]["clips"][0].clone();
    let clips: Vec<_> = (1..=65)
        .map(|i| {
            let mut c = master.clone();
            c["id"]["raw"] = json!(format!("{i:064x}"));
            c
        })
        .collect();
    v["bins"][0]["catalog"]["clips"] = json!(clips);
    v["bins"][0]["catalog"]["summary"]["master_count"] = json!(65);
    v["bins"][0]["mobs"] = json!(clips);
    let (mut s, _) = session(&v, media);
    let out = s.execute("file.importAvidCatalog", params("sequences")).unwrap();
    assert_eq!(out["bound"].as_array().unwrap().len(), 65);
    assert_eq!(s.history.undo.len(), 1);
    assert_eq!(s.project.items.len(), 65);
    assert!(matches!(&s.project.root.children[0], BinEntry::Bin(b) if b.children.len() == 65));
}

#[test]
#[ignore = "private reports supplied through environment; reads existing media only"]
fn avid_import_combined_local_matches_staged_conversion() {
    for key in ["FILMCRAFT_AVID_PHASE9_REPORT", "FILMCRAFT_AVID_PHASE10_REPORT", "FILMCRAFT_AVID_PHASE11_REPORT", "FILMCRAFT_AVID_PHASE12_REPORT"] {
        let path = std::env::var(key).expect("provide local report");
        let mut combined = Session::default();
        let out = combined.execute("file.importAvidCatalog", json!({"path":path,"mode":"sequences"})).unwrap();
        assert_eq!(combined.history.undo.len(), 1);
        let mut staged = Session::default();
        let bins = staged.execute("file.importAvidCatalog", json!({"path":path})).unwrap();
        let ids: Vec<_> = staged.project.items.keys().copied().collect();
        staged.execute("media.bindAvidMedia", json!({"items":ids})).unwrap();
        staged.execute("file.importAvidSequences", json!({"path":path,"bin":bins["bins"][0]["bin"]})).unwrap();
        assert_eq!(combined.project, staged.project, "{key}: must equal already-validated separate commands");
        assert_eq!(filmcraft_format::decode(&filmcraft_format::encode(&combined.project, false)).unwrap().project, *combined.project);
        eprintln!(
            "{key}: {} masters, {} sequence(s), identical native project, one undo step",
            out["bound"].as_array().unwrap().len(),
            out["sequences"].as_array().unwrap().len()
        );
    }
}

#[test]
fn avid_audit_binding_rejects_inconsistent_reference_evidence() {
    let (good, media) = fixture();
    for (field, value) in [
        ("source_id", json!({"raw":"ff".repeat(32),"byte_order":"big"})),
        ("source_track", json!(2)),
        ("start", json!(1)),
        ("length", json!(4799)),
        ("rate", json!({"numerator":96000,"denominator":1})),
    ] {
        let mut v = good.clone();
        v["bins"][0]["catalog"]["clips"][0]["tracks"][0]["component"][field] = value;
        v["bins"][0]["mobs"][0] = v["bins"][0]["catalog"]["clips"][0].clone();
        let (mut s, _) = session(&v, media.clone());
        let before = s.project.clone();
        assert!(s.execute("file.importAvidCatalog", params("media")).is_err(), "accepted inconsistent {field}");
        assert_eq!(s.project, before);
        assert!(s.history.undo.is_empty());
    }
}
#[test]
fn avid_audit_sequence_rejects_incomplete_composition_and_timecode() {
    let (good, media) = fixture();
    for target in ["composition", "timecode", "track"] {
        let mut v = good.clone();
        let c = &mut v["bins"][0]["mobs"][1];
        match target {
            "composition" => c["truncated"] = json!(true),
            "timecode" => c["tracks"][0]["component"]["truncated"] = json!(true),
            _ => c["tracks"][0]["truncated"] = json!(true),
        }
        let (mut s, _) = session(&v, media.clone());
        let before = s.project.clone();
        assert!(s.execute("file.importAvidCatalog", params("sequences")).is_err(), "accepted incomplete {target}");
        assert_eq!(s.project, before);
        assert!(s.history.undo.is_empty());
    }
}

#[test]
fn avid_audit_catalog_rejects_disagreement_with_decoded_master() {
    let (good, media) = fixture();
    for field in ["name", "user", "length", "attributes", "tracks"] {
        let mut v = good.clone();
        let c = &mut v["bins"][0]["catalog"]["clips"][0];
        match field {
            "name" => c["name"] = json!("unverified rename"),
            "user" => c["user"]["Scene"] = json!("unverified scene"),
            "length" => c["length"] = json!(4700),
            "attributes" => c["attributes"] = json!({"_CHANNEL_GROUP_LIST":"ST:A1A2,"}),
            _ => c["tracks"][0]["component"]["source_id"] = c["tracks"][1]["component"]["source_id"].clone(),
        }
        let (mut s, _) = session(&v, media.clone());
        let before = s.project.clone();
        assert!(s.execute("file.importAvidCatalog", params("metadata")).is_err(), "accepted conflicting decoded {field}");
        assert_eq!(s.project, before);
    }
}

fn with_audio_descriptors(mut v: Value) -> Value {
    let tracks = v["bins"][0]["catalog"]["clips"][0]["tracks"].as_array().unwrap().clone();
    for t in tracks {
        let id = t["references"][0]["source_id"].clone();
        let d = json!({"class":"PCMA","mob_kind":1,"is_omfi":0,"channels":1,"coding_format":1,
            "sample_rate":48000.0,"edit_rate":48000.0,"quantization_bits":16,"block_align":2,"average_bps":96000,"length":4800,
            "locator":{"class":"MSML","properties":{"mob_id":{"id_raw":id["raw"],"byte_order":id["byte_order"]}}}});
        v["bins"][0]["mobs"].as_array_mut().unwrap().push(json!({"kind":"SourceMob","live":true,"id":id,"descriptor":d}));
    }
    v
}
#[test]
fn offline_audio_import_retains_slots_timing_and_relinks_atomically() {
    let (v, media) = fixture();
    let v = with_audio_descriptors(v);
    let (mut s, _) = session(&v, BTreeMap::new());
    let before = s.project.clone();
    let preview = s.execute("file.previewAvidCatalog", params("sequences")).unwrap();
    assert_eq!(s.project, before);
    assert_eq!(preview["mediaBinding"], "offline_components");
    assert_eq!(preview["bound"][0]["offlineComponents"], 2);
    let imported = s.execute("file.importAvidCatalog", params("sequences")).unwrap();
    assert_eq!(imported, preview);
    let offline = s.project.clone();
    let item = ItemId(imported["bound"][0]["item"].as_u64().unwrap());
    assert_eq!(crate::relink::scan(&s.project, &*s.services).len(), 2);
    assert_eq!(filmcraft_format::decode(&filmcraft_format::encode(&offline, false)).unwrap().project, *offline);
    let sequence = ItemId(imported["sequences"][0]["item"].as_u64().unwrap());
    let timeline = s.project.sequence(sequence).unwrap().clone();
    s.services = session(&v, media).1;
    s.execute(
        "media.relinkComponents",
        json!({"bindings":[
            {"item":item,"component":{"kind":"audio","channel":0},"path":"/combined/A1.mxf"},
            {"item":item,"component":{"kind":"audio","channel":1},"path":"/combined/A2.mxf"}
        ]}),
    )
    .unwrap();
    assert_eq!(s.project.sequence(sequence).unwrap(), &timeline);
    assert!(crate::relink::scan(&s.project, &*s.services).is_empty());
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.project, offline);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.project, before);
}
#[test]
fn offline_import_requires_validated_descriptors_and_explicit_policy() {
    let (v, _) = fixture();
    let good = with_audio_descriptors(v);
    for case in ["strict", "invalid_policy", "no_descriptor", "duplicate", "inactive", "locator", "short", "rate", "duration"] {
        let mut v = good.clone();
        let mut p = params("sequences");
        match case {
            "strict" => p["missingMedia"] = json!("error"),
            "invalid_policy" => p["missingMedia"] = json!(true),
            "no_descriptor" => {
                v["bins"][0]["mobs"].as_array_mut().unwrap().truncate(2);
            }
            "duplicate" => {
                let duplicate = v["bins"][0]["mobs"][2].clone();
                v["bins"][0]["mobs"].as_array_mut().unwrap().push(duplicate);
            }
            "inactive" => v["bins"][0]["mobs"][2]["live"] = json!(false),
            "locator" => v["bins"][0]["mobs"][2]["descriptor"]["locator"]["properties"]["mob_id"]["id_raw"] = json!("aa".repeat(32)),
            "short" => v["bins"][0]["mobs"][2]["id"]["raw"] = json!("aa".repeat(12)),
            "rate" => v["bins"][0]["mobs"][2]["descriptor"]["sample_rate"] = json!(44100.0),
            _ => v["bins"][0]["mobs"][2]["descriptor"]["length"] = json!(1),
        }
        let (mut s, _) = session(&v, BTreeMap::new());
        let before = s.project.clone();
        assert!(s.execute("file.importAvidCatalog", p).is_err(), "accepted {case}");
        assert_eq!(s.project, before);
        assert!(s.history.undo.is_empty());
    }
    // Inactive history does not make the unique live descriptor ambiguous.
    let mut v = good;
    let mut old = v["bins"][0]["mobs"][2].clone();
    old["live"] = json!(false);
    v["bins"][0]["mobs"].as_array_mut().unwrap().push(old);
    session(&v, BTreeMap::new()).0.execute("file.importAvidCatalog", params("sequences")).unwrap();
}
#[test]
fn offline_fallback_does_not_hide_existing_corrupt_media() {
    let (v, mut media) = fixture();
    let v = with_audio_descriptors(v);
    media.insert("/combined/A1.mxf".into(), vec![0; 128]);
    let (mut s, _) = session(&v, media);
    let before = s.project.clone();
    assert!(s.execute("file.importAvidCatalog", params("sequences")).is_err());
    assert_eq!(s.project, before);
}

#[test]
#[ignore = "private Phase 12 report supplied via environment; never modifies originals"]
fn offline_phase12_import_and_relink_preserve_exact_timeline() {
    struct Offline;
    impl Services for Offline {
        fn read_file(&self, path: &str) -> std::io::Result<Vec<u8>> {
            FsServices.read_file(path)
        }
        fn file_size(&self, path: &str) -> std::io::Result<u64> {
            if path.to_ascii_lowercase().ends_with(".mxf") {
                Err(std::io::Error::new(std::io::ErrorKind::NotFound, "test volume unavailable"))
            } else {
                FsServices.file_size(path)
            }
        }
        fn reader(&self, _: &str) -> Option<std::io::Result<filmcraft_media::SharedReader>> {
            Some(Err(std::io::Error::new(std::io::ErrorKind::NotFound, "test volume unavailable")))
        }
        fn write_file(&self, _: &str, _: &[u8]) -> std::io::Result<()> {
            Err(std::io::Error::other("read only"))
        }
    }
    let path = std::env::var("FILMCRAFT_AVID_PHASE12_REPORT").unwrap();
    let p = json!({"path":path,"mode":"sequences"});
    let mut online = Session::default();
    let online_out = online.execute("file.importAvidCatalog", p.clone()).unwrap();
    let mut s = Session::new(Arc::new(Offline));
    let out = s.execute("file.importAvidCatalog", p).unwrap();
    assert_eq!(out["mediaBinding"], "offline_components");
    assert_eq!(crate::relink::scan(&s.project, &*s.services).len(), 6);
    let sequence = ItemId(out["sequences"][0]["item"].as_u64().unwrap());
    let online_sequence = ItemId(online_out["sequences"][0]["item"].as_u64().unwrap());
    assert_eq!(s.project.sequence(sequence), online.project.sequence(online_sequence));
    assert_eq!(s.project.sequence(sequence).unwrap().duration(), filmcraft_time::Tick(filmcraft_time::FrameRate::FPS_24.frame_duration().0 * 152));
    let offline = s.project.clone();
    assert_eq!(filmcraft_format::decode(&filmcraft_format::encode(&offline, false)).unwrap().project, *offline);
    // Isolated user-test project; originals and their media are never moved.
    let mut disposable = (*offline).clone();
    for item in disposable.items.values_mut() {
        if let Some(m) = item.as_media_mut()
            && let filmcraft_project::MediaRef::Components { channels, picture } = &mut m.media
        {
            for path in channels.iter_mut().map(|c| &mut c.path).chain(picture.iter_mut().map(|c| &mut c.path)) {
                if let Some(old) = path.take() {
                    *path = Some(format!("/unmounted{old}"));
                }
            }
        }
    }
    std::fs::write(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/Avid Offline Import Test.fcproj"),
        filmcraft_format::encode(&disposable, false),
    )
    .unwrap();
    let mut bindings = Vec::new();
    for (id, item) in &s.project.items {
        if let Some(m) = item.as_media()
            && let filmcraft_project::MediaRef::Components { channels, picture } = &m.media
        {
            for (i, c) in channels.iter().enumerate() {
                bindings.push(json!({"item":id,"component":{"kind":"audio","channel":i},"path":c.path}));
            }
            if let Some(c) = picture {
                assert!(c.format_pending);
                bindings.push(json!({"item":id,"component":{"kind":"picture"},"path":c.path}));
            }
        }
    }
    s.services = Arc::new(FsServices);
    // Existing paths must not bypass pending picture format verification.
    assert_eq!(crate::relink::scan(&s.project, &*s.services).len(), 2);
    let picture_ids: Vec<_> = bindings.iter().filter(|b| b["component"]["kind"] == "picture").map(|b| b["item"].clone()).collect();
    for id in &picture_ids {
        let status = s.execute("media.status", json!({"item":id})).unwrap();
        assert_ne!(status[0]["status"], "online", "pending picture must agree with playback's offline slate");
        let picture = status[0]["components"].as_array().unwrap().iter().find(|c| c["kind"] == "picture").unwrap();
        assert_eq!(picture["present"], false);
        assert_eq!(picture["formatPending"], true);
    }
    for wrong in ["rate", "aspect", "width"] {
        let mut invalid = (*offline).clone();
        let m = invalid.items.values_mut().filter_map(|i| i.as_media_mut()).find(|m| m.info.video.is_some()).unwrap();
        let video = m.info.video.as_mut().unwrap();
        match wrong {
            "rate" => video.frame_rate = filmcraft_time::FrameRate::new(25, 1),
            "aspect" => video.par = (2, 1),
            _ => video.width = 1280,
        }
        let mut rejected = Session { project: Arc::new(invalid), ..Default::default() };
        let before = rejected.project.clone();
        assert!(rejected.execute("media.relinkComponents", json!({"bindings":bindings})).is_err(), "accepted incompatible {wrong}");
        assert_eq!(rejected.project, before);
        assert!(rejected.history.undo.is_empty());
    }
    s.execute("media.relinkComponents", json!({"bindings":bindings})).unwrap();
    for id in &picture_ids {
        assert_eq!(s.execute("media.status", json!({"item":id})).unwrap()[0]["status"], "online");
    }
    for (id, item) in &s.project.items {
        assert_eq!(item.kind, online.project.item(*id).unwrap().kind, "relinked contents differ for {}", item.name);
    }
    assert!(crate::relink::scan(&s.project, &*s.services).is_empty());
    assert_eq!(s.project.sequence(sequence), offline.sequence(sequence));
    assert_eq!(filmcraft_format::decode(&filmcraft_format::encode(&s.project, false)).unwrap().project, *s.project);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.project, offline);
    s.execute("edit.redo", json!({})).unwrap();
    assert!(crate::relink::scan(&s.project, &*s.services).is_empty());
    eprintln!("Phase 12: six offline components, identical 152-frame timeline, hydrated media identical to online import, undo/redo/save/reopen pass");
}

#[test]
fn offline_import_reports_permission_failure_instead_of_absence() {
    struct Denied(Arc<Input>);
    impl Services for Denied {
        fn read_file(&self, path: &str) -> std::io::Result<Vec<u8>> {
            self.0.read_file(path)
        }
        fn file_size(&self, path: &str) -> std::io::Result<u64> {
            if path.ends_with(".mxf") { Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "access denied")) } else { self.0.file_size(path) }
        }
        fn write_file(&self, _: &str, _: &[u8]) -> std::io::Result<()> {
            panic!("import must not write")
        }
    }
    let (v, media) = fixture();
    let v = with_audio_descriptors(v);
    let (_, input) = session(&v, media);
    let mut s = Session::new(Arc::new(Denied(input)));
    let before = s.project.clone();
    let error = s.execute("file.importAvidCatalog", params("media")).unwrap_err().to_string();
    assert!(error.contains("access denied"), "{error}");
    assert_eq!(s.project, before);
    assert!(s.history.undo.is_empty());
}

#[test]
fn best_effort_sequence_bypasses_effects_and_marks_failed_segments() {
    let (mut v, media) = fixture();
    let source = v["bins"][0]["mobs"][1]["tracks"][1]["component"]["components"][1].clone();
    let mut second = v["bins"][0]["mobs"][1].clone();
    second["name"] = json!("Needs placeholder");
    second["id"]["raw"] = json!("03".repeat(32));
    second["tracks"][1]["component"]["components"][1]["class"] = json!("UNKNOWN");
    v["bins"][0]["mobs"].as_array_mut().unwrap().push(second);
    let mut wrapper = source.clone();
    wrapper["class"] = json!("TKFX");
    wrapper["unsupported"] = json!("component_semantics");
    wrapper["properties"] = json!({"effect_id":"UNKNOWN_EFFECT"});
    wrapper["input_tracks"] = json!([{"index":1,"component":source}]);
    v["bins"][0]["mobs"][1]["tracks"][1]["component"]["components"][1] = wrapper;
    let mut unreadable = v["bins"][0]["mobs"][1].clone();
    unreadable["name"] = json!("Uninterpretable tracks");
    unreadable["id"]["raw"] = json!("04".repeat(32));
    unreadable["tracks"][1]["index"] = json!(0);
    v["bins"][0]["mobs"].as_array_mut().unwrap().push(unreadable);
    let (s, _) = session(&v, media);
    let prepared = crate::avid_catalog::prepare_document(
        &s.project,
        &*s.services,
        &serde_json::to_vec(&v).unwrap(),
        crate::avid_catalog::ImportOptions { mode: filmcraft_interchange::avid_catalog::TransferMode::Sequences, allow_missing: true, best_effort: true },
        None,
        &|| Ok(()),
    )
    .unwrap();
    assert_eq!(prepared.result["sequences"].as_array().unwrap().len(), 3);
    assert!(prepared.result["errors"].to_string().contains("Entire sequence represented"));
    assert!(prepared.result["warnings"].to_string().contains("effect bypassed"));
    assert!(prepared.result["errors"].to_string().contains("offline placeholder"));
    let a = prepared.project.sequence(ItemId(prepared.result["sequences"][0]["item"].as_u64().unwrap())).unwrap();
    let b = prepared.project.sequence(ItemId(prepared.result["sequences"][1]["item"].as_u64().unwrap())).unwrap();
    assert_eq!(a.duration(), Tick::from_units(2, 24));
    assert_eq!(a.duration(), b.duration());
    assert_eq!(a.audio_tracks[1].items[0].start, b.audio_tracks[1].items[0].start);
    let missing = prepared.project.item(b.audio_tracks[1].items[0].item).unwrap();
    assert!(missing.as_media().unwrap().offline);
    assert!(missing.metadata.contains_key("filmcraft:avid-import-error"));
    assert_eq!(filmcraft_format::decode(&filmcraft_format::encode(&prepared.project, false)).unwrap().project, prepared.project);
    assert!(s.history.undo.is_empty());
}

#[test]
fn best_effort_picture_placeholder_has_finite_saved_motion_points() {
    let (mut v, media) = fixture();
    let c = &mut v["bins"][0]["mobs"][1]["tracks"][1]["component"];
    c["media_kind"] = json!(1);
    for child in c["components"].as_array_mut().unwrap() {
        child["media_kind"] = json!(1);
    }
    c["components"][1]["class"] = json!("UNKNOWN_PICTURE");
    let (s, _) = session(&v, media);
    let p = crate::avid_catalog::prepare_document(
        &s.project,
        &*s.services,
        &serde_json::to_vec(&v).unwrap(),
        crate::avid_catalog::ImportOptions { mode: filmcraft_interchange::avid_catalog::TransferMode::Sequences, allow_missing: true, best_effort: true },
        None,
        &|| Ok(()),
    )
    .unwrap()
    .project;
    assert_eq!(filmcraft_format::decode(&filmcraft_format::encode(&p, false)).unwrap().project, p);
}

#[test]
fn best_effort_sequences_isolate_bad_entries_without_reviving_history() {
    let (mut v, media) = fixture();
    let good = v["bins"][0]["mobs"][1].clone();
    let mut inactive = good.clone();
    inactive["name"] = json!("Inactive history");
    inactive["live"] = json!(false);
    // Historical duplicates must not make the current identity ambiguous.
    let mut malformed = good.clone();
    malformed["name"] = json!("Invalid identity");
    malformed["id"]["raw"] = json!("bad");
    let mut unnamed = good.clone();
    unnamed["name"] = Value::Null;
    unnamed["id"]["raw"] = json!("04".repeat(32));
    let mut duplicate = good.clone();
    duplicate["name"] = json!("Ambiguous sequence");
    duplicate["id"]["raw"] = json!("05".repeat(32));
    v["bins"][0]["mobs"].as_array_mut().unwrap().extend([inactive, malformed, unnamed, duplicate.clone(), duplicate]);
    let (s, _) = session(&v, media);
    let prepared = crate::avid_catalog::prepare_document(
        &s.project,
        &*s.services,
        &serde_json::to_vec(&v).unwrap(),
        crate::avid_catalog::ImportOptions { mode: filmcraft_interchange::avid_catalog::TransferMode::Sequences, allow_missing: true, best_effort: true },
        None,
        &|| Ok(()),
    )
    .unwrap();
    assert_eq!(prepared.result["sequences"].as_array().unwrap().len(), 1, "only the valid live sequence should import: {}", prepared.result);
    assert_eq!(prepared.result["sequences"][0]["name"], "Trimmed");
    let errors = prepared.result["errors"].to_string();
    for reason in ["inactive composition", "Invalid identity", "invalid name", "duplicate composition identity"] {
        assert!(errors.contains(reason), "missing {reason}: {errors}");
    }
    assert!(!prepared.project.items.values().any(|item| item.name == "Unconverted Avid segment"));
    assert_eq!(filmcraft_format::decode(&filmcraft_format::encode(&prepared.project, false)).unwrap().project, prepared.project);
}
