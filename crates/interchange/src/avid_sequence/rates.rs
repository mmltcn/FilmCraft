//! Bounded exact-clock evidence for automatic motion adapters.
use super::*;

/// Strip only complete single-source sequences with zero-length padding, retaining frame
/// counts in their original domain until the identified MXF rate is available.
fn picture_leaf<'b>(c: &'b Value, depth: usize, nodes: &mut usize) -> Result<&'b Value> {
    *nodes = nodes.checked_add(1).ok_or_else(|| bad("node count overflow"))?;
    if *nodes > MAX_NODES
        || depth >= 16
        || c["media_kind"] != 1
        || c.get("truncated").is_some_and(|v| v != &Value::Bool(false))
        || c.get("unsupported").is_some()
    {
        return Err(bad("incomplete or excessive motion input"));
    }
    rate(c)?;
    number(c, "length")?;
    if c["class"] == "SCLP" {
        return Ok(c);
    }
    if c["class"] != "SEQU" {
        return Err(bad("unsupported motion input structure"));
    }
    let mut found = None;
    for child in list(c, "components", 64)? {
        if child["media_kind"] != 1
            || rate(child)? != rate(c)?
            || child.get("unsupported").is_some()
            || child.get("truncated").is_some_and(|v| v != &Value::Bool(false))
        {
            return Err(bad("incomplete motion input padding"));
        }
        if child["class"] == "FILL" && number(child, "length")? == 0 {
            continue;
        }
        if found.is_some() || child["length"] != c["length"] {
            return Err(bad("motion input must be one source with zero padding"));
        }
        found = Some(picture_leaf(child, depth + 1, nodes)?);
    }
    found.ok_or_else(|| bad("empty motion input"))
}

/// Exact source clock for the supported constant automatic adapter. Does not infer
/// creative speed effects or certify the renderer's interpolation/cadence.
pub(super) fn automatic_source_rate<'a>(c: &'a Value, depth: usize, nodes: &mut usize) -> Result<(&'a Value, FrameRate)> {
    let p = &c["properties"];
    if c["class"] != "SPED"
        || c["media_kind"] != 1
        || c.get("truncated").is_some_and(|v| v != &Value::Bool(false))
        || c.get("unsupported").is_some_and(|v| v != "component_semantics")
        || p["effect_id"] != "EFF_TIMEWARP.MOTION_CTL"
        || p.pointer("/attributes/_MIXMATCH_MOTIONADAPTER") != Some(&json!(2))
        || p["phase_offset"] != 0
        || p["offset_adjust"].as_f64() != Some(0.0)
        || p["new_source_calculation"] != false
        || p["mc_mode"] != 0
    {
        return Err(bad("unsupported automatic adapter"));
    }
    let output = rate(c)?;
    let length = ticks(c, "length")?;
    if length <= Tick::ZERO {
        return Err(bad("empty adapter"));
    }
    let inputs = list(c, "input_tracks", 1)?;
    let input = inputs.first().filter(|i| i["index"] == 1).ok_or_else(|| bad("unsupported motion input"))?;
    let leaf = picture_leaf(&input["component"], depth + 1, nodes)?;
    let ratio = list(p, "speed_ratio", 2)?;
    let n = ratio.first().and_then(Value::as_i64).filter(|n| *n > 0 && *n <= i64::from(u32::MAX)).ok_or_else(|| bad("invalid adapter ratio"))?;
    let d = ratio.get(1).and_then(Value::as_i64).filter(|n| *n > 0 && *n <= i64::from(u32::MAX)).ok_or_else(|| bad("invalid adapter ratio"))?;
    let num = output.num.checked_mul(d).ok_or_else(|| bad("adapter rate overflow"))?;
    let den = output.den.checked_mul(n).ok_or_else(|| bad("adapter rate overflow"))?;
    let source = FrameRate::new(num, den);
    rate(&json!({"rate":{"numerator":source.num,"denominator":source.den}}))?;
    if !avid_catalog::picture_rate_matches(leaf, source) || number(leaf, "source_track")? == 0 {
        return Err(bad("adapter source clock disagrees with recorded source"));
    }
    let source_length = ticks(&json!({"length":leaf["length"],"rate":{"numerator":source.num,"denominator":source.den}}), "length")?;
    if source_length < length || source_length.0 - length.0 >= source.frame_duration().0 {
        return Err(bad("adapter input does not cover output duration"));
    }
    Ok((leaf, source))
}

/// Retain corroborating clocks only from complete live compositions. Conflicting
/// clocks for a master disable this fallback. Full sequence validation still runs
/// when importing sequences; this evidence authorizes only the source clock.
pub(crate) fn offline_rate_evidence(mobs: &[Value]) -> Result<BTreeMap<[u8; 32], Option<Value>>> {
    fn visit(c: &Value, depth: usize, nodes: &mut usize, out: &mut BTreeMap<[u8; 32], Option<Value>>) -> Result<()> {
        *nodes = nodes.checked_add(1).ok_or_else(|| bad("node count overflow"))?;
        if depth >= 16 || *nodes > MAX_NODES {
            return Err(bad("rate evidence graph exceeds bounds"));
        }
        if c.get("truncated").is_some_and(|v| v != &Value::Bool(false)) {
            return Ok(());
        }
        if c["class"] == "SPED" {
            if let Ok((leaf, r)) = automatic_source_rate(c, depth, nodes) {
                let id = avid_catalog::full_identity(&leaf["source_id"])?;
                let proof = json!({"basis":"automatic_adapter","rate":{"numerator":r.num,"denominator":r.den},"source_track":leaf["source_track"],"output_rate":c["rate"],"speed_ratio":c["properties"]["speed_ratio"]});
                out.entry(id)
                    .and_modify(|old| {
                        if old.as_ref().is_some_and(|v| v["rate"] != proof["rate"] || v["source_track"] != proof["source_track"]) {
                            *old = None;
                        }
                    })
                    .or_insert(Some(proof));
            }
            return Ok(());
        }
        if c["class"] == "SEQU" {
            for child in list(c, "components", MAX_NODES)? {
                visit(child, depth + 1, nodes, out)?;
            }
        } else if c["class"] == "TKFX" && c["properties"]["effect_id"] == "INT_EFF_SPATIAL_ADAPTER" {
            for input in list(c, "input_tracks", 1)? {
                visit(&input["component"], depth + 1, nodes, out)?;
            }
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    let mut nodes = 0;
    for m in mobs.iter().filter(|m| {
        m["kind"] == "CompositionMob" && m["live"] == true && m["user_placed"] == true && !m.get("truncated").is_some_and(|v| v != &Value::Bool(false))
    }) {
        for t in list(m, "tracks", 64)? {
            if !t.get("truncated").is_some_and(|v| v != &Value::Bool(false)) {
                visit(&t["component"], 0, &mut nodes, &mut out)?;
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn adapter() -> Value {
        json!({"class":"SPED","media_kind":1,"rate":{"numerator":24,"denominator":1},"length":46,"unsupported":"component_semantics",
            "properties":{"effect_id":"EFF_TIMEWARP.MOTION_CTL","attributes":{"_MIXMATCH_MOTIONADAPTER":2},"speed_ratio":[1001,1250],"phase_offset":0,"offset_adjust":0.0,"new_source_calculation":false,"mc_mode":0},
            "input_tracks":[{"index":1,"component":{"class":"SCLP","media_kind":1,"source_id":{"raw":"12".repeat(32),"byte_order":"big"},"source_track":1,"start":25,"length":58,"rate":{"numerator":2997,"denominator":100}}}]})
    }
    fn composition(c: Value) -> Value {
        json!({"kind":"CompositionMob","live":true,"user_placed":true,"tracks":[{"component":c}]})
    }
    #[test]
    fn exact_adapter_clock_and_nonzero_source_offset() {
        let c = adapter();
        let (leaf, r) = automatic_source_rate(&c, 0, &mut 0).unwrap();
        assert_eq!(r, FrameRate::new(30000, 1001));
        assert_eq!(leaf["start"], 25);
        // An hour of integer source frames stays on the exact tick grid.
        let frames = 108000;
        let t = ticks(&json!({"length":frames,"rate":{"numerator":r.num,"denominator":r.den}}), "length").unwrap();
        assert_eq!(t.0, TICKS_PER_SECOND * 3600 * 1001 / 1000);
        assert!(ticks(&json!({"length":frames,"rate":{"numerator":2997,"denominator":100}}), "length").is_err());
    }
    #[test]
    fn malformed_and_creative_adapters_never_authorize_a_clock() {
        let original = adapter();
        for (pointer, value) in [
            ("/properties/speed_ratio", json!([0, 1250])),
            ("/properties/speed_ratio", json!([1001, 0])),
            ("/properties/speed_ratio", json!([1001])),
            ("/properties/speed_ratio", json!([i64::MAX, 1250])),
            ("/properties/speed_ratio", json!([1000, 1250])),
            ("/properties/phase_offset", json!(1)),
            ("/properties/attributes/_MIXMATCH_MOTIONADAPTER", json!(0)),
            ("/input_tracks/0/component/length", json!(57)),
            ("/input_tracks/0/component/source_track", json!(0)),
        ] {
            let mut c = original.clone();
            *c.pointer_mut(pointer).unwrap() = value;
            assert!(std::panic::catch_unwind(|| automatic_source_rate(&c, 0, &mut 0).map(|(_, r)| r)).unwrap().is_err(), "{pointer}");
            assert!(offline_rate_evidence(&[composition(c)]).unwrap().is_empty());
        }
        let mut truncated = original.clone();
        truncated["truncated"] = json!(true);
        assert!(offline_rate_evidence(&[composition(truncated)]).unwrap().is_empty());
        assert!(automatic_source_rate(&original, 16, &mut 0).is_err());
        assert!(automatic_source_rate(&original, 0, &mut MAX_NODES.clone()).is_err());
    }
    #[test]
    fn conflicting_clocks_disable_offline_fallback_and_inactive_history_is_ignored() {
        let first = composition(adapter());
        let mut other = adapter();
        other["properties"]["speed_ratio"] = json!([4, 5]);
        other["input_tracks"][0]["component"]["rate"] = json!({"numerator":30,"denominator":1});
        let other = composition(other);
        let evidence = offline_rate_evidence(&[first.clone(), other.clone(), first.clone()]).unwrap();
        assert_eq!(evidence.len(), 1);
        assert!(evidence.values().next().unwrap().is_none());
        let mut inactive = other;
        inactive["live"] = json!(false);
        assert!(offline_rate_evidence(&[first, inactive]).unwrap().values().next().unwrap().is_some());
    }
}
