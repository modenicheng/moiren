//! User-interface localization. Keys are the English source strings; the
//! compiled locale catalogs translate them at render time. Locale state is
//! process-global (shared with gpui-component's translation backend) and only
//! ever read on the UI thread, so switching costs one atomic store plus an
//! explicit repaint request.

use serde::{Deserialize, Serialize};

pub const ENGLISH: &str = "en";
pub const SIMPLIFIED_CHINESE: &str = "zh-CN";

/// Languages shipped with the workstation. Selectable in Settings > Appearance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum UiLanguage {
    #[default]
    #[serde(rename = "en")]
    English,
    #[serde(rename = "zh-CN")]
    SimplifiedChinese,
}

impl UiLanguage {
    pub const ALL: [UiLanguage; 2] = [UiLanguage::English, UiLanguage::SimplifiedChinese];

    /// Locale code persisted in preferences and passed to `rust_i18n`.
    pub fn code(self) -> &'static str {
        match self {
            UiLanguage::English => ENGLISH,
            UiLanguage::SimplifiedChinese => SIMPLIFIED_CHINESE,
        }
    }

    /// Display name in its own language, as locale switchers conventionally show.
    pub fn title(self) -> &'static str {
        match self {
            UiLanguage::English => "English",
            UiLanguage::SimplifiedChinese => "简体中文",
        }
    }
}

/// Switch every `t!` lookup (app and gpui-component) to this language.
pub fn apply(language: UiLanguage) {
    rust_i18n::set_locale(language.code());
}

pub fn current() -> UiLanguage {
    match rust_i18n::locale().as_ref() {
        SIMPLIFIED_CHINESE => UiLanguage::SimplifiedChinese,
        _ => UiLanguage::English,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untranslated_source_text_falls_back_to_english() {
        // Keys are English source strings; en.yml intentionally stays minimal.
        apply(UiLanguage::SimplifiedChinese);
        assert_eq!(
            rust_i18n::t!("untranslated fallback probe"),
            "untranslated fallback probe"
        );
        apply(UiLanguage::English);
    }

    #[test]
    fn known_keys_translate_per_locale() {
        apply(UiLanguage::SimplifiedChinese);
        assert_eq!(rust_i18n::t!("Settings"), "设置");
        apply(UiLanguage::English);
        assert_eq!(rust_i18n::t!("Settings"), "Settings");
    }
}
