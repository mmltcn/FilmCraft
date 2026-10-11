//! Adjustment layers (M5.6): effects apply to the composite of every track below, opacity and
//! blend mode mix the adjusted picture over the original, Motion and masks restrict the region,
//! and nesting scopes them to their own sequence.

use super::*;
use filmcraft_media::generators::GeneratorSource;
use filmcraft_media::{DemoScene, Generator, MediaSource};
use filmcraft_project::{ClipId, Label, Mask, MaskPath, MediaClip, MediaRef, SequenceSettings, TrackKind, find_effect};
use filmcraft_time::{FrameRate, TICKS_PER_SECOND, TimeRange};

const R: FrameRate = FrameRate::FPS_24;

struct Scene {
    p: Project,
    seq: ItemId,
    map: SourceMap,
}

fn media(p: &mut Project, map: &mut SourceMap, g: GeneratorSource) -> ItemId {
    let info = g.info().clone();
    let generator = g.generator.clone();
    let id = p.add_item(
        &info.name.clone(),
        Label::Iris,
        ItemKind::Media(MediaClip {
            media: MediaRef::Generator(generator),
            info,
            interpret: Default::default(),
            mark_in: None,
            mark_out: None,
            markers: vec![],
            offline: false,
            proxy: None,
            identity: None,
        }),
        None,
    );
    map.0.insert(id, Arc::new(g) as SharedSource);
    id
}

fn matte(p: &mut Project, map: &mut SourceMap, c: [f32; 4]) -> ItemId {
    media(p, map, GeneratorSource::new(Generator::ColorMatte { color: c }, 320, 180, R, Tick(10 * TICKS_PER_SECOND)))
}

fn place(p: &mut Project, seq: ItemId, track: usize, item: ItemId) -> ClipId {
    let ti = p.make_track_item(item, TrackKind::Video, Tick::ZERO, TimeRange::new(Tick::ZERO, R.tick_of(48)), R).unwrap();
    let id = ti.id;
    let mut ti = ti;
    ti.scale_to_frame = true;
    let s = p.sequence_mut(seq).unwrap();
    s.video_tracks[track].items.push(ti);
    s.video_tracks[track].sort();
    id
}

fn adjustment(p: &mut Project, seq: ItemId, track: usize) -> ClipId {
    let a = p.add_item("Adjustment Layer", Label::Lavender, ItemKind::AdjustmentLayer { width: 320, height: 180, rate: R, duration: R.tick_of(48) }, None);
    place(p, seq, track, a)
}

fn scene(tracks: usize) -> Scene {
    let mut p = Project::new("adj");
    let seq = p.new_sequence("s", SequenceSettings { width: 320, height: 180, frame_rate: R, ..Default::default() }, tracks, 0, None);
    Scene { p, seq, map: SourceMap::default() }
}

fn clip(s: &mut Scene, c: ClipId) -> &mut TrackItem {
    s.p.sequence_mut(s.seq).unwrap().find_item_mut(c).unwrap().1
}

fn fx(s: &mut Scene, c: ClipId, id: &str) {
    let e = find_effect(id).unwrap().instance();
    let it = clip(s, c);
    let pos = it.effects.iter().position(|e| e.def().is_some_and(|d| d.intrinsic)).unwrap_or(it.effects.len());
    it.effects.insert(pos, e);
}

fn render(s: &Scene) -> Image {
    render_sequence(&s.p, s.seq, R.tick_of(12), RenderOptions::default(), &s.map)
}

fn near(a: [f32; 4], b: [f32; 3], tol: f32) -> bool {
    (0..3).all(|k| (a[k] - b[k]).abs() < tol)
}

#[test]
fn applies_to_everything_below_but_not_above() {
    let mut s = scene(3);
    let red = matte(&mut s.p, &mut s.map, [1.0, 0.0, 0.0, 1.0]);
    let blue = matte(&mut s.p, &mut s.map, [0.0, 0.0, 1.0, 1.0]);
    place(&mut s.p, s.seq, 0, red);
    let adj = adjustment(&mut s.p, s.seq, 1);
    fx(&mut s, adj, "invert");
    let img = render(&s);
    assert!(near(img.get(160, 90), [0.0, 1.0, 1.0], 1e-4), "red inverted to cyan: {:?}", img.get(160, 90));
    // a small blue layer above the adjustment layer is untouched
    let top = place(&mut s.p, s.seq, 2, blue);
    clip(&mut s, top).effect_mut("motion").unwrap().params.get_mut("scale").unwrap().value = ParamValue::Float(25.0);
    let img = render(&s);
    assert!(near(img.get(160, 90), [0.0, 0.0, 1.0], 1e-4), "{:?}", img.get(160, 90));
    assert!(near(img.get(10, 10), [0.0, 1.0, 1.0], 1e-4));
}

#[test]
fn opacity_and_blend_mode_mix_over_the_original() {
    let mut s = scene(2);
    let red = matte(&mut s.p, &mut s.map, [1.0, 0.0, 0.0, 1.0]);
    place(&mut s.p, s.seq, 0, red);
    let adj = adjustment(&mut s.p, s.seq, 1);
    fx(&mut s, adj, "invert");
    clip(&mut s, adj).effect_mut("opacity").unwrap().params.get_mut("opacity").unwrap().value = ParamValue::Float(25.0);
    let c = render(&s).get(100, 100);
    assert!(near(c, [0.75, 0.25, 0.25], 1e-3), "{c:?}");
    // Multiply: red × cyan = black at 100 %
    let op = clip(&mut s, adj).effect_mut("opacity").unwrap();
    op.params.get_mut("opacity").unwrap().value = ParamValue::Float(100.0);
    op.params.get_mut("blend").unwrap().value =
        ParamValue::Choice(filmcraft_project::effect::BLEND_MODES.iter().position(|b| *b == "Multiply").unwrap() as u32);
    let c = render(&s).get(100, 100);
    assert!(near(c, [0.0, 0.0, 0.0], 1e-3), "{c:?}");
}

#[test]
fn motion_and_masks_restrict_the_region() {
    let mut s = scene(2);
    let red = matte(&mut s.p, &mut s.map, [1.0, 0.0, 0.0, 1.0]);
    place(&mut s.p, s.seq, 0, red);
    let adj = adjustment(&mut s.p, s.seq, 1);
    fx(&mut s, adj, "invert");
    // Motion: scaled to 50 % around the centre → only the middle is inverted
    clip(&mut s, adj).effect_mut("motion").unwrap().params.get_mut("scale").unwrap().value = ParamValue::Float(50.0);
    let img = render(&s);
    assert!(near(img.get(160, 90), [0.0, 1.0, 1.0], 1e-4), "{:?}", img.get(160, 90));
    assert!(near(img.get(10, 10), [1.0, 0.0, 0.0], 1e-4), "{:?}", img.get(10, 10));
    clip(&mut s, adj).effect_mut("motion").unwrap().params.get_mut("scale").unwrap().value = ParamValue::Float(100.0);
    // a mask on the adjustment layer's effect (sequence pixels): left half
    let inv = clip(&mut s, adj).effects.iter_mut().find(|e| e.effect == "invert").unwrap();
    let mut m = Mask::new("m", MaskPath::rect(0.0, 0.0, 160.0, 180.0));
    m.feather.value = ParamValue::Float(0.0);
    inv.masks.push(m);
    let img = render(&s);
    assert!(near(img.get(40, 90), [0.0, 1.0, 1.0], 1e-4));
    assert!(near(img.get(280, 90), [1.0, 0.0, 0.0], 1e-4));
    // the same at half resolution (mask in sequence pixels scales with the output)
    let half = render_sequence(&s.p, s.seq, R.tick_of(12), RenderOptions { scale: 0.5, ..Default::default() }, &s.map);
    assert!(near(half.get(20, 45), [0.0, 1.0, 1.0], 1e-4));
    assert!(near(half.get(140, 45), [1.0, 0.0, 0.0], 1e-4));
    // an opacity mask on the adjustment layer: right half only (with the effect mask → nothing)
    let op = clip(&mut s, adj).effect_mut("opacity").unwrap();
    let mut m = Mask::new("o", MaskPath::rect(160.0, 0.0, 320.0, 180.0));
    m.feather.value = ParamValue::Float(0.0);
    op.masks.push(m);
    let img = render(&s);
    assert!(near(img.get(40, 90), [1.0, 0.0, 0.0], 1e-4), "{:?}", img.get(40, 90));
    assert!(near(img.get(280, 90), [1.0, 0.0, 0.0], 1e-4));
}

#[test]
fn nested_adjustment_layers_stay_in_their_sequence() {
    let mut s = scene(2);
    let red = matte(&mut s.p, &mut s.map, [1.0, 0.0, 0.0, 1.0]);
    let ocean = media(&mut s.p, &mut s.map, GeneratorSource::demo(DemoScene::OceanSunset));
    // inner sequence: red + adjustment (invert) → cyan
    let inner = s.p.new_sequence("inner", SequenceSettings { width: 320, height: 180, frame_rate: R, ..Default::default() }, 2, 0, None);
    place(&mut s.p, inner, 0, red);
    let a = s.p.add_item("Adjustment Layer", Label::Lavender, ItemKind::AdjustmentLayer { width: 320, height: 180, rate: R, duration: R.tick_of(48) }, None);
    let ac = place(&mut s.p, inner, 1, a);
    {
        let it = s.p.sequence_mut(inner).unwrap().find_item_mut(ac).unwrap().1;
        it.effects.insert(0, find_effect("invert").unwrap().instance());
    }
    // outer: ocean on V1, the nested sequence (scaled to 50 %) on V2
    place(&mut s.p, s.seq, 0, ocean);
    let nest = place(&mut s.p, s.seq, 1, inner);
    clip(&mut s, nest).effect_mut("motion").unwrap().params.get_mut("scale").unwrap().value = ParamValue::Float(50.0);
    let img = render(&s);
    assert!(near(img.get(160, 90), [0.0, 1.0, 1.0], 1e-4), "inner adjusted: {:?}", img.get(160, 90));
    // the outer V1 (outside the nest) is untouched
    let plain = {
        let mut p2 = s.p.clone();
        p2.sequence_mut(s.seq).unwrap().video_tracks[1].items.clear();
        render_sequence(&p2, s.seq, R.tick_of(12), RenderOptions::default(), &s.map)
    };
    assert_eq!(img.get(5, 5), plain.get(5, 5));
}
#[test]
fn full_frame_faded_print_plans_adjustment_and_preserves_alpha() {
    for alpha in [0.0, 0.4, 1.0] {
        for opacity in [0.0, 25.0, 100.0] {
            let mut s = scene(3);
            let color = matte(&mut s.p, &mut s.map, [0.2, 0.5, 0.8, alpha]);
            place(&mut s.p, s.seq, 0, color);
            let adj = adjustment(&mut s.p, s.seq, 1);
            let e = crate::lumetri_presets::find("Faded Print").unwrap().instance();
            clip(&mut s, adj).effects.insert(0, e);
            clip(&mut s, adj).effect_mut("opacity").unwrap().param_mut("opacity").unwrap().value = ParamValue::Float(opacity);
            let p = crate::plan::plan_frame(&s.p, s.seq, R.tick_of(12), RenderOptions::default(), &s.map);
            assert!(
                matches!(&p, crate::plan::FramePlan::Composite { steps, .. } if steps.iter().any(|s| matches!(s, crate::plan::PlanStep::Adjustment { .. })))
            );
            let cpu = render(&s);
            let planned = crate::plan::execute_cpu(&p);
            let max = cpu.px.iter().zip(&planned.px).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
            assert!(max < 1e-5, "alpha {alpha} opacity {opacity}: {max}");
        }
    }
}

#[test]
fn unsupported_adjustments_keep_cpu_fallback() {
    let mut s = scene(2);
    let color = matte(&mut s.p, &mut s.map, [0.2, 0.5, 0.8, 1.0]);
    place(&mut s.p, s.seq, 0, color);
    let adj = adjustment(&mut s.p, s.seq, 1);
    fx(&mut s, adj, "lumetri");
    clip(&mut s, adj).effect_mut("lumetri").unwrap().param_mut("look").unwrap().value = ParamValue::Choice(4);
    let p = crate::plan::plan_frame(&s.p, s.seq, R.tick_of(12), RenderOptions::default(), &s.map);
    assert!(matches!(p, crate::plan::FramePlan::Image(_)));
    clip(&mut s, adj).effect_mut("lumetri").unwrap().param_mut("look").unwrap().value = ParamValue::Choice(0);
    clip(&mut s, adj).effect_mut("motion").unwrap().param_mut("scale").unwrap().value = ParamValue::Float(50.0);
    let p = crate::plan::plan_frame(&s.p, s.seq, R.tick_of(12), RenderOptions::default(), &s.map);
    assert!(matches!(p, crate::plan::FramePlan::Image(_)));
}

#[test]
fn masked_adjustment_and_non_normal_blend_keep_cpu_fallback() {
    let mut s = scene(2);
    let color = matte(&mut s.p, &mut s.map, [0.2, 0.5, 0.8, 1.0]);
    place(&mut s.p, s.seq, 0, color);
    let adj = adjustment(&mut s.p, s.seq, 1);
    fx(&mut s, adj, "invert");
    clip(&mut s, adj).effect_mut("invert").unwrap().masks.push(Mask::new("region", MaskPath::rect(0.0, 0.0, 100.0, 100.0)));
    let p = crate::plan::plan_frame(&s.p, s.seq, R.tick_of(12), RenderOptions::default(), &s.map);
    assert!(matches!(p, crate::plan::FramePlan::Image(_)));
    clip(&mut s, adj).effect_mut("invert").unwrap().masks.clear();
    clip(&mut s, adj).effect_mut("opacity").unwrap().param_mut("blend").unwrap().value =
        ParamValue::Choice(filmcraft_project::effect::BLEND_MODES.iter().position(|b| *b == "Multiply").unwrap() as u32);
    let p = crate::plan::plan_frame(&s.p, s.seq, R.tick_of(12), RenderOptions::default(), &s.map);
    assert!(matches!(p, crate::plan::FramePlan::Image(_)));
}

fn assert_adjustment_plan_parity(s: &Scene, opts: RenderOptions) -> crate::plan::FramePlan {
    let plan = crate::plan::plan_frame(&s.p, s.seq, R.tick_of(12), opts, &s.map);
    assert_eq!(
        crate::plan::is_cpu_frame(&s.p, s.seq, R.tick_of(12), opts),
        matches!(plan, crate::plan::FramePlan::Image(_)),
        "export pool admission must agree with the planner, including disabled effects and output scale"
    );
    let cpu = render_sequence(&s.p, s.seq, R.tick_of(12), opts, &s.map);
    let planned = crate::plan::execute_cpu(&plan);
    let max = cpu.px.iter().zip(&planned.px).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
    assert!(max < 1e-5, "planned adjustment diff {max}");
    plan
}

#[test]
fn adjustment_planning_handles_disabled_effects_and_multiple_alpha_overlays() {
    let mut s = scene(3);
    let color = matte(&mut s.p, &mut s.map, [0.2, 0.5, 0.8, 0.4]);
    place(&mut s.p, s.seq, 0, color);
    let first = adjustment(&mut s.p, s.seq, 1);
    let second = adjustment(&mut s.p, s.seq, 2);
    // Even a no-effect adjustment composites its translucent backdrop over itself.
    assert!(matches!(assert_adjustment_plan_parity(&s, RenderOptions::default()), crate::plan::FramePlan::Composite { .. }));
    fx(&mut s, first, "invert");
    fx(&mut s, second, "brightness_contrast");
    clip(&mut s, second).effect_mut("opacity").unwrap().param_mut("opacity").unwrap().value = ParamValue::Float(35.0);
    assert_adjustment_plan_parity(&s, RenderOptions::default());
    assert!(matches!(assert_adjustment_plan_parity(&s, RenderOptions { effects: false, ..Default::default() }), crate::plan::FramePlan::Layers { .. }));
    clip(&mut s, first).enabled = false;
    clip(&mut s, second).enabled = false;
    assert!(matches!(assert_adjustment_plan_parity(&s, RenderOptions::default()), crate::plan::FramePlan::Layers { .. }));
}

#[test]
fn nested_full_frame_adjustment_does_not_touch_parent_backdrop() {
    let mut s = scene(2);
    let background = matte(&mut s.p, &mut s.map, [0.8, 0.2, 0.1, 1.0]);
    let foreground = matte(&mut s.p, &mut s.map, [0.2, 0.5, 0.8, 0.4]);
    place(&mut s.p, s.seq, 0, background);
    let inner = s.p.new_sequence("inner", SequenceSettings { width: 320, height: 180, frame_rate: R, ..Default::default() }, 2, 0, None);
    place(&mut s.p, inner, 0, foreground);
    let adj = adjustment(&mut s.p, inner, 1);
    s.p.sequence_mut(inner).unwrap().find_item_mut(adj).unwrap().1.effects.insert(0, find_effect("invert").unwrap().instance());
    place(&mut s.p, s.seq, 1, inner);
    assert_adjustment_plan_parity(&s, RenderOptions::default());
}

#[test]
fn differently_sized_adjustment_stays_cpu() {
    let mut s = scene(2);
    let color = matte(&mut s.p, &mut s.map, [0.2, 0.5, 0.8, 1.0]);
    place(&mut s.p, s.seq, 0, color);
    let source =
        s.p.add_item("small adjustment", Label::Lavender, ItemKind::AdjustmentLayer { width: 160, height: 180, rate: R, duration: R.tick_of(48) }, None);
    let adj = place(&mut s.p, s.seq, 1, source);
    fx(&mut s, adj, "invert");
    assert!(matches!(assert_adjustment_plan_parity(&s, RenderOptions::default()), crate::plan::FramePlan::Image(_)));
}

#[test]
fn nonfinite_adjustment_parameters_keep_cpu_fallback() {
    let mut s = scene(2);
    let color = matte(&mut s.p, &mut s.map, [0.2, 0.5, 0.8, 1.0]);
    place(&mut s.p, s.seq, 0, color);
    let adj = adjustment(&mut s.p, s.seq, 1);
    fx(&mut s, adj, "brightness_contrast");
    clip(&mut s, adj).effect_mut("brightness_contrast").unwrap().param_mut("brightness").unwrap().value = ParamValue::Float(f64::NAN);
    let p = crate::plan::plan_frame(&s.p, s.seq, R.tick_of(12), RenderOptions::default(), &s.map);
    assert!(matches!(p, crate::plan::FramePlan::Image(_)));
}

#[test]
fn adjustment_with_translucent_dissolve_keeps_whole_frame_cpu_fallback() {
    let mut s = scene(2);
    let red = matte(&mut s.p, &mut s.map, [1.0, 0.0, 0.0, 1.0]);
    let blue = matte(&mut s.p, &mut s.map, [0.0, 0.0, 1.0, 1.0]);
    let a = place(&mut s.p, s.seq, 0, red);
    let b = place(&mut s.p, s.seq, 0, blue);
    clip(&mut s, b).start = R.tick_of(24);
    clip(&mut s, a).duration = R.tick_of(24);
    clip(&mut s, b).duration = R.tick_of(24);
    s.p.sequence_mut(s.seq).unwrap().video_tracks[0].sort();
    for id in [a, b] {
        clip(&mut s, id).effect_mut("opacity").unwrap().param_mut("opacity").unwrap().value = ParamValue::Float(50.0);
    }
    let adj = adjustment(&mut s.p, s.seq, 1);
    s.p.sequence_mut(s.seq).unwrap().video_tracks[0].transitions.push(filmcraft_project::Transition {
        id: filmcraft_project::TransitionId(999),
        effect: find_effect("cross_dissolve").unwrap().instance(),
        start: R.tick_of(18),
        duration: R.tick_of(12),
        from: Some(a),
        to: Some(b),
        align: Default::default(),
        reverse: false,
    });
    let opts = RenderOptions::default();
    let time = R.tick_of(24);
    let cpu = render_sequence(&s.p, s.seq, time, opts, &s.map);
    let plan = crate::plan::plan_frame(&s.p, s.seq, time, opts, &s.map);
    assert!(matches!(plan, crate::plan::FramePlan::Image(_)));
    assert_eq!(crate::plan::execute_cpu(&plan).px, cpu.px);
    // The dissolve produces alpha .5; the no-op adjustment composites it over itself.
    assert!((cpu.get(100, 100)[3] - 0.75).abs() < 1e-5);
    clip(&mut s, adj).enabled = false;
    assert!(matches!(crate::plan::plan_frame(&s.p, s.seq, time, opts, &s.map), crate::plan::FramePlan::Layers { .. }));
}
