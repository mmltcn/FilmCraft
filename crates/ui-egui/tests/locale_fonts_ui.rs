//! Exercise saved language and runtime font changes through the real app/control channel.
//! FILMCRAFT_UI_SHOTS optionally writes native wgpu renders for visual verification.

use std::sync::mpsc::channel;

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_ui_egui::{FilmcraftApp, control::ControlRequest, i18n::Language, theme};
use serde_json::{Value, json};

fn call(harness: &mut Harness<'_, FilmcraftApp>, tx: &std::sync::mpsc::Sender<ControlRequest>, method: &str, params: Value) {
    let (request, reply) = ControlRequest::new(method, params);
    tx.send(request).unwrap();
    for _ in 0..100 {
        harness.step();
        if let Ok(result) = reply.try_recv() {
            assert_eq!(result["ok"], true, "{method}: {result}");
            for _ in 0..4 {
                harness.step();
            }
            return;
        }
    }
    panic!("control request timed out: {method}");
}

#[test]
fn saved_locale_and_runtime_switches_render_in_the_app() {
    if !filmcraft_ui_egui::i18n::chinese_font_available() {
        eprintln!("SKIPPED: no Hans craft/system face for Chinese UI rendering");
        return;
    }
    let shots = std::env::var_os("FILMCRAFT_UI_SHOTS").map(std::path::PathBuf::from);
    let mut session = Session::default();
    session.execute("file.openDemoProject", json!({})).unwrap();
    session.prefs.general.interface_language = "zh-cn".into();
    let project = serde_json::to_value(&session.project).unwrap();
    let (tx, rx) = channel();
    let app = FilmcraftApp::new(session).with_control(rx);
    let mut builder = Harness::builder().with_size(egui::vec2(1600.0, 980.0)).with_max_steps(10_000);
    if shots.is_some() {
        builder = builder.wgpu().with_pixels_per_point(1.0);
    }
    // A prior app/thread-local language must not override the saved effective language.
    filmcraft_ui_egui::i18n::set_current(Language::Ja);
    let mut harness = builder.build_eframe(move |_cc| app);
    for _ in 0..6 {
        harness.step();
    }
    assert_eq!(harness.state().ui.language, Language::ZhCn);
    harness.ctx.fonts_mut(|fonts| assert_eq!(fonts.definitions().families, theme::build_font_definitions(Language::ZhCn).families));

    for (language, command) in [(Language::ZhCn, "app.language.chinese"), (Language::Ja, "app.language.japanese"), (Language::En, "app.language.english")] {
        if language == Language::Ja && !filmcraft_ui_egui::i18n::japanese_font_available() {
            continue;
        }
        call(&mut harness, &tx, "ui.menu.invoke", json!({"id": command}));
        assert_eq!(harness.state().ui.language, language);
        harness.ctx.fonts_mut(|fonts| assert_eq!(fonts.definitions().families, theme::build_font_definitions(language).families));
        if let Some(dir) = &shots {
            std::fs::create_dir_all(dir).unwrap();
            harness.render().unwrap().save(dir.join(format!("locale-{}.png", language.code()))).unwrap();
        }
    }
    // prefs.set reaches apply_prefs during the app's UI pass, independently of menu dispatch.
    call(&mut harness, &tx, "engine.execute", json!({"command": "prefs.set", "params": {"key": "general.interfaceLanguage", "value": "zh-cn"}}));
    assert_eq!(harness.state().ui.language, Language::ZhCn);
    harness.ctx.fonts_mut(|fonts| assert_eq!(fonts.definitions().families, theme::build_font_definitions(Language::ZhCn).families));
    assert_eq!(serde_json::to_value(&harness.state().session.project).unwrap(), project);
    filmcraft_ui_egui::i18n::set_current(Language::En);
}
