//! Regional CJK fallbacks for UI text, including media and track names.
//! Script metadata / known system families determine glyph forms; coverage alone does not.

use std::sync::{Arc, OnceLock};

use egui::{FontData, FontDefinitions, FontFamily};

use crate::i18n::Language;

const SAMPLE: &str = "中文轨道配乐音效简体汉语繁體漢語车";
const PREFERRED_HANS: &[&str] = &[
    "Microsoft YaHei UI",
    "Microsoft YaHei",
    "PingFang SC",
    "Noto Sans CJK SC",
    "Noto Sans SC",
    "Source Han Sans SC",
    "Source Han Sans CN",
    "WenQuanYi Micro Hei",
];
const PREFERRED_HANT: &[&str] =
    &["Microsoft JhengHei UI", "Microsoft JhengHei", "PingFang TC", "Noto Sans CJK TC", "Noto Sans TC", "Source Han Sans TC", "Source Han Sans TW"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Script {
    Hans,
    Hant,
    Jpan,
    Other,
}

impl Script {
    fn from_scripts(scripts: &[&str]) -> Option<Self> {
        // An entry shared with Jpan is not proof of Simplified regional glyph forms.
        if scripts.contains(&"Jpan") {
            Some(Self::Jpan)
        } else if scripts.contains(&"Hans") {
            Some(Self::Hans)
        } else if scripts.contains(&"Hant") {
            Some(Self::Hant)
        } else {
            None
        }
    }

    fn priority(self, language: Language) -> u8 {
        match (language, self) {
            (Language::ZhCn, Self::Hans) => 0,
            (Language::ZhCn, Self::Jpan) => 1,
            (_, Self::Jpan) => 0,
            (_, Self::Hans) => 1,
            (_, Self::Hant) => 2,
            (_, Self::Other) => 3,
        }
    }
}

struct Fallback {
    name: String,
    script: Script,
    serif: bool,
    bold: bool,
    data: Arc<FontData>,
}

pub(crate) fn craft_fonts() -> impl Iterator<Item = &'static filmcraft_text::fonts::CraftFont> {
    filmcraft_text::fonts::CRAFT_FONTS.iter().filter(|f| f.scripts.iter().any(|s| matches!(*s, "Hans" | "Hant")))
}

/// Keep owned bytes and TTC face indices across language / theme changes. Unknown families
/// remain a last-resort coverage fallback, never evidence of Hans regional glyph forms.
fn system_fonts() -> &'static Vec<Fallback> {
    static FONTS: OnceLock<Vec<Fallback>> = OnceLock::new();
    FONTS.get_or_init(|| {
        if cfg!(target_arch = "wasm32") {
            return Vec::new();
        }
        if [Script::Hans, Script::Hant].iter().all(|script| filmcraft_text::fonts::CRAFT_FONTS.iter().any(|f| Script::from_scripts(f.scripts) == Some(*script)))
        {
            return Vec::new();
        }
        filmcraft_text::fonts::scan_system();
        let faces: Vec<_> = filmcraft_text::fonts::all_faces().into_iter().filter(|f| f.info.origin == "system" && !f.info.italic).collect();
        let mut fonts = Vec::new();
        for (script, preferred, name) in [(Script::Hans, PREFERRED_HANS, "system-hans"), (Script::Hant, PREFERRED_HANT, "system-hant")] {
            if filmcraft_text::fonts::CRAFT_FONTS.iter().any(|f| Script::from_scripts(f.scripts) == Some(script)) {
                continue;
            }
            let face = preferred.iter().find_map(|name| {
                faces.iter().filter(|f| f.info.family.eq_ignore_ascii_case(name) && f.covers_text(SAMPLE)).min_by_key(|f| f.info.weight.abs_diff(400))
            });
            if let Some(data) =
                face.and_then(|f| f.data().map(|bytes| FontData { font: std::borrow::Cow::Owned(bytes), index: f.info.index, tweak: Default::default() }))
            {
                fonts.push(Fallback { name: name.into(), script, serif: false, bold: false, data: Arc::new(data) });
            }
        }
        if fonts.is_empty() && craft_fonts().next().is_none() {
            let face = faces.iter().filter(|f| f.covers_text(SAMPLE)).min_by_key(|f| f.info.weight.abs_diff(400));
            if let Some(data) =
                face.and_then(|f| f.data().map(|bytes| FontData { font: std::borrow::Cow::Owned(bytes), index: f.info.index, tweak: Default::default() }))
            {
                fonts.push(Fallback { name: "system-cjk".into(), script: Script::Other, serif: false, bold: false, data: Arc::new(data) });
            }
        }
        fonts
    })
}

/// The Simplified interface requires a known Hans face, not just a font covering its hanzi.
pub(crate) fn available() -> bool {
    craft_fonts().any(|f| Script::from_scripts(f.scripts) == Some(Script::Hans)) || system_fonts().iter().any(|f| f.script == Script::Hans)
}

pub(crate) fn install(fonts: &mut FontDefinitions, language: Language) {
    let mut fallbacks: Vec<_> = filmcraft_text::fonts::CRAFT_FONTS
        .iter()
        .filter_map(|f| {
            let script = Script::from_scripts(f.scripts)?;
            Some(Fallback {
                name: format!("craft:{} {}", f.family, f.style),
                script,
                serif: if script == Script::Jpan { !f.family.contains("Gothic") } else { f.family.contains("Serif") && !f.family.contains("Sans") },
                bold: f.style == "Bold",
                data: Arc::new(FontData::from_static(f.bytes)),
            })
        })
        .collect();
    if !crate::i18n::craft_japanese_font()
        && let Some(data) = crate::i18n::system_japanese_font()
    {
        fallbacks.push(Fallback { name: "system-japanese".into(), script: Script::Jpan, serif: false, bold: false, data });
    }
    let fallbacks: Vec<_> = fallbacks.iter().chain(system_fonts()).collect();
    append_fallbacks(fonts, language, &fallbacks);
}

fn append_fallbacks(fonts: &mut FontDefinitions, language: Language, fallbacks: &[&Fallback]) {
    for f in fallbacks {
        fonts.font_data.entry(f.name.clone()).or_insert_with(|| f.data.clone());
    }
    for (family, stack) in &mut fonts.families {
        let heavy = matches!(family, FontFamily::Name(n) if matches!(n.as_ref(), "semibold" | "medium"));
        let mut order = fallbacks.to_vec();
        // Stable within a script: retain the existing Gothic/sans and Bold/Regular preferences.
        order.sort_by_key(|f| (f.script.priority(language), f.serif, f.bold != heavy));
        for f in order {
            if !stack.contains(&f.name) {
                stack.push(f.name.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fallback(name: &str, script: Script, serif: bool, bold: bool) -> Fallback {
        Fallback { name: name.into(), script, serif, bold, data: Arc::new(FontData::from_static(filmcraft_text::fonts::INTER_REGULAR)) }
    }

    #[test]
    fn regional_order_preserves_latin_weights_and_deduplicates() {
        // Deliberately shuffled; the Hans face may be a system font even with craft Japanese.
        let fixtures = [
            fallback("hant", Script::Hant, false, false),
            fallback("jp-serif", Script::Jpan, true, false),
            fallback("jp-bold", Script::Jpan, false, true),
            fallback("jp", Script::Jpan, false, false),
            fallback("system-hans", Script::Hans, false, false),
            fallback("hans-bold", Script::Hans, false, true),
            fallback("hans-serif", Script::Hans, true, false),
            fallback("unknown", Script::Other, false, false),
        ];
        let mut fallbacks: Vec<_> = fixtures.iter().collect();
        fallbacks.push(&fixtures[3]);
        for language in Language::ALL {
            let mut fonts =
                FontDefinitions { families: crate::theme::font_families().into_iter().map(|f| (f, vec!["latin".into()])).collect(), ..Default::default() };
            append_fallbacks(&mut fonts, language, &fallbacks);
            let families = fonts.families.clone();
            append_fallbacks(&mut fonts, language, &fallbacks);
            assert_eq!(fonts.families, families);
            assert_eq!(fonts.font_data.keys().filter(|n| fixtures.iter().any(|f| &f.name == *n)).count(), fixtures.len());
            for (family, stack) in &fonts.families {
                let heavy = matches!(family, FontFamily::Name(n) if matches!(n.as_ref(), "medium" | "semibold"));
                let jp = if heavy { ["jp-bold", "jp", "jp-serif"] } else { ["jp", "jp-bold", "jp-serif"] };
                let hans = if heavy { ["hans-bold", "system-hans", "hans-serif"] } else { ["system-hans", "hans-bold", "hans-serif"] };
                let groups = if language == Language::ZhCn { [hans, jp] } else { [jp, hans] };
                let expected: Vec<_> = ["latin"].into_iter().chain(groups.into_iter().flatten()).chain(["hant", "unknown"]).collect();
                assert_eq!(stack.iter().map(String::as_str).collect::<Vec<_>>(), expected, "{language:?} / {family:?}");
            }
        }
    }

    #[test]
    fn no_fallbacks_and_script_metadata_are_handled_explicitly() {
        let mut fonts = FontDefinitions::default();
        let families = fonts.families.clone();
        append_fallbacks(&mut fonts, Language::ZhCn, &[]);
        assert_eq!(fonts.families, families);
        assert_eq!(Script::from_scripts(&["Hans", "Latn"]), Some(Script::Hans));
        assert_eq!(Script::from_scripts(&["Hant", "Latn"]), Some(Script::Hant));
        assert_eq!(Script::from_scripts(&["Jpan", "Hans", "Hant"]), Some(Script::Jpan));
        assert_eq!(Script::from_scripts(&["Latn", "Arab"]), None);
    }
}
