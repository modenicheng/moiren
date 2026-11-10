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

/// Locale-pinned translation for tests: renders `key` under Simplified
/// Chinese no matter which locale parallel UI tests set globally. Keys are
/// the English source strings; the ledger guards in `i18n_guards` keep every
/// literal key resolvable in the shipped catalog.
#[cfg(test)]
pub(crate) fn zh(key: &str) -> String {
    rust_i18n::t!(key, locale = SIMPLIFIED_CHINESE).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untranslated_source_text_falls_back_to_english() {
        // Keys are English source strings; en.yml intentionally stays minimal.
        assert_eq!(
            rust_i18n::t!("untranslated fallback probe", locale = SIMPLIFIED_CHINESE),
            "untranslated fallback probe"
        );
    }

    #[test]
    fn known_keys_translate_per_locale() {
        // Explicit locales keep parallel UI tests independent of global state.
        assert_eq!(
            rust_i18n::t!("Settings", locale = SIMPLIFIED_CHINESE),
            "设置"
        );
        assert_eq!(rust_i18n::t!("Settings", locale = ENGLISH), "Settings");
    }

    #[test]
    fn mixer_routes_translate_interpolation_and_meter_preferences() {
        assert_eq!(
            rust_i18n::t!(
                "%{name} · out %{source_port} → %{target} · in %{port}",
                locale = SIMPLIFIED_CHINESE,
                name = "Mic",
                source_port = 1,
                target = "Bus",
                port = 2
            ),
            "Mic · 输出 1 → Bus · 输入 2"
        );
        assert_eq!(
            rust_i18n::t!("Peak hold", locale = SIMPLIFIED_CHINESE),
            "峰值保持"
        );
        // The input dropdown's live multi-select chrome.
        assert_eq!(
            rust_i18n::t!(
                "Manage inputs (%{used} of %{max})",
                locale = SIMPLIFIED_CHINESE,
                used = 2,
                max = 8
            ),
            "管理输入（已用 2 / 8）"
        );
        assert_eq!(
            rust_i18n::t!(
                "Input limit reached (%{max}) — release one first",
                locale = SIMPLIFIED_CHINESE,
                max = 2
            ),
            "已达上限 2 路 — 先取消一路再勾选"
        );
        // Strip and channel-mode terms (M/S/ST/MONO/SWAP/L/R/n CH/CH n ONLY/DIM)
        // are audio-engineering shorthand: they stay identical in every locale,
        // so the zh-CN catalog mirrors them instead of translating.
        assert_eq!(rust_i18n::t!("ST", locale = SIMPLIFIED_CHINESE), "ST");
        assert_eq!(rust_i18n::t!("MONO", locale = SIMPLIFIED_CHINESE), "MONO");
        assert_eq!(
            rust_i18n::t!("%{channels} CH", locale = SIMPLIFIED_CHINESE, channels = 6),
            "6 CH"
        );
        assert_eq!(rust_i18n::t!("No sends", locale = ENGLISH), "No sends");
    }
}
