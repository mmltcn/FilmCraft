use super::*;
use crate::Services;
use std::sync::atomic::{AtomicUsize, Ordering};
fn text(b: &mut Vec<u8>, s: &str) {
    b.extend_from_slice(&(s.len() as u16).to_le_bytes());
    b.extend_from_slice(s.as_bytes());
}
fn identity(b: &mut Vec<u8>) {
    b.push(0x41);
    b.extend_from_slice(&12u32.to_le_bytes());
    b.extend_from_slice(&[6, 10, 43, 52, 1, 1, 1, 5, 1, 1, 15, 16]);
    for n in [19, 0, 0, 0] {
        b.extend_from_slice(&[0x44, n]);
    }
    for (tag, n) in [(0x48, 4), (0x46, 2), (0x46, 2)] {
        b.push(tag);
        b.extend(std::iter::repeat_n(1, n));
    }
    b.push(0x41);
    b.extend_from_slice(&8u32.to_le_bytes());
    b.extend_from_slice(&[1; 8]);
}
fn common(name: Option<&str>, attrs: u32) -> Vec<u8> {
    let mut b = vec![2, 3];
    b.extend_from_slice(&[0; 8]);
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&24u32.to_le_bytes());
    b.extend_from_slice(&[0; 2]);
    if let Some(name) = name {
        text(&mut b, name);
    } else {
        b.extend_from_slice(&[255; 2]);
    }
    b.extend_from_slice(&[255; 2]);
    b.extend_from_slice(&attrs.to_le_bytes());
    b.extend_from_slice(&[0; 8]);
    b.extend_from_slice(&[1, 1, 0x48, 0, 0, 0, 0]);
    b
}
fn fixture() -> Vec<u8> {
    let mut user = vec![2, 1];
    user.extend_from_slice(&1u32.to_le_bytes());
    user.extend_from_slice(&2u32.to_le_bytes());
    text(&mut user, "Scene");
    text(&mut user, "Native test");
    user.push(3);
    let mut attrs = vec![2, 1];
    attrs.extend_from_slice(&1u32.to_le_bytes());
    attrs.extend_from_slice(&3u32.to_le_bytes());
    text(&mut attrs, "_USER");
    attrs.extend_from_slice(&1u32.to_le_bytes());
    attrs.push(3);
    let mut clip = common(None, 0);
    clip.extend_from_slice(&[2, 1]);
    clip.extend_from_slice(&120u32.to_le_bytes());
    clip.extend_from_slice(&[2, 3]);
    clip.extend_from_slice(&[0; 8]);
    clip.extend_from_slice(&1u16.to_le_bytes());
    clip.extend_from_slice(&0u32.to_le_bytes());
    clip.extend_from_slice(&[1, 1]);
    identity(&mut clip);
    clip.push(3);
    let mut mob = common(Some("Audio"), 2);
    mob.extend_from_slice(&[2, 8, 0]);
    mob.extend_from_slice(&120u32.to_le_bytes());
    mob.extend_from_slice(&[0; 4]);
    mob.extend_from_slice(&1u32.to_le_bytes());
    mob.extend_from_slice(&[5, 0, 1, 0]);
    mob.extend_from_slice(&3u32.to_le_bytes());
    mob.extend_from_slice(&[1, 1, 0x45, 0, 0, 2, 2]);
    mob.extend_from_slice(&[1; 8]);
    mob.extend_from_slice(&[0; 4]);
    mob.push(2);
    mob.extend_from_slice(&[0; 8]);
    mob.extend_from_slice(&[1, 2]);
    identity(&mut mob);
    mob.push(3);
    let mut root = vec![2, 14];
    root.extend_from_slice(&[0; 12]);
    root.extend_from_slice(&1u16.to_le_bytes());
    root.extend_from_slice(&4u32.to_le_bytes());
    root.extend_from_slice(&[0; 8]);
    root.push(1);
    root.extend_from_slice(&[0; 103]);
    let mut out = b"\x06\0DomainDJBO\x07\0AObjDoc\x04\0\0".to_vec();
    out.extend_from_slice(&5u32.to_le_bytes());
    out.extend_from_slice(&5u32.to_le_bytes());
    out.extend_from_slice(&[0; 38]);
    for (tag, b) in [(*b"ATTR", user), (*b"ATTR", attrs), (*b"SCLP", clip), (*b"CMPO", mob), (*b"ABIN", root)] {
        out.extend(tag.into_iter().rev());
        out.extend_from_slice(&(b.len() as u32).to_le_bytes());
        out.extend_from_slice(&b);
    }
    out
}
struct Input {
    bytes: Vec<u8>,
    reads: AtomicUsize,
    changing: bool,
}
impl Services for Input {
    fn read_file(&self, _: &str) -> std::io::Result<Vec<u8>> {
        Ok(self.bytes.clone())
    }
    fn file_size(&self, _: &str) -> std::io::Result<u64> {
        Ok(self.bytes.len() as u64)
    }
    fn read_range(&self, _: &str, offset: u64, len: usize) -> std::io::Result<Vec<u8>> {
        let mut bytes = self.bytes[offset as usize..(offset as usize + len).min(self.bytes.len())].to_vec();
        if self.changing && self.reads.fetch_add(1, Ordering::Relaxed) > 0 {
            bytes[0] ^= 1;
        }
        Ok(bytes)
    }
    fn write_file(&self, _: &str, _: &[u8]) -> std::io::Result<()> {
        panic!("native import must not write sources");
    }
}
fn session(changing: bool) -> Session {
    Session::new(Arc::new(Input { bytes: fixture(), reads: AtomicUsize::new(0), changing }))
}
fn prepare(s: &mut Session) -> Value {
    s.execute("file.prepareAvidBin", json!({"path":"Sound Effects.avb","mode":"metadata","mediaFolders":[],"wait":true})).unwrap()
}
#[test]
fn native_preview_commit_suffix_undo_and_saved_metadata() {
    let mut s = session(false);
    let before = s.project.clone();
    let state = serde_json::to_value(&s.state).unwrap();
    let job = prepare(&mut s);
    let preview = s.execute("file.avidBinPreparation", job.clone()).unwrap();
    assert_eq!(preview["report"]["bins"][0]["clips"], 1);
    assert_eq!(s.project, before);
    assert_eq!(serde_json::to_value(&s.state).unwrap(), state);
    assert!(s.history.undo.is_empty());
    let out = s.execute("file.importAvidBin", job.clone()).unwrap();
    assert_eq!(out["bins"][0]["name"], "Sound Effects");
    let item = s.project.items.values().next().unwrap();
    assert_eq!(item.name, "Audio");
    assert_eq!(item.metadata["Scene"], "Native test");
    assert!(item.as_media().unwrap().offline);
    let after = s.project.clone();
    assert_eq!(s.history.undo.len(), 1);
    assert_eq!(filmcraft_format::decode(&filmcraft_format::encode(&after, false)).unwrap().project, *after);
    assert!(s.execute("file.importAvidBin", job).is_err());
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.project, before);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(s.project, after);
    let job = prepare(&mut s);
    let out = s.execute("file.importAvidBin", job).unwrap();
    assert_eq!(out["bins"][0]["name"], "Sound Effects_2");
}
#[test]
fn cancellation_stale_projects_and_same_size_source_changes_cannot_commit() {
    let mut s = session(false);
    let job = prepare(&mut s);
    s.execute("jobs.cancel", job.clone()).unwrap();
    assert!(s.execute("file.importAvidBin", job).is_err());
    assert!(s.history.undo.is_empty());
    let job = prepare(&mut s);
    s.execute("file.newBin", json!({"name":"Changed"})).unwrap();
    let before = s.project.clone();
    assert!(s.execute("file.importAvidBin", job).is_err());
    assert_eq!(s.project, before);
    let mut changing = session(true);
    let job = prepare(&mut changing);
    assert!(changing.execute("file.avidBinPreparation", job.clone()).unwrap_err().to_string().contains("changed"));
    assert!(changing.execute("file.importAvidBin", job).is_err());
    assert!(changing.history.undo.is_empty());
}
#[test]
fn hostile_inputs_and_malformed_bins_never_edit() {
    for p in [
        Value::Null,
        json!({}),
        json!({"path":12}),
        json!({"path":"\0"}),
        json!({"path":"x","parent":-1}),
        json!({"path":"x","wait":"yes"}),
        json!({"path":"x","mode":false}),
        json!({"path":"x","mediaFolder":12}),
        json!({"path":"x","mediaFolders":"folder"}),
        json!({"path":"x","mediaFolders":[null]}),
        json!({"path":"x","mediaFolders":["relative"]}),
        json!({"path":"x","mediaFolders":vec!["/";33]}),
        json!({"path":"x","mediaFolder":"/","mediaFolders":[]}),
    ] {
        let mut s = session(false);
        assert!(s.execute("file.prepareAvidBin", p).is_err());
        assert!(s.history.undo.is_empty());
        assert!(s.jobs.is_empty());
    }
    let mut s = Session::new(Arc::new(Input { bytes: vec![0; 30], reads: AtomicUsize::new(0), changing: false }));
    let job = prepare(&mut s);
    assert!(s.execute("file.importAvidBin", job).is_err());
    assert!(s.history.undo.is_empty());
}

#[test]
#[ignore = "explicit private AVB manifest; never discovers additional folders"]
fn native_graph_imports_tested_managed_sequences_offline() {
    let manifest: Value = serde_json::from_slice(&std::fs::read(std::env::var("FILMCRAFT_AVB_CORPUS_MANIFEST").unwrap()).unwrap()).unwrap();
    let mut count = 0;
    for entry in manifest.as_array().unwrap() {
        let path = entry["path"].as_str().unwrap();
        if !(9..=12).any(|phase| path.contains(&format!("/Phase {phase}/"))) {
            continue;
        }
        let bytes = std::fs::read(path).unwrap();
        let document = avid_catalog::native_document(&bytes, path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let bytes = serde_json::to_vec(&document).unwrap();
        let prepared = crate::avid_catalog::prepare_document(
            &Project::default(),
            &crate::FsServices,
            &bytes,
            crate::avid_catalog::ImportOptions { mode: avid_catalog::TransferMode::Sequences, allow_missing: true, best_effort: false },
            None,
            &|| Ok(()),
        )
        .unwrap_or_else(|e| panic!("{path}: {e}"));
        assert_eq!(prepared.result["sequences"].as_array().unwrap().len(), 1, "{path}");
        assert_eq!(filmcraft_format::decode(&filmcraft_format::encode(&prepared.project, false)).unwrap().project, prepared.project);
        eprintln!("native {path}: {}", prepared.result["sequences"]);
        count += 1;
    }
    assert_eq!(count, 4);
}

#[test]
#[ignore = "explicit private Phase 12 report; native AVB import and media scan"]
fn native_phase12_matches_catalog_and_links_in_one_transaction() {
    let report_path = std::env::var("FILMCRAFT_AVID_PHASE12_REPORT").unwrap();
    let report: Value = serde_json::from_slice(&std::fs::read(&report_path).unwrap()).unwrap();
    let path = report["bins"][0]["path"].as_str().unwrap();
    let clips = report["bins"][0]["catalog"]["clips"].as_array().unwrap();
    let folders: std::collections::BTreeSet<_> = clips
        .iter()
        .flat_map(|c| c["tracks"].as_array().unwrap())
        .flat_map(|t| t["references"].as_array().unwrap())
        .flat_map(|r| r["locations"].as_array().unwrap())
        .map(|l| l["folder"].as_str().unwrap())
        .collect();
    assert_eq!(folders.len(), 1);
    let folder = *folders.first().unwrap();
    let mut native = Session::default();
    let before = native.project.clone();
    let job = native.execute("file.prepareAvidBin", json!({"path":path,"mode":"sequences","mediaFolders":[folder,folder],"wait":true})).unwrap();
    let preview = native.execute("file.avidBinPreparation", job.clone()).unwrap();
    assert_eq!(native.project, before);
    assert!(native.history.undo.is_empty());
    assert_eq!(preview["report"]["mediaSearch"]["linkedComponents"], 6, "{preview}");
    assert_eq!(preview["report"]["mediaSearch"]["offlineComponents"], 0);
    let result = native.execute("file.importAvidBin", job).unwrap();
    assert_eq!(result, preview["report"]);
    let mut reference = Session::default();
    reference.execute("file.importAvidCatalog", json!({"path":report_path,"mode":"sequences"})).unwrap();
    assert_eq!(native.project.items.len(), reference.project.items.len());
    for item in native.project.items.values() {
        let expected = reference.project.items.values().find(|i| i.name == item.name).unwrap();
        assert_eq!(item.kind, expected.kind, "{}", item.name);
    }
    assert!(crate::relink::scan(&native.project, &*native.services).is_empty());
    assert_eq!(native.history.undo.len(), 1);
    let after = native.project.clone();
    assert_eq!(filmcraft_format::decode(&filmcraft_format::encode(&after, false)).unwrap().project, *after);
    native.execute("edit.undo", json!({})).unwrap();
    assert_eq!(native.project, before);
    native.execute("edit.redo", json!({})).unwrap();
    assert_eq!(native.project, after);
    std::fs::write(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/Avid Native Phase 12.fcproj"), filmcraft_format::encode(&after, false))
        .unwrap();
}

#[test]
#[ignore = "explicit private manifest; native large-bin metadata benchmark"]
fn native_large_bin_metadata_roundtrip() {
    let manifest: Value = serde_json::from_slice(&std::fs::read(std::env::var("FILMCRAFT_AVB_CORPUS_MANIFEST").unwrap()).unwrap()).unwrap();
    let path = manifest.as_array().unwrap().iter().filter_map(|e| e["path"].as_str()).find(|p| p.contains("/Phase 6/")).unwrap();
    let mut s = Session::default();
    let start = web_time::Instant::now();
    let job = s.execute("file.prepareAvidBin", json!({"path":path,"mode":"metadata","wait":true})).unwrap();
    let out = s.execute("file.importAvidBin", job).unwrap();
    assert_eq!(out["bins"][0]["clips"], 1953);
    let import_ms = start.elapsed().as_millis();
    let bytes = filmcraft_format::encode(&s.project, false);
    assert_eq!(filmcraft_format::decode(&bytes).unwrap().project, *s.project);
    assert_eq!(s.history.undo.len(), 1);
    eprintln!("native Phase 6: 1953 masters, import {import_ms} ms, saved {} bytes", bytes.len());
}

#[test]
#[ignore = "private Phase 12 report; mutate only an in-memory AVB copy"]
fn native_unknown_effect_imports_partial_sequence_with_diagnostics() {
    let report: Value = serde_json::from_slice(&std::fs::read(std::env::var("FILMCRAFT_AVID_PHASE12_REPORT").unwrap()).unwrap()).unwrap();
    let path = report["bins"][0]["path"].as_str().unwrap();
    let mut bytes = std::fs::read(path).unwrap();
    let needle = b"EFF_TIMEWARP.MOTION_CTL";
    let offset = bytes.windows(needle.len()).position(|v| v == needle).unwrap();
    bytes[offset..offset + 3].copy_from_slice(b"BAD");
    let mut s = Session::new(Arc::new(Input { bytes, reads: AtomicUsize::new(0), changing: false }));
    let before = s.project.clone();
    let job = s.execute("file.prepareAvidBin", json!({"path":"Debug.avb","wait":true})).unwrap();
    let preview = s.execute("file.avidBinPreparation", job.clone()).unwrap();
    assert_eq!(preview["report"]["sequences"].as_array().unwrap().len(), 1);
    assert!(preview["report"]["errors"].to_string().contains("placeholder"), "{preview}");
    assert_eq!(s.project, before);
    let out = s.execute("file.importAvidBin", job).unwrap();
    let sequence = s.project.sequence(filmcraft_project::ItemId(out["sequences"][0]["item"].as_u64().unwrap())).unwrap();
    assert_eq!(sequence.duration(), filmcraft_time::Tick::from_units(152, 24));
    assert_eq!(sequence.video_tracks[0].items.len(), 2);
    assert_eq!(sequence.video_tracks[0].items[1].start, filmcraft_time::Tick::from_units(106, 24));
    let placeholder = s.project.item(sequence.video_tracks[0].items[1].item).unwrap();
    assert!(placeholder.as_media().unwrap().offline);
    assert!(placeholder.metadata.contains_key("filmcraft:avid-import-error"));
    let after = s.project.clone();
    assert_eq!(filmcraft_format::decode(&filmcraft_format::encode(&after, false)).unwrap().project, *after);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.project, before);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(s.project, after);
}

#[test]
#[ignore = "explicit private Phase 6 manifest; all-content native import"]
fn native_large_bin_default_attempts_every_master() {
    let manifest: Value = serde_json::from_slice(&std::fs::read(std::env::var("FILMCRAFT_AVB_CORPUS_MANIFEST").unwrap()).unwrap()).unwrap();
    let path = manifest.as_array().unwrap().iter().filter_map(|e| e["path"].as_str()).find(|p| p.contains("/Phase 6/")).unwrap();
    let mut s = Session::default();
    let started = web_time::Instant::now();
    let job = s.execute("file.prepareAvidBin", json!({"path":path,"wait":true})).unwrap();
    let preview = s.execute("file.avidBinPreparation", job.clone()).unwrap();
    assert_eq!(preview["report"]["bins"][0]["clips"], 1953);
    assert!(s.history.undo.is_empty());
    let result = s.execute("file.importAvidBin", job).unwrap();
    assert_eq!(s.project.items.len(), 1953);
    let mut reasons = std::collections::BTreeMap::new();
    for error in result["errors"].as_array().unwrap().iter().filter_map(Value::as_str) {
        let reason = error.split_once(": media remains offline: ").map_or(error, |(_, reason)| reason);
        *reasons.entry(reason).or_insert(0usize) += 1;
    }
    eprintln!("large-bin offline binding reasons: {reasons:?}");
    assert_eq!(s.history.undo.len(), 1);
    assert_eq!(result["bound"].as_array().unwrap().len() + result["errors"].as_array().unwrap().len(), 1953);
    assert_eq!(filmcraft_format::decode(&filmcraft_format::encode(&s.project, false)).unwrap().project, *s.project);
    eprintln!(
        "native Phase 6 all-content: {} ms; {} bound offline; {} diagnosed unbound",
        started.elapsed().as_millis(),
        result["bound"].as_array().unwrap().len(),
        result["errors"].as_array().unwrap().len()
    );
}
