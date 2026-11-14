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
    fn native_rename_notice_translates_without_promising_a_windows_name_change() {
        assert_eq!(
            rust_i18n::t!(
                "Renaming recreates the Windows endpoint and can interrupt audio or require other apps to select it again. The requested name is not proof of the Windows display name; check the observed name above.",
                locale = SIMPLIFIED_CHINESE
            ),
            "重命名会重新创建 Windows 端点，可能中断音频，或需要其他应用重新选择端点。请求的名称不代表 Windows 显示名称已更新；请核对上方读回的名称。"
        );
    }

    #[test]
    fn application_capture_scope_translates_without_promising_tab_isolation() {
        assert_eq!(
            rust_i18n::t!(
                "Captures the selected process and its descendants, not an individual browser tab or audio session.",
                locale = SIMPLIFIED_CHINESE
            ),
            "捕获所选进程及其后代进程，而非单个浏览器标签页或音频会话。"
        );
        assert_eq!(
            rust_i18n::t!(
                "Captures the selected process and its descendants, not an individual browser tab or audio session.",
                locale = ENGLISH
            ),
            "Captures the selected process and its descendants, not an individual browser tab or audio session."
        );
    }

    #[test]
    fn vad_recovery_cap_translates_without_promising_latency() {
        assert_eq!(
            rust_i18n::t!("VAD recovery cap", locale = SIMPLIFIED_CHINESE),
            "VAD 追赶周期上限"
        );
        assert_eq!(
            rust_i18n::t!("Compatible (8 periods)", locale = SIMPLIFIED_CHINESE),
            "兼容（8 周期）"
        );
        assert_eq!(
            rust_i18n::t!(
                "%{periods} periods",
                locale = SIMPLIFIED_CHINESE,
                periods = 1
            ),
            "1 周期"
        );
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

    #[test]
    fn measured_input_actions_and_limits_translate_without_a_qualification_claim() {
        assert_eq!(
            rust_i18n::t!("Measured Process input", locale = SIMPLIFIED_CHINESE),
            "实测进程输入"
        );
        assert_eq!(
            rust_i18n::t!(
                "Apply measured budget and restart",
                locale = SIMPLIFIED_CHINESE
            ),
            "应用实测预算并重启音频"
        );
        assert_eq!(
            rust_i18n::t!(
                "Measured budget is active; physical latency remains unqualified.",
                locale = SIMPLIFIED_CHINESE
            ),
            "实测预算已生效；物理端到端延迟仍未验收。"
        );
    }

    #[test]
    fn process_window_actions_translate_without_promising_admission_or_latency() {
        assert_eq!(
            rust_i18n::t!(
                "Evaluate closed Process prefix",
                locale = SIMPLIFIED_CHINESE
            ),
            "评估已闭合的进程历史前缀"
        );
        assert_eq!(
            rust_i18n::t!("No violation in this window", locale = SIMPLIFIED_CHINESE),
            "本窗口内未发现违规"
        );
        assert_eq!(
            rust_i18n::t!(
                "Not qualified: no sustained, waveform, sample-age or physical latency proof",
                locale = SIMPLIFIED_CHINESE
            ),
            "未通过验收：不能证明持续运行、波形正确性、样本年龄或物理延迟"
        );
        assert_eq!(
            rust_i18n::t!(
                "Apply evaluated budget and start audio",
                locale = SIMPLIFIED_CHINESE
            ),
            "应用已评估预算并启动音频"
        );
        assert_eq!(
            rust_i18n::t!(
                "Empirical whole-drain / gap / whole-wake",
                locale = SIMPLIFIED_CHINESE
            ),
            "实测整批传输 / 间隔 / 整次唤醒"
        );
        assert_eq!(
            rust_i18n::t!(
                "A stopped matching evaluated native candidate is required.",
                locale = SIMPLIFIED_CHINESE
            ),
            "需要已停止且身份匹配的原生评估候选配置。"
        );
    }
}
