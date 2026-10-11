//! Development bridge from the standalone Avid reader's JSON catalog.
//!
//! No AVB/PMR/MDB binary decoding or file I/O. Imported clips are metadata-only offline
//! placeholders until FilmCraft supports their independently located media components.
use std::collections::{BTreeMap, BTreeSet};

use filmcraft_media::{MediaInfo, MediaKind};
use filmcraft_project::{BinEntry, BinId, Interpretation, ItemKind, Label, MediaClip, MediaRef, Project};
use filmcraft_time::{TICKS_PER_SECOND, Tick};
use serde::Serialize;
use serde_json::Value;

use crate::{Error, Result};

mod native;
pub use native::{native_document, plan_native};

pub const MAX_BYTES: usize = 256 * 1024 * 1024;
const MAX_BINS: usize = 1024;
const MAX_CLIPS: usize = 100_000;
const MAX_MOBS: usize = 200_000;
const MAX_TEXT: usize = 4096;
/// All supported source fields are copied here, not linked to the AVB/report.
pub const SOURCE_KEY: &str = "filmcraft:avid-source";

#[derive(Clone, Debug, Serialize)]
pub struct Plan {
    bins: Vec<BinPlan>,
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug, Serialize)]
struct BinPlan {
    pub name: String,
    pub uid: String,
    pub clips: Vec<ClipPlan>,
    pub compositions: usize,
}
#[derive(Clone, Debug, Serialize)]
struct ClipPlan {
    pub name: String,
    pub duration: Tick,
    pub evidence: Value,
    pub source_descriptors: Vec<Value>,
    pub picture_rate_evidence: Option<Value>,
}
#[derive(Debug, Serialize)]
pub struct ImportedBin {
    pub bin: BinId,
    pub name: String,
    pub clips: usize,
    pub compositions: usize,
}

/// Requested transfer stages. Media/sequence stages must be verified by the engine before commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferMode {
    Metadata,
    Media,
    Sequences,
}
fn bad(message: impl Into<String>) -> Error {
    Error::Other(format!("Avid catalog: {}", message.into()))
}
fn array<'a>(v: &'a Value, name: &str) -> Result<&'a Vec<Value>> {
    v.get(name).and_then(Value::as_array).ok_or_else(|| bad(format!("missing or invalid {name}")))
}
fn text<'a>(v: &'a Value, name: &str) -> Result<&'a str> {
    let s = v.get(name).and_then(Value::as_str).ok_or_else(|| bad(format!("missing or invalid {name}")))?;
    if s.len() > MAX_TEXT || s.contains('\0') {
        return Err(bad(format!("invalid {name} text")));
    }
    Ok(s)
}
/// The catalog contract carries binary IDs with explicit byte order, never display URNs.
/// Normalize the material-number fields for duplicate detection; preserve raw evidence.
fn identity_key(m: &Value) -> Result<Vec<u8>> {
    let id = m.get("id").ok_or_else(|| bad("missing source identity"))?;
    let raw = text(id, "raw")?;
    let order = text(id, "byte_order")?;
    if !matches!(raw.len(), 24 | 64) || !raw.bytes().all(|b| b.is_ascii_hexdigit()) || !matches!(order, "little" | "big") {
        return Err(bad("invalid source identity"));
    }
    let mut bytes = raw
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let digits = std::str::from_utf8(pair).map_err(|_| bad("invalid identity hex"))?;
            u8::from_str_radix(digits, 16).map_err(|_| bad("invalid identity hex"))
        })
        .collect::<Result<Vec<_>>>()?;
    if bytes.len() == 12 {
        if order != "little" || bytes.get(..4) != Some(&[42, 0, 0, 0]) {
            return Err(bad("unsupported short source identity"));
        }
    } else if order == "little" {
        for range in [16..20, 20..22, 22..24] {
            bytes.get_mut(range).ok_or_else(|| bad("invalid identity width"))?.reverse();
        }
    }
    Ok(bytes)
}
fn membership(m: &Value) -> Result<bool> {
    m.get("user_placed").and_then(Value::as_bool).ok_or_else(|| bad("missing master bin membership"))
}

/// Canonical full identity for media binding; short-only evidence cannot authorize a file.
pub fn full_identity(id: &Value) -> Result<[u8; 32]> {
    identity_key(&serde_json::json!({"id":id}))?.try_into().map_err(|_| bad("media binding requires a full source identity"))
}

/// Compare recorded picture edit units with an independently verified MXF rate. The only
/// rounded representation currently supported is the observed 29.97 / 30000:1001 pair.
/// This is not permission to substitute rates without opening the identified picture source.
pub fn picture_rate_matches(recorded: &Value, actual: filmcraft_time::FrameRate) -> bool {
    let Some(n) = recorded["rate"]["numerator"].as_i64().filter(|n| *n > 0) else { return false };
    let Some(d) = recorded["rate"]["denominator"].as_i64().filter(|d| *d > 0) else { return false };
    actual.num > 0
        && actual.den > 0
        && (i128::from(n) * i128::from(actual.den) == i128::from(d) * i128::from(actual.num)
            || (filmcraft_time::FrameRate::new(n, d) == filmcraft_time::FrameRate::new(2997, 100) && actual == filmcraft_time::FrameRate::new(30000, 1001)))
}
/// Integer conversion with sub-tick flooring; original count/rate stays in evidence.
fn duration(v: &Value) -> Result<(Tick, bool)> {
    let count = v.get("length").and_then(Value::as_i64).filter(|n| *n >= 0).ok_or_else(|| bad("invalid clip length"))?;
    let rate = v.get("rate").ok_or_else(|| bad("missing clip rate"))?;
    let num = rate.get("numerator").and_then(Value::as_i64).filter(|n| *n > 0).ok_or_else(|| bad("invalid rate numerator"))?;
    let den = rate.get("denominator").and_then(Value::as_i64).filter(|n| *n > 0).ok_or_else(|| bad("invalid rate denominator"))?;
    let scaled =
        i128::from(count).checked_mul(i128::from(den)).and_then(|x| x.checked_mul(i128::from(TICKS_PER_SECOND))).ok_or_else(|| bad("timing overflow"))?;
    let quantized = scaled % i128::from(num) != 0;
    let ticks = i64::try_from(scaled / i128::from(num)).map_err(|_| bad("clip duration exceeds tick range"))?;
    if ticks > Tick::MAX.0 {
        return Err(bad("clip duration exceeds supported tick range"));
    }
    Ok((Tick(ticks), quantized))
}
/// Derive the bin label portably, including catalog paths recorded on Windows.
fn bin_name(path: &str) -> Result<String> {
    let file = path.rsplit(['/', '\\']).next().unwrap_or("");
    let stem = file.get(..file.len().saturating_sub(4)).filter(|_| file.get(file.len().saturating_sub(4)..).is_some_and(|x| x.eq_ignore_ascii_case(".avb")));
    let name = stem.filter(|s| !s.trim().is_empty()).ok_or_else(|| bad("bin path must end in a nonempty .avb filename"))?;
    Ok(name.to_owned())
}
/// Validate the complete transfer before starting a project transaction.
pub fn plan(bytes: &[u8]) -> Result<Plan> {
    plan_scoped(bytes, TransferMode::Metadata)
}

/// Folder/MDB scan inventories are not project-transfer inputs. Let serde validate their JSON
/// syntax without constructing a second in-memory inventory alongside the bin evidence.
#[derive(serde::Deserialize)]
pub(super) struct Document {
    schema_version: u32,
    workflow: String,
    pub bins: Vec<Value>,
}
pub(super) fn read_document(bytes: &[u8]) -> Result<Document> {
    if bytes.len() > MAX_BYTES {
        return Err(bad("report exceeds 256 MiB"));
    }
    let doc: Document = serde_json::from_slice(bytes).map_err(|e| bad(format!("invalid JSON: {e}")))?;
    if doc.schema_version != 2 || doc.workflow != "avid_managed_mxf" {
        return Err(bad("expected schema-2 avid_managed_mxf catalog"));
    }
    if doc.bins.is_empty() || doc.bins.len() > MAX_BINS {
        return Err(bad("expected 1–1024 bins"));
    }
    Ok(doc)
}

/// Defer composition diagnostics only when the caller will run the sequence converter.
pub fn plan_scoped(bytes: &[u8], mode: TransferMode) -> Result<Plan> {
    let doc = read_document(bytes)?;
    let bins = &doc.bins;
    let warnings =
        if mode == TransferMode::Metadata { vec!["Metadata-only import: media components remain unbound and clips are offline.".into()] } else { Vec::new() };
    let mut result = Plan { bins: Vec::new(), warnings };
    let mut count = 0usize;
    let mut mob_count = 0usize;
    let mut quantized_count = 0usize;
    for b in bins {
        if b.get("status").and_then(Value::as_str) != Some("parsed")
            || b.get("snapshot_stable") != Some(&Value::Bool(true))
            || !array(b, "diagnostics")?.is_empty()
        {
            return Err(bad("bin was not decoded completely from a stable snapshot"));
        }
        let name = bin_name(text(b, "path")?)?;
        let summary = b.get("summary").ok_or_else(|| bad("missing bin summary"))?;
        let uid = text(summary, "bin_uid")?.to_owned();
        let catalog = b.get("catalog").ok_or_else(|| bad("missing catalog"))?;
        let clips = array(catalog, "clips")?;
        count = count.checked_add(clips.len()).filter(|n| *n <= MAX_CLIPS).ok_or_else(|| bad("too many clips"))?;
        if catalog.get("summary").and_then(|s| s.get("master_count")).and_then(Value::as_u64) != Some(clips.len() as u64) {
            return Err(bad("incomplete master catalog"));
        }
        let mut out = BinPlan { name, uid, clips: Vec::new(), compositions: 0 };
        // Cross-check the derived catalog against independently exported decoded mobs.
        // Its own master_count cannot prove that no master was omitted during assembly.
        let mobs = array(b, "mobs")?;
        mob_count = mob_count.checked_add(mobs.len()).filter(|n| *n <= MAX_MOBS).ok_or_else(|| bad("too many decoded mobs"))?;
        let mut decoded_masters = BTreeMap::new();
        for m in mobs {
            if m.get("kind").and_then(Value::as_str) == Some("MasterMob") {
                let key = identity_key(m)?;
                membership(m)?;
                if decoded_masters.insert(key, m).is_some() {
                    return Err(bad("duplicate decoded master identity in bin"));
                }
            }
        }
        let rate_evidence = crate::avid_sequence::offline_rate_evidence(mobs)?;
        let mut sources = BTreeMap::new();
        for m in mobs.iter().filter(|m| m["kind"] == "SourceMob" && m["live"] == true) {
            if let Ok(id) = full_identity(&m["id"]) {
                // Duplicate live identities cannot authorize offline metadata.
                sources.entry(id).and_modify(|entry| *entry = None).or_insert(Some(m));
            }
        }
        let mut seen = BTreeSet::new();
        for c in clips {
            if c.get("kind").and_then(Value::as_str) != Some("MasterMob") {
                return Err(bad("catalog entry is not a master clip"));
            }
            let name = text(c, "name")?.to_owned();
            let key = identity_key(c)?;
            if !seen.insert(key.clone()) {
                return Err(bad("duplicate master identity in bin"));
            }
            let placed = membership(c)?;
            let decoded = decoded_masters.remove(&key).ok_or_else(|| bad("catalog master absent from decoded bin"))?;
            if membership(decoded)? != placed {
                return Err(bad("catalog master identity or membership disagrees with decoded bin"));
            }
            for field in ["name", "user", "length", "rate", "attributes"] {
                if c.get(field) != decoded.get(field) {
                    return Err(bad(format!("catalog master {field} disagrees with decoded bin")));
                }
            }
            let decoded_tracks = array(decoded, "tracks")?;
            let catalog_tracks = array(c, "tracks")?;
            if catalog_tracks.len() != decoded_tracks.len()
                || catalog_tracks.iter().zip(decoded_tracks).any(|(a, b)| ["index", "component", "properties"].iter().any(|k| a.get(k) != b.get(k)))
            {
                return Err(bad("catalog master tracks disagree with decoded bin"));
            }
            let user = c.get("user").and_then(Value::as_object).ok_or_else(|| bad("missing user metadata"))?;
            if user.contains_key(SOURCE_KEY) {
                return Err(bad("user metadata conflicts with reserved provenance key"));
            }
            for key in user.keys() {
                if key.len() > MAX_TEXT || key.contains('\0') {
                    return Err(bad("invalid metadata key"));
                }
            }
            let tracks = array(c, "tracks")?;
            if tracks.len() > 64 {
                return Err(bad("too many master tracks"));
            }
            for t in tracks {
                if t.get("index").and_then(Value::as_u64).filter(|n| *n > 0 && *n <= 65535).is_none() {
                    return Err(bad("invalid master track number"));
                }
                if !matches!(t.get("status").and_then(Value::as_str), Some("present" | "absent" | "partial" | "ambiguous" | "unresolved")) {
                    return Err(bad("invalid track availability"));
                }
            }
            let (duration, quantized) = duration(c)?;
            if quantized {
                quantized_count = quantized_count.saturating_add(1);
            }
            // Only placed masters are bin contents; referenced masters remain source evidence.
            if placed {
                let mut source_descriptors = Vec::new();
                let mut descriptor_ids = BTreeSet::new();
                for r in tracks.iter().filter_map(|t| t["references"].as_array()).flatten() {
                    if let Ok(id) = full_identity(&r["source_id"])
                        && let Some(Some(m)) = sources.get(&id)
                        && m["descriptor"].is_object()
                    {
                        if r.get("source_descriptor").is_some_and(|d| d != &m["descriptor"]) {
                            return Err(bad("source descriptor disagrees with decoded live source mob"));
                        }
                        if descriptor_ids.insert(id) {
                            source_descriptors.push(serde_json::json!({"id":m["id"],"descriptor":m["descriptor"]}));
                        }
                    }
                }
                let picture_rate_evidence = full_identity(&c["id"]).ok().and_then(|id| rate_evidence.get(&id)).cloned().flatten();
                out.clips.push(ClipPlan { name, duration, evidence: c.clone(), source_descriptors, picture_rate_evidence });
            } else {
                result.warnings.push(format!("{}: referenced master {name} was not imported as a bin item", out.name));
            }
        }
        if !decoded_masters.is_empty() {
            return Err(bad("catalog omitted decoded masters"));
        }
        for m in mobs {
            if m.get("user_placed") == Some(&Value::Bool(true)) && m.get("kind").and_then(Value::as_str) != Some("MasterMob") {
                if m["kind"] == "CompositionMob" {
                    out.compositions += 1; // bounded by MAX_MOBS
                    if mode == TransferMode::Sequences {
                        continue;
                    }
                }
                result.warnings.push(format!("{}: unsupported bin item {} ({}) was not imported", out.name, text(m, "name")?, text(m, "kind")?));
            }
        }
        result.bins.push(out);
    }
    if quantized_count > 0 {
        result.warnings.push(format!("{quantized_count} durations were floored by less than one FilmCraft tick; original counts and rational rates are preserved without NTSC rate substitution."));
    }
    Ok(result)
}
/// Validate depth and ID space before using the project's recursive bin helpers.
pub(crate) fn validate_project(project: &Project, parent: Option<BinId>) -> Result<()> {
    let mut stack = vec![(&project.root, 0usize)];
    let mut ids = BTreeSet::new();
    let mut found = parent.is_none();
    while let Some((bin, depth)) = stack.pop() {
        if depth >= 128 {
            return Err(bad("destination bin tree is too deep"));
        }
        if !ids.insert(bin.id.0) || bin.id.0 >= project.next_id {
            return Err(bad("inconsistent project bin IDs"));
        }
        if Some(bin.id) == parent {
            found = true;
        }
        for child in &bin.children {
            if let BinEntry::Bin(b) = child {
                stack.push((b, depth.saturating_add(1)));
            }
        }
    }
    if !found {
        return Err(bad("destination parent does not exist"));
    }
    if project.items.keys().any(|id| id.0 >= project.next_id || ids.contains(&id.0)) {
        return Err(bad("inconsistent project item IDs"));
    }
    Ok(())
}
fn allocate_name(names: &mut BTreeSet<String>, base: &str) -> Result<String> {
    if names.insert(base.into()) {
        return Ok(base.into());
    }
    for n in 2..=names.len().saturating_add(2) {
        let name = format!("{base}_{n}");
        if names.insert(name.clone()) {
            return Ok(name);
        }
    }
    Err(bad("cannot allocate bin name"))
}
#[derive(Debug, Serialize)]
pub struct PreviewBin {
    pub name: String,
    pub clips: usize,
}
/// Resolve all names together, accounting for other bins in this import without editing a project.
pub fn preview(project: &Project, plan: &Plan, parent: Option<BinId>) -> Result<Vec<PreviewBin>> {
    validate_project(project, parent)?;
    let required = plan.bins.iter().try_fold(0u64, |n, b| n.checked_add(1)?.checked_add(b.clips.len() as u64)).ok_or_else(|| bad("too many items"))?;
    project.next_id.checked_add(required).ok_or_else(|| bad("project ID space exhausted"))?;
    let bin = match parent {
        Some(id) => project.root.find_bin(id).ok_or_else(|| bad("destination parent does not exist"))?,
        None => &project.root,
    };
    let mut names: BTreeSet<String> = bin.children.iter().filter_map(|x| if let BinEntry::Bin(b) = x { Some(b.name.clone()) } else { None }).collect();
    plan.bins.iter().map(|b| Ok(PreviewBin { name: allocate_name(&mut names, &b.name)?, clips: b.clips.len() })).collect()
}
/// Apply a validated plan to a transaction-local project copy; no I/O or AVB dependency remains.
pub fn apply(project: &mut Project, plan: Plan, parent: Option<BinId>) -> Result<Vec<ImportedBin>> {
    let destinations = preview(project, &plan, parent)?;
    let mut imported = Vec::new();
    for (b, destination) in plan.bins.into_iter().zip(destinations) {
        let name = destination.name;
        let id = project.add_bin(&name, parent);
        let count = b.clips.len();
        for c in b.clips {
            // Deliberately no guessed file path: multi-component sources need a media binding.
            let has_video = c
                .evidence
                .get("tracks")
                .and_then(Value::as_array)
                .is_some_and(|ts| ts.iter().any(|t| t.pointer("/component/media_kind").and_then(Value::as_u64) == Some(1)));
            let info = MediaInfo {
                name: c.name.clone(),
                kind: if has_video { MediaKind::Movie } else { MediaKind::AudioOnly },
                duration: c.duration,
                video: None,
                audio_streams: Vec::new(),
                container: "Avid metadata (unbound)".into(),
                start_timecode: None,
                file_size: None,
            };
            let mid = project.add_item(
                &c.name,
                Label::Lavender,
                ItemKind::Media(MediaClip {
                    media: MediaRef::File { path: String::new() },
                    info,
                    interpret: Interpretation::default(),
                    mark_in: None,
                    mark_out: None,
                    markers: Vec::new(),
                    offline: true,
                    proxy: None,
                    identity: None,
                }),
                Some(id),
            );
            let item = project.item_mut(mid).ok_or_else(|| bad("new item missing"))?;
            if let Some(user) = c.evidence.get("user").and_then(Value::as_object) {
                for (k, v) in user {
                    item.metadata.insert(k.clone(), v.as_str().map(str::to_owned).unwrap_or_else(|| v.to_string()));
                }
            }
            let provenance = serde_json::json!({"version":1,"bin_name":b.name,"bin_uid":b.uid,"clip":c.evidence,"source_descriptors":c.source_descriptors,"picture_rate_evidence":c.picture_rate_evidence,"binding":"unbound"});
            item.metadata.insert(SOURCE_KEY.into(), provenance.to_string());
        }
        imported.push(ImportedBin { bin: id, name, clips: count, compositions: b.compositions });
    }
    Ok(imported)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn picture_rate_reconciliation_is_narrow_and_requires_actual_rate() {
        let actual = filmcraft_time::FrameRate::new(30000, 1001);
        for (n, d, expected) in [(2997, 100, true), (30000, 1001, true), (60000, 2002, true), (2998, 100, false), (30, 1, false), (0, 1, false), (24, 0, false)]
        {
            assert_eq!(picture_rate_matches(&serde_json::json!({"rate":{"numerator":n,"denominator":d}}), actual), expected);
        }
        assert!(!picture_rate_matches(&serde_json::json!({"rate":{"numerator":2997,"denominator":100}}), filmcraft_time::FrameRate::FPS_24));
    }
    use serde_json::json;
    fn fixture() -> Value {
        let mut v = json!({"schema_version":2,"workflow":"avid_managed_mxf","bins":[{
            "path":"C:\\Bins\\音声.avb","status":"parsed","snapshot_stable":true,"diagnostics":[],
            "summary":{"bin_uid":"synthetic"},"mobs":[],"catalog":{"summary":{"master_count":1},"clips":[{
                "name":"Source","kind":"MasterMob","user_placed":true,"id":{"raw":"01".repeat(32),"byte_order":"little"},
                "length":1,"rate":{"numerator":2997,"denominator":125},"user":{},"tracks":[]
            }]}
        }]});
        v["bins"][0]["mobs"] = json!([v["bins"][0]["catalog"]["clips"][0].clone()]);
        v
    }
    fn decode(v: &Value) -> Result<Plan> {
        plan(&serde_json::to_vec(v).unwrap())
    }
    #[test]
    fn decimal_rate_is_preserved_with_explicit_subtick_quantization() {
        let v = fixture();
        let p = decode(&v).unwrap();
        assert_eq!(p.bins[0].clips[0].duration.0, (i128::from(TICKS_PER_SECOND) * 125 / 2997) as i64);
        assert_eq!(p.bins[0].clips[0].evidence["rate"], v["bins"][0]["catalog"]["clips"][0]["rate"]);
        assert!(p.warnings.iter().any(|w| w.contains("less than one")));
        assert_eq!(p.bins[0].name, "音声");
    }
    #[test]
    fn invalid_catalogs_rejected_and_no_mutation_panics() {
        let cases = [
            ("/schema_version", json!(1)),
            ("/bins/0/snapshot_stable", json!(false)),
            ("/bins/0/diagnostics", json!(["incomplete"])),
            ("/bins/0/path", json!("😀")),
            ("/bins/0/catalog/summary/master_count", json!(2)),
            ("/bins/0/catalog/clips/0/length", json!(-1)),
            ("/bins/0/catalog/clips/0/length", json!(i64::MAX)),
            ("/bins/0/catalog/clips/0/rate/numerator", json!(0)),
            ("/bins/0/catalog/clips/0/rate/denominator", json!(0)),
            ("/bins/0/catalog/clips/0/id/raw", json!("urn:smpte:umid:01")),
            ("/bins/0/catalog/clips/0/id/byte_order", json!("middle")),
            ("/bins/0/catalog/clips/0/user", json!({SOURCE_KEY:"reserved"})),
            ("/bins/0/catalog/clips/0/user_placed", json!(null)),
            ("/bins/0/catalog/clips/0/tracks", json!([{"index":-1,"status":"present"}])),
        ];
        for (pointer, value) in cases {
            let mut v = fixture();
            *v.pointer_mut(pointer).unwrap() = value;
            assert!(decode(&v).is_err(), "{pointer}");
        }
        let mut v = fixture();
        v["bins"][0]["catalog"]["clips"][0]["length"] = json!(i64::MAX);
        v["bins"][0]["catalog"]["clips"][0]["rate"]["denominator"] = json!(i64::MAX);
        assert!(decode(&v).is_err());
        let bytes = serde_json::to_vec(&fixture()).unwrap();
        for end in 0..bytes.len() {
            assert!(std::panic::catch_unwind(|| plan(&bytes[..end])).unwrap().is_err());
        }
        for pos in 0..bytes.len() {
            let mut changed = bytes.clone();
            changed[pos] ^= 0xff;
            assert!(std::panic::catch_unwind(|| plan(&changed)).is_ok());
        }
        let deep = format!("{}0{}", "[".repeat(256), "]".repeat(256));
        assert!(std::panic::catch_unwind(|| plan(deep.as_bytes())).unwrap().is_err());
    }
    #[test]
    fn empty_repeated_bins_and_unsupported_items_are_explicit() {
        let mut v = fixture();
        v["bins"][0]["catalog"]["clips"] = json!([]);
        v["bins"][0]["catalog"]["summary"]["master_count"] = json!(0);
        v["bins"][0]["mobs"] = json!([{"kind":"CompositionMob","name":"Sequence","user_placed":true}]);
        let b = v["bins"][0].clone();
        v["bins"].as_array_mut().unwrap().push(b);
        let p = decode(&v).unwrap();
        assert_eq!(p.warnings.len(), 3);
        let mut project = Project::new("Test");
        let out = apply(&mut project, p, None).unwrap();
        assert_eq!(out[0].name, "音声");
        assert_eq!(out[1].name, "音声_2");
        assert!(project.items.is_empty());
    }
    #[test]
    fn duplicate_master_identity_is_not_silently_deduplicated() {
        let mut v = fixture();
        let c = v["bins"][0]["catalog"]["clips"][0].clone();
        v["bins"][0]["catalog"]["clips"].as_array_mut().unwrap().push(c);
        v["bins"][0]["catalog"]["summary"]["master_count"] = json!(2);
        assert!(decode(&v).unwrap_err().to_string().contains("duplicate"));
    }
    #[test]
    fn catalog_must_account_for_decoded_masters_and_membership() {
        let mut v = fixture();
        v["bins"][0]["catalog"]["clips"] = json!([]);
        v["bins"][0]["catalog"]["summary"]["master_count"] = json!(0);
        assert!(decode(&v).is_err(), "a self-consistent count must not conceal an omitted master");
        let mut v = fixture();
        v["bins"][0]["catalog"]["clips"][0]["user_placed"] = json!(false);
        assert!(decode(&v).is_err(), "catalog membership must agree with the decoded bin");
    }
    #[test]
    fn mixed_byte_order_duplicate_master_is_rejected() {
        let mut v = fixture();
        let raw = "060a2b340101010501010f101300000002385d50221306a105dc1c1dd3d90a1b";
        let canonical = "060a2b340101010501010f1013000000505d38021322a10605dc1c1dd3d90a1b";
        v["bins"][0]["catalog"]["clips"][0]["id"]["raw"] = json!(raw);
        let mut second = v["bins"][0]["catalog"]["clips"][0].clone();
        second["id"] = json!({"raw":canonical,"byte_order":"big"});
        v["bins"][0]["catalog"]["clips"].as_array_mut().unwrap().push(second);
        v["bins"][0]["catalog"]["summary"]["master_count"] = json!(2);
        v["bins"][0]["mobs"] = v["bins"][0]["catalog"]["clips"].clone();
        assert!(decode(&v).is_err(), "byte-order aliases must not import twice");
    }
    #[test]
    fn invalid_project_id_counter_is_rejected_before_changes() {
        for next in [0, u64::MAX] {
            let mut project = Project::new("Test");
            project.next_id = next;
            let before = project.clone();
            assert!(apply(&mut project, decode(&fixture()).unwrap(), None).is_err());
            assert_eq!(project, before);
        }
    }
}
