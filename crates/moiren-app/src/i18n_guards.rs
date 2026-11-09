//! i18n ledger guards (design 21 §5, P2). Test-only: nothing here ships.
//!
//! Three guards keep the localization ledger honest, in the spirit of the
//! `action_assets` source-scan guards:
//!
//! 1. Debug-format tripwire — `format!("{..:?}")` results may not reach UI
//!    text (`t!` args, `label`/`detail`/`heading`, text-named field
//!    assignments). The migration series (#121, #154-#158 and the closeout
//!    sweep) retired every known site and emptied its allowlist, so any hit
//!    now fails outright; fix it with a display-name mapping (design 21 §4)
//!    rather than re-allowlisting.
//! 2. Two-way key ledger — every string literal passed to `t!` must exist as
//!    a key in `locales/zh-CN.yml`, and every catalog key must be referenced
//!    by a literal call site or listed in `DYNAMIC_KEY_EXCEPTIONS` (keys
//!    translated at paint from dynamically produced strings, e.g. the fault
//!    cause catalog via `t!(fault.cause.as_str())`).
//! 3. Placeholder agreement — for each literal key with `%{name}`
//!    placeholders, the call-site arguments and the catalog value must expose
//!    exactly the same placeholder set.
//!
//! The scans approximate Rust with a comment/string-aware lexer over
//! whitespace-normalized sources, so rustfmt reflowing never changes the
//! verdict; they err toward missing exotic shapes, not toward false alarms.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const SRC_DIR: &str = "src";
const UI_DIR: &str = "src/ui";
const CATALOG_YML: &str = "locales/zh-CN.yml";

// ---------------------------------------------------------------------------
// source plumbing
// ---------------------------------------------------------------------------

fn manifest_path(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

/// Production `.rs` files under `root`, sorted for stable failure messages.
/// Files that exist only as `#[cfg(test)]` module sources (`tests.rs`,
/// `*_tests.rs`) are skipped wholesale; inline `#[cfg(test)]` items are
/// stripped per file by [`strip_cfg_test_items`].
fn collect_rs_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries: Vec<_> = std::fs::read_dir(&dir)
            .unwrap_or_else(|error| panic!("read directory {}: {error}", dir.display()))
            .collect();
        for entry in entries {
            let path = entry.expect("directory entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") && !is_test_module_file(&path)
            {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

fn is_test_module_file(path: &Path) -> bool {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    name == "tests.rs" || name.ends_with("_tests.rs")
}

/// Removes `#[cfg(test)]`-attributed items so tests never enter the ledger.
/// Handles brace-delimited items (`mod tests { .. }`, test fns) and
/// declaration-only items (`mod tests;`). Attribute-shaped text inside
/// comments (test_harness.rs docs) or braces inside strings never derail
/// the removal: one forward lex marks every non-lexical index first.
fn strip_cfg_test_items(src: &str) -> String {
    const ATTR: [char; 12] = ['#', '[', 'c', 'f', 'g', '(', 't', 'e', 's', 't', ')', ']'];
    let chars: Vec<char> = src.chars().collect();
    let lexical = lexical_indices(&chars);
    let mut out = String::with_capacity(src.len());
    let mut pos = 0;
    while let Some(at) =
        (pos..chars.len()).find(|&i| lexical[i] && chars[i] == '#' && chars[i..].starts_with(&ATTR))
    {
        out.extend(&chars[pos..at]);
        // Lex forward to the item's first `{` or `;`.
        let mut i = skip_trivia(&chars, at + ATTR.len());
        while i < chars.len() {
            if !lexical[i] {
                i += 1;
            } else if chars[i] == '{' || chars[i] == ';' {
                break;
            } else {
                i += 1;
            }
        }
        if i < chars.len() && chars[i] == '{' {
            i = matching_delimiter(&chars, i, '{', '}') + 1;
        } else {
            i += 1; // consume the `;`
        }
        pos = i;
    }
    out.extend(&chars[pos.min(chars.len())..]);
    out
}

/// Whether a `//` or `/*` comment starts at `i`; the index just past it.
/// The guard on `chars[i]` matters: a `\n` before a `//` line must not be
/// mistaken for a comment start (its newline search would return `i`).
fn skip_comment(chars: &[char], i: usize) -> Option<usize> {
    if chars.get(i) != Some(&'/') {
        return None;
    }
    match chars.get(i + 1) {
        Some('/') => Some(
            (i..chars.len())
                .find(|&j| chars[j] == '\n')
                .unwrap_or(chars.len()),
        ),
        Some('*') => Some(
            (i + 2..chars.len())
                .find(|&j| chars[j] == '*' && chars.get(j + 1) == Some(&'/'))
                .map_or(chars.len(), |j| j + 2),
        ),
        _ => None,
    }
}

/// One forward lex: `true` at every index that is ordinary code, `false`
/// inside comments, string literals and char literals.
fn lexical_indices(chars: &[char]) -> Vec<bool> {
    let mut lexical = vec![true; chars.len()];
    let mut i = 0;
    while i < chars.len() {
        if let Some(end) = skip_comment(chars, i) {
            for mark in &mut lexical[i..end.min(chars.len())] {
                *mark = false;
            }
            i = end;
        } else if chars[i] == '"' {
            let end = skip_string(chars, i);
            for mark in &mut lexical[i..end.min(chars.len())] {
                *mark = false;
            }
            i = end;
        } else if chars[i] == '\'' && is_char_literal(chars, i) {
            let end = skip_char(chars, i);
            for mark in &mut lexical[i..end.min(chars.len())] {
                *mark = false;
            }
            i = end;
        } else {
            i += 1;
        }
    }
    lexical
}

/// Skips whitespace and `//`/`/* */` comments from `at`.
fn skip_trivia(chars: &[char], at: usize) -> usize {
    let mut i = at;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
        } else if chars[i] == '/' && chars.get(i + 1) == Some(&'/') {
            i = (i..chars.len())
                .find(|&j| chars[j] == '\n')
                .unwrap_or(chars.len());
        } else if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
            i = (i + 2..chars.len())
                .find(|&j| chars[j] == '*' && chars.get(j + 1) == Some(&'/'))
                .map_or(chars.len(), |j| j + 2);
        } else {
            break;
        }
    }
    i
}

/// Index of the delimiter matching the one at `open` (string-literal aware).
fn matching_delimiter(chars: &[char], open: usize, open_ch: char, close_ch: char) -> usize {
    let mut depth = 0usize;
    let mut i = open;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' {
            i = skip_string(chars, i);
            continue;
        }
        if c == '\'' && is_char_literal(chars, i) {
            i = skip_char(chars, i);
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            i = (i..chars.len())
                .find(|&j| chars[j] == '\n')
                .unwrap_or(chars.len());
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            i = (i + 2..chars.len())
                .find(|&j| chars[j] == '*' && chars.get(j + 1) == Some(&'/'))
                .map_or(chars.len(), |j| j + 2);
            continue;
        }
        if c == open_ch {
            depth += 1;
        } else if c == close_ch {
            depth -= 1;
            if depth == 0 {
                return i;
            }
        }
        i += 1;
    }
    chars.len().saturating_sub(1)
}

/// Index just past the double-quoted string opening at `open`.
fn skip_string(chars: &[char], open: usize) -> usize {
    let mut i = open + 1;
    while i < chars.len() {
        if chars[i] == '\\' {
            i += 2;
        } else if chars[i] == '"' {
            return i + 1;
        } else {
            i += 1;
        }
    }
    chars.len()
}

/// Index just past the char literal opening at `open`.
fn skip_char(chars: &[char], open: usize) -> usize {
    let mut i = open + 1;
    while i < chars.len() {
        if chars[i] == '\\' {
            i += 2;
        } else if chars[i] == '\'' {
            return i + 1;
        } else {
            i += 1;
        }
    }
    chars.len()
}

fn is_char_literal(chars: &[char], open: usize) -> bool {
    chars.get(open + 1) == Some(&'\\')
        || (chars.get(open + 2) == Some(&'\'') && chars.get(open + 1).is_some())
}

/// Collapses whitespace runs within a line to one space but keeps line
/// breaks, so rustfmt reflow never changes what the scanners see while
/// `//` comments still end where they end.
fn normalize(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut in_run = false;
    for c in src.chars() {
        if c == '\n' {
            out.push('\n');
            in_run = false;
        } else if c.is_whitespace() {
            if !in_run {
                out.push(' ');
                in_run = true;
            }
        } else {
            out.push(c);
            in_run = false;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// comment/string-aware macro scanner
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Macro {
    Format,
    T,
}

struct MacroCall {
    kind: Macro,
    /// Index of the macro name's first char (for prefix-shape checks).
    tok_start: usize,
    open: usize,
    close: usize,
    /// Enclosing groups, innermost first: (head ident, open index). The
    /// macro's own paren is not among them.
    heads: Vec<(String, usize)>,
}

/// Finds every `format!(..)` / `t!(..)` call, skipping string literals,
/// char literals and comments so parens inside them never confuse the
/// paren stack.
fn scan_macro_calls(text: &str) -> Vec<MacroCall> {
    let chars: Vec<char> = text.chars().collect();
    let mut calls = Vec::new();
    let mut heads: Vec<(String, usize)> = Vec::new();
    // Macros awaiting their close paren: (kind, tok_start, depth at token).
    let mut pending: Vec<(Macro, usize, usize)> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            i = (i..chars.len())
                .find(|&j| chars[j] == '\n')
                .unwrap_or(chars.len());
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            i = (i + 2..chars.len())
                .find(|&j| chars[j] == '*' && chars.get(j + 1) == Some(&'/'))
                .map_or(chars.len(), |j| j + 2);
        } else if c == '"' {
            i = skip_string(&chars, i);
        } else if c == '\'' && is_char_literal(&chars, i) {
            i = skip_char(&chars, i);
        } else if c == '(' {
            heads.push((head_before(&chars, i), i));
            i += 1;
        } else if c == ')' {
            let own_open = heads.pop().map(|(_, open)| open);
            if let Some(&(kind, tok_start, depth)) = pending.last()
                && depth == heads.len()
            {
                pending.pop();
                calls.push(MacroCall {
                    kind,
                    tok_start,
                    open: own_open.unwrap_or(tok_start),
                    close: i,
                    heads: heads.clone(),
                });
            }
            i += 1;
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let ident: String = chars[start..i].iter().collect();
            let kind = if ident == "format" && chars.get(i) == Some(&'!') {
                Some(Macro::Format)
            } else if ident == "t" && chars.get(i) == Some(&'!') {
                Some(Macro::T)
            } else {
                None
            };
            if let Some(kind) = kind
                && chars.get(i + 1) == Some(&'(')
            {
                pending.push((kind, start, heads.len()));
            }
        } else {
            i += 1;
        }
    }
    calls
}

/// The identifier immediately before the `(` at `open`, including a `!`
/// (e.g. `t!`); empty when the paren is not a call (tuple, closure).
fn head_before(chars: &[char], open: usize) -> String {
    let mut i = open;
    while i > 0 && chars[i - 1].is_whitespace() {
        i -= 1;
    }
    let mut end = i;
    if i > 0 && chars[i - 1] == '!' {
        i -= 1;
        end = i + 1; // keep the `!` in the head so `t!` stays distinct from `t`
    }
    let mut start = i;
    while start > 0 && (chars[start - 1].is_alphanumeric() || chars[start - 1] == '_') {
        start -= 1;
    }
    if start == end {
        String::new()
    } else {
        chars[start..end].iter().collect()
    }
}

// ---------------------------------------------------------------------------
// guard 1: Debug-format flows into UI text
// ---------------------------------------------------------------------------

/// Heads whose argument is by-convention visible UI text.
const UI_TEXT_HEADS: [&str; 4] = ["t!", "label", "heading", "detail"];

/// Struct-literal field names that carry display text; `name: format!(..)`
/// shapes with these fields are treated as UI-text flows.
const TEXT_FIELD_INITS: [&str; 8] = [
    "reason", "message", "error", "hint", "title", "text", "label", "note",
];

/// Whether a `{:?}` (or pretty `{:#?}`) spec appears in the first string
/// literal of a `format!` call.
fn has_debug_spec(text: &str, call: &MacroCall) -> bool {
    let Some(literal) = first_arg_literal(text, call) else {
        return false;
    };
    literal.contains(":?}") || literal.contains("#?}")
}

/// The first plain string literal of a macro call, if its first argument is
/// one. Escaped quotes are unescaped.
fn first_arg_literal(text: &str, call: &MacroCall) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut i = call.open + 1;
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    if chars.get(i) != Some(&'"') {
        return None;
    }
    i += 1;
    let mut literal = String::new();
    while i < chars.len() {
        if chars[i] == '\\'
            && let Some(next) = chars.get(i + 1)
        {
            if *next == '"' {
                literal.push('"');
            } else {
                literal.push('\\');
                literal.push(*next);
            }
            i += 2;
            continue;
        }
        if chars[i] == '"' {
            return Some(literal);
        }
        literal.push(chars[i]);
        i += 1;
    }
    None
}

/// The guard's judgement call: does this `format!` call's result flow into
/// visible UI text? True when any enclosing call head is `t!`/`label`/
/// `heading`/`detail`, when the call directly initializes a text-named field
/// or is `Some(..)`-wrapped into an assignment, or when it is a match-arm
/// value inside a group that also translates a label (the settings-rows
/// shape).
fn is_ui_text_flow(text: &[char], call: &MacroCall) -> bool {
    if call
        .heads
        .iter()
        .any(|(head, _)| UI_TEXT_HEADS.contains(&head.as_str()))
    {
        return true;
    }
    let prefix: String = text[..call.tok_start].iter().collect();
    let trimmed = prefix.trim_end();
    if trimmed.ends_with("= Some(") {
        return true;
    }
    if let Some(tail) = trimmed.strip_suffix(':') {
        // The field name is the ident immediately before the colon; a
        // quoted `"reason"` JSON-style key (platform_trace) has none.
        let field: String = tail
            .trim_end()
            .chars()
            .rev()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        let field: String = field.chars().rev().collect();
        if TEXT_FIELD_INITS.contains(&field.as_str()) {
            return true;
        }
    }
    if trimmed.ends_with("=>") {
        // Match-arm value: only suspicious when a nearby enclosing group
        // also builds a translated label (the (label, value) row tuple).
        for (_, open) in call.heads.iter().take(3) {
            let inner: String = text[open + 1..call.tok_start].iter().collect();
            if inner.contains("t!(") {
                return true;
            }
        }
    }
    false
}

fn rel_path(src_root: &Path, path: &Path) -> String {
    path.strip_prefix(src_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[test]
fn debug_format_does_not_reach_ui_text() {
    let src_root = manifest_path(SRC_DIR);
    let ui_root = manifest_path(UI_DIR);
    let files = collect_rs_files(&ui_root);
    assert!(files.len() > 30, "ui source tree went missing");

    let mut leaks = Vec::new();
    let mut debug_calls = 0;
    for path in &files {
        let source = std::fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let stripped = strip_cfg_test_items(&source);
        let text = normalize(&stripped);
        let chars: Vec<char> = text.chars().collect();
        for call in scan_macro_calls(&text) {
            if call.kind != Macro::Format || !has_debug_spec(&text, &call) {
                continue;
            }
            debug_calls += 1;
            if is_ui_text_flow(&chars, &call) {
                leaks.push(format!(
                    "  {} -> {}",
                    rel_path(&src_root, path),
                    chars[call.open..=call.close].iter().collect::<String>()
                ));
            }
        }
    }

    // Scanner sanity: a regression that lexes nothing must fail loudly, not
    // pass vacuously. Roughly fifty Debug-format calls live under ui/,
    // most of them element-id builders; lower this floor deliberately if
    // that ever legitimately shrinks.
    assert!(
        debug_calls >= 20,
        "the Debug-format scan went blind: only {debug_calls} format! calls \
         with a Debug spec found under ui/"
    );
    assert!(
        leaks.is_empty(),
        "Debug-format text reached the UI; user-visible strings must come from \
         t! keys (design 21 §4). The closeout migration emptied the allowlist, \
         so every hit is a new leak: fix the site with a display-name registry \
         entry (`title()` accessor + `t!` at paint, zh-CN catalog key), or — \
         only for a truly permanent, arbitrary infra exception — reintroduce a \
         small allowlist here with a why note. Site(s):\n{}",
        leaks.join("\n")
    );
}

// ---------------------------------------------------------------------------
// guards 2 and 3: the t!-literal key ledger and placeholder agreement
// ---------------------------------------------------------------------------

struct TCite {
    rel: String,
    args: BTreeSet<String>,
}

/// Every literal-key `t!` call in the crate, conservatively approximated by
/// "first argument is a plain string literal". Dynamically keyed calls
/// (`t!(cause.as_str())`) are invisible here by construction and are covered
/// by `DYNAMIC_KEY_EXCEPTIONS` in the reverse direction.
fn collect_literal_t_calls(src_root: &Path) -> BTreeMap<String, Vec<TCite>> {
    let mut calls: BTreeMap<String, Vec<TCite>> = BTreeMap::new();
    for path in collect_rs_files(src_root) {
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let text = normalize(&strip_cfg_test_items(&source));
        for call in scan_macro_calls(&text) {
            if call.kind != Macro::T {
                continue;
            }
            let Some(key) = first_arg_literal(&text, &call) else {
                continue;
            };
            calls.entry(key).or_default().push(TCite {
                rel: rel_path(src_root, &path),
                args: named_args(&text, &call),
            });
        }
    }
    calls
}

/// Named interpolation arguments after the key literal (`name = value`),
/// excluding rust_i18n's special `locale` argument.
fn named_args(text: &str, call: &MacroCall) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let chars: Vec<char> = text.chars().collect();
    let literal_end = call.open + 1; // scan from just inside the call
    let mut i = literal_end;
    while i + 1 < call.close {
        if chars[i] == '"' {
            i = skip_string(&chars, i);
            continue;
        }
        if (chars[i].is_ascii_lowercase() || chars[i] == '_')
            && (i == 0 || !chars[i - 1].is_alphanumeric() && chars[i - 1] != '_')
        {
            let start = i;
            while i < call.close && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let ident: String = chars[start..i].iter().collect();
            let mut j = i;
            while j < call.close && chars[j].is_whitespace() {
                j += 1;
            }
            if j < call.close && chars[j] == '=' && chars.get(j + 1) != Some(&'=') {
                if ident != "locale" {
                    names.insert(ident);
                }
                continue;
            }
            continue;
        }
        i += 1;
    }
    names
}

/// `%{name}` placeholder names in a key or catalog value.
fn placeholders(text: &str) -> BTreeSet<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut names = BTreeSet::new();
    let mut i = 0;
    while i + 1 < chars.len() {
        if chars[i] == '%'
            && chars[i + 1] == '{'
            && let Some(close) = (i + 2..chars.len()).find(|&j| chars[j] == '}')
        {
            let name: String = chars[i + 2..close].iter().collect();
            if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                names.insert(name);
            }
            i = close + 1;
            continue;
        }
        i += 1;
    }
    names
}

/// The zh-CN catalog: top-level key -> zh-CN value. Tolerant parse of the
/// rust_i18n version-2 shape (`"Key":` then indented `en:` / `zh-CN:` lines);
/// block scalars and anchors are not used in this catalog.
fn parse_zh_cn_catalog() -> BTreeMap<String, String> {
    let path = manifest_path(CATALOG_YML);
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let mut catalog = BTreeMap::new();
    let mut current: Option<String> = None;
    for line in source.lines() {
        let trimmed_end = line.trim_end();
        if !line.starts_with(' ') && trimmed_end.starts_with('"') && trimmed_end.ends_with("\":") {
            current = Some(trimmed_end[1..trimmed_end.len() - 2].to_string());
            catalog
                .entry(current.clone().expect("just set"))
                .or_default();
        } else if let Some(value) = trimmed_end.trim_start().strip_prefix("zh-CN: ")
            && let Some(key) = &current
        {
            catalog.insert(key.clone(), value.trim().to_string());
        }
    }
    catalog
}

#[test]
fn every_t_literal_key_exists_in_the_zh_cn_catalog() {
    let src_root = manifest_path(SRC_DIR);
    let catalog = parse_zh_cn_catalog();
    assert!(catalog.len() > 400, "zh-CN catalog went missing");
    let calls = collect_literal_t_calls(&src_root);
    // Scanner sanity: the crate holds several hundred literal call sites;
    // a regression that lexes nothing must fail loudly, not pass vacuously.
    assert!(
        calls.len() >= 300,
        "the t! literal scan went blind: only {} distinct keys found",
        calls.len()
    );
    let missing: Vec<String> = calls
        .keys()
        .filter(|key| !catalog.contains_key(*key))
        .cloned()
        .collect();
    assert!(
        missing.is_empty(),
        "t! literal(s) with no zh-CN catalog entry — add the key to \
         locales/zh-CN.yml: {missing:?}"
    );
}

/// Catalog keys translated at paint from dynamically produced strings, so
/// they legitimately have no literal `t!` call site. This list is generated
/// from the actual ledger diff (not hand-waved); it falls into these
/// families:
/// - fault cause/hint strings raised by the service, graph preparation and
///   driver layers and translated at paint via `t!(fault.cause.as_str())` /
///   `t!(hint)` (shell/fault_popover.rs, graph_canvas_view/nodes.rs);
/// - display-name registries translated at paint from `&'static str`
///   accessors: page titles, node catalog titles/descriptions/groups,
///   window-control labels, latency-profile titles/descriptions, backend /
///   precision / sample-format / source-mode / meter-status labels,
///   compressor parameter names;
/// - `DEFAULT_ELIGIBILITY_HELP` help prose (devices/exports.rs).
///
/// When one of these keys gains a literal call site, remove it here — the
/// guard fails on stale entries on purpose.
const DYNAMIC_KEY_EXCEPTIONS: [&str; 116] = [
    // AudioConfigError Display strings looked up at paint from the enum payload
    // (UiFailure::InvalidAudioSettings); mirrored in tests/i18n_ledger.rs
    // ALLOWED_RUNTIME_KEYS.
    "sample rate must be 44.1, 48, 88.2 or 96 kHz",
    "buffer must be 64, 128, 256, 512, 1024, 2048 or 4096 frames",
    "device identity must contain 1 to 32768 bytes without NUL",
    "at most 256 device overrides can be saved per direction",
    "16-bit PCM",
    "24-bit PCM",
    "32-bit PCM",
    "32-bit float",
    "64-bit float",
    "A Microphone export can only bind an output node",
    "A Playback export can only bind an input node",
    "ASIO",
    "Adjust linear amplitude",
    "Adjust the channel count, or reconnect the route",
    "Application",
    "Application audio",
    "Attack",
    "Balanced",
    "Bus dynamic input ports",
    "Channel parameters are outside their valid range",
    "Check the channel's parameter settings, or delete and re-add it",
    "Choose an input resource",
    "Choose an input source at the top of the channel, or delete the channel",
    "Choose an output device",
    "Choose an output device at the top of the channel, or delete the channel",
    "Choose another device, or check the sample rate/precision settings",
    "Close window",
    "Compatible",
    "Compressor parameters are outside their valid range",
    "Compressor parameters only apply to Compressor nodes",
    "DESTINATIONS",
    "Default eligibility is permission on this computer only, not saved in the project. Off prevents both Windows automatic and user manual default selection. On only permits eligibility; it does not set a Windows default.",
    "Delete this channel and re-add one of the same type",
    "Device native",
    "Device output",
    "Devices",
    "Dry / wet",
    "Each device or application source can feed one channel; unbind the other one first",
    "Faders cover −∞ to +24 dB; cycle the channel-mode button back to a valid setting",
    "Feedback",
    "Graph",
    "Hold",
    "INPUTS",
    "Input gain",
    "Knee",
    "Linked peak dynamics",
    "Low",
    "Makeup gain",
    "Managed FFT observation",
    "Managed recording tap",
    "Matrix",
    "Maximize window",
    "Minimize window",
    "Mix bus",
    "Mixer gain or channel mode is invalid",
    "Mono and Swap only apply to Balance nodes",
    "Multiple Sink nodes are bound to the same device",
    "Multiple Source nodes are bound to the same application source",
    "Multiple Source nodes are bound to the same physical device",
    "No native audio backend is configured",
    "Node names cannot be empty and are limited to 256 bytes",
    "Node state count does not match the graph, or the graph exceeds 4096 nodes",
    "OBSERVATION",
    "One parametric peaking band",
    "Only Balance nodes have an adjustable position",
    "Only Gain nodes have an adjustable gain",
    "Only Sink nodes can bind a valid output device",
    "Only Source nodes can bind an input source",
    "Ordered processor chain",
    "Output gain",
    "PROCESSING",
    "Parameters are outside their valid range",
    "Peak and RMS observation",
    "Physical input",
    "Prepared sine-wave source",
    "Process loopback source",
    "Project node states do not match the topology identity",
    "ROUTING",
    "Range",
    "Ratio",
    "Release",
    "Request ~10 ms device buffers (512 frames at 48 kHz).",
    "Request ~5 ms device buffers (256 frames at 48 kHz).",
    "SENDS",
    "SOURCES",
    "Static channel routing matrix",
    "Stereo balance, mono and swap",
    "Test tone",
    "The Balance node's position must be a finite value in -1..1",
    "The Gain node's gain must be a finite value in 0..16",
    "The Sink node is not bound to an output device",
    "The Source node is not bound to an input source",
    "The application source is missing a stable process identity",
    "The apply request identity space is exhausted",
    "The channel's channel count disagrees with its wiring",
    "The channel's configuration does not match its kind",
    "The configuration points at a channel that no longer exists",
    "The graph is limited to 4096 nodes with at most 256 channels each",
    "The graph needs a Sink node",
    "The graph needs a Source node",
    "The graph revision space is exhausted",
    "The input source configuration is invalid",
    "The input/output device configuration is invalid",
    "The live parameter queue is busy; the candidate value is kept",
    "The node does not exist",
    "The virtual export and graph node have different channel counts",
    "The virtual export does not exist",
    "These two switches only apply to Balance channels",
    "This channel is missing a usable processor configuration",
    "Threshold",
    "Tone generator",
    "Use each device's default period.",
    "WASAPI",
    "WASAPI capture endpoint",
    "WASAPI render endpoint",
    "Wet mix",
];

#[test]
fn every_zh_cn_key_is_referenced_by_a_literal_or_exempted() {
    let src_root = manifest_path(SRC_DIR);
    let catalog = parse_zh_cn_catalog();
    let referenced: BTreeSet<String> = collect_literal_t_calls(&src_root).keys().cloned().collect();
    let exceptions: BTreeSet<&str> = DYNAMIC_KEY_EXCEPTIONS.into_iter().collect();

    let orphaned: Vec<String> = catalog
        .keys()
        .filter(|key| !referenced.contains(*key) && !exceptions.contains(key.as_str()))
        .cloned()
        .collect();
    assert!(
        orphaned.is_empty(),
        "zh-CN catalog key(s) referenced by no literal t! call and not in \
         DYNAMIC_KEY_EXCEPTIONS — either the code literal is missing (add one), the \
         key is dynamically translated at paint (document it in \
         DYNAMIC_KEY_EXCEPTIONS), or the key is dead (remove it from the yml with \
         its consumer): {orphaned:?}"
    );

    let stale: Vec<&str> = exceptions
        .iter()
        .copied()
        .filter(|key| referenced.contains(*key))
        .collect();
    assert!(
        stale.is_empty(),
        "DYNAMIC_KEY_EXCEPTIONS entr(y|ies) now have literal t! call sites — the \
         exception list must track reality, remove them: {stale:?}"
    );
}

#[test]
fn placeholders_agree_between_call_sites_and_the_catalog() {
    let src_root = manifest_path(SRC_DIR);
    let catalog = parse_zh_cn_catalog();
    let calls = collect_literal_t_calls(&src_root);
    let mut problems = Vec::new();
    for (key, cites) in &calls {
        let literal = placeholders(key);
        if let Some(value) = catalog.get(key) {
            let in_catalog = placeholders(value);
            if in_catalog != literal {
                problems.push(format!(
                    "catalog value for {key:?} exposes {in_catalog:?}, literal says {literal:?}"
                ));
            }
        } // missing keys are the forward ledger guard's job.
        for cite in cites {
            if cite.args != literal {
                problems.push(format!(
                    "{} calls {key:?} with args {:?}, literal placeholders are {:?}",
                    cite.rel, cite.args, literal
                ));
            }
        }
    }
    assert!(
        problems.is_empty(),
        "placeholder disagreement between t! call sites, literals and the zh-CN \
         catalog (names must match exactly):\n{}",
        problems.join("\n")
    );
}

// ---------------------------------------------------------------------------
// guard self-checks
// ---------------------------------------------------------------------------

/// The migration-PR helper (`i18n::zh`) pinned here so it stays exercised:
/// keys are English source strings, zh-CN comes from the catalog.
#[test]
fn zh_helper_pins_the_catalog_locale() {
    assert_eq!(crate::i18n::zh("Settings"), "设置");
    assert_eq!(
        crate::i18n::zh("Meter subscription: %{error}"),
        "电平表订阅：%{error}"
    );
}

#[test]
fn the_scanners_see_through_formatting_and_tests() {
    // Inline cfg(test) modules vanish; declaration-only test modules too.
    let source = "fn prod() {}\n/// keep {\n#[cfg(test)]\nmod tests {\n    fn inner() { let s = \"}\"; }\n}\n#[cfg(test)]\nmod harness;\nfn prod2() {}\n";
    let stripped = strip_cfg_test_items(source);
    assert!(stripped.contains("fn prod()"));
    assert!(stripped.contains("fn prod2()"));
    assert!(!stripped.contains("fn inner"));
    assert!(!stripped.contains("mod harness"));

    assert_eq!(normalize("a\n  b\tc  \n d"), "a\n b c \n d");

    // Escapes, comments and strings must not confuse paren matching.
    let text = normalize("fn f() { // (\n let s = \")\"; let c = '('; format!(\"{:?}\", s) }");
    let calls = scan_macro_calls(&text);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].kind, Macro::Format);
    assert!(has_debug_spec(&text, &calls[0]));
}

#[test]
fn the_ui_text_flow_classifier_matches_the_documented_shapes() {
    // Snippets may carry a nested t! alongside the format! under scrutiny;
    // the guard judges the format! call.
    let format_call = |text: &str| -> MacroCall {
        let formats: Vec<MacroCall> = scan_macro_calls(text)
            .into_iter()
            .filter(|call| call.kind == Macro::Format)
            .collect();
        assert_eq!(formats.len(), 1, "self-check snippet has one format!");
        formats.into_iter().next().expect("checked above")
    };

    let flagged = [
        "self.dispatch_error = Some(format!(\"{error:?}\"))",
        "label(format!(\"{v:?}\"))",
        "heading(format!(\"{v:?}\"))",
        "detail(&t!(\"K\"), format!(\"{v:?}\"))",
        "t!(\"Meter: %{error}\", error = format!(\"{e:?}\"))",
        "Invalid { reason: format!(\"{error:?}\") }",
        "rows.push((t!(\"%{prefix}\", prefix = p), match m { A => format!(\"{v:?}\") }))",
    ];
    for snippet in flagged {
        let text = normalize(snippet);
        let chars: Vec<char> = text.chars().collect();
        let call = format_call(&text);
        assert!(
            has_debug_spec(&text, &call) && is_ui_text_flow(&chars, &call),
            "expected this to be flagged as UI text: {snippet}"
        );
    }

    let clean = [
        // Element ids and lookup keys are plumbing, not paint.
        ".id(SharedString::from(format!(\"strip-{id:?}\")))",
        "let key = format!(\"{id:?}\");",
        ".map(|id| format!(\"{id:?}\"))",
        // Display-only formatting is the approved shape.
        "label(format!(\"{}%\", zoom))",
        "section(format!(\"inputs-{:?}\", entry.id), \"INPUTS\", ..)",
    ];
    for snippet in clean {
        let text = normalize(snippet);
        let chars: Vec<char> = text.chars().collect();
        let call = format_call(&text);
        assert!(
            !(has_debug_spec(&text, &call) && is_ui_text_flow(&chars, &call)),
            "expected this to pass the guard: {snippet}"
        );
    }
}
