//! Ledger guard for the locale catalog (issue #120, design 21 §4).
//!
//! `locales/zh-CN.yml` is the single source of truth for translations: every
//! key is the English source string, and each entry carries `en` (mirroring
//! the key) and `zh-CN` lines. The retired `locales/i18n-keys.tsv` ledger
//! drifted against this file once already, so this test re-derives the key
//! set from the code itself and refuses to let the catalog drift again:
//!
//! 1. every literal `t!("...")` key in the app and engine sources has a
//!    non-empty `zh-CN` entry;
//! 2. the ledger has no duplicate keys;
//! 3. every ledger key is reachable from a literal `t!` call or from the
//!    runtime tables enumerated in [`ALLOWED_RUNTIME_KEYS`] below — a key
//!    nobody can look up is dead weight that hides real gaps;
//! 4. every `zh-CN` value interpolates exactly the key's `%{placeholder}`
//!    set, so a renamed argument cannot silently render literally.
//!
//! The reachability scanner is deliberately conservative: it skips `#[cfg(test)]`
//! items, dedicated test files, and the `gpui-test` capture harness, because
//! test-only usage is not production reachability (that distinction pruned
//! the `Name` and `Virtual microphone` keys).

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

/// The `zh-CN` line of one ledger entry.
struct Entry {
    zh_cn: String,
}

/// Parsed `zh-CN.yml`: ordered keys, entries, and any duplicated keys.
struct Ledger {
    keys: Vec<String>,
    entries: BTreeMap<String, Entry>,
    duplicates: Vec<String>,
}

/// A literal `t!` key with the file it was found in.
struct Usage {
    key: String,
    file: String,
}

/// Keys that reach `t!` through a runtime value instead of a literal. Each
/// group mirrors one table in the sources; keep the group in sync with the
/// match arms named in its comment. Entries that intentionally render
/// untranslated (for example the `"Unknown tint"` cause) fall back to English
/// and are therefore not listed here — only catalog keys need reachability.
const ALLOWED_RUNTIME_KEYS: &[&str] = &[
    // Toolbar page tabs: `Page::title` in src/ui/pages/mod.rs.
    "Graph",
    "Mixer",
    "Devices",
    "Matrix",
    // Node library cards: `NodeTemplate::title` in src/ui/pages/graph/node_catalog.rs.
    "Physical input",
    "Application audio",
    "Tone generator",
    "Gain",
    "Balance",
    "Compressor",
    "Parametric EQ",
    "Gate",
    "Limiter",
    "Delay",
    "Channel mapper",
    "Recorder",
    "Level meter",
    "Spectrum analyzer",
    "Processor rack",
    "Mix bus",
    "Device output",
    // Node library section headers: `NodeTemplate::group`.
    "SOURCES",
    "PROCESSING",
    "OBSERVATION",
    "ROUTING",
    "DESTINATIONS",
    // Node library card body: `NodeTemplate::description`.
    "WASAPI capture endpoint",
    "Process loopback source",
    "Prepared sine-wave source",
    "Adjust linear amplitude",
    "Stereo balance, mono and swap",
    "Linked peak dynamics",
    "One parametric peaking band",
    "Linked downward expander",
    "Zero-latency peak limiter",
    "Bounded feedback delay",
    "Static channel routing matrix",
    "Managed recording tap",
    "Peak and RMS observation",
    "Managed FFT observation",
    "Ordered processor chain",
    "Bus dynamic input ports",
    "WASAPI render endpoint",
    // Devices page source selector: `SourceMode::title` in src/ui/state.rs.
    "Test tone",
    "Physical input",
    "Application",
    // Settings/backend titles: `AudioBackend`, `ProcessingPrecision`, and
    // `SampleFormat` `title()` in crates/moiren-engine/src/audio_config.rs.
    "WASAPI",
    "ASIO",
    "32-bit float",
    "64-bit float",
    "Device native",
    "16-bit PCM",
    "24-bit PCM",
    "32-bit PCM",
    // Latency profile picker: `AudioLatencyProfile::title` and
    // `::description` in src/service/latency.rs.
    "Compatible",
    "Balanced",
    "Low",
    "Use each device's default period.",
    "Request ~10 ms device buffers (512 frames at 48 kHz).",
    "Request ~5 ms device buffers (256 frames at 48 kHz).",
    // Compressor inspector rows: `CompressorParameter::label` in
    // src/service/graph/compressor.rs.
    "Input gain",
    "Threshold",
    "Ratio",
    "Attack",
    "Release",
    "Hold",
    "Knee",
    "Makeup gain",
    "Output gain",
    "Dry / wet",
    // Window controls: `WindowAction::label` in src/ui/widgets/navigation.rs.
    "Minimize window",
    "Maximize window",
    "Close window",
    // Inspector sliders: `label_text:` fields in
    // src/ui/pages/graph/node_inspector.rs.
    "Frequency",
    "Gain",
    "Q",
    "Threshold",
    "Attack",
    "Release",
    "Hold",
    "Range",
    "Time",
    "Wet mix",
    "Feedback",
    "Input gain",
    // Mixer routing sections: title tables in src/ui/pages/mixer/routing.rs.
    "INPUTS",
    "SENDS",
    // Export eligibility tooltip: `DEFAULT_ELIGIBILITY_HELP` in
    // src/ui/pages/devices/exports.rs.
    "Default eligibility is permission on this computer only, not saved in the project. \
     Off prevents both Windows automatic and user manual default selection. On only permits \
     eligibility; it does not set a Windows default.",
    // Fault causes and hints produced by the service layer and translated at
    // render time (`t!(fault.cause.as_str())` / `t!(hint)` in
    // src/ui/shell/fault_popover.rs): edit validation in src/service/graph.rs
    "Mono and Swap only apply to Balance nodes",
    "Compressor parameters are outside their valid range",
    "Mixer gain or channel mode is invalid",
    "Node names cannot be empty and are limited to 256 bytes",
    "The Gain node's gain must be a finite value in 0..16",
    "The Balance node's position must be a finite value in -1..1",
    "Only Source nodes can bind an input source",
    "The input source configuration is invalid",
    "Only Sink nodes can bind a valid output device",
    "The graph revision space is exhausted",
    "Choose an input resource",
    "Choose an output device",
    "The node does not exist",
    "Compressor parameters only apply to Compressor nodes",
    "The graph is limited to 4096 nodes with at most 256 channels each",
    "Only Gain nodes have an adjustable gain",
    "Only Balance nodes have an adjustable position",
    // and src/service/graph/edit.rs,
    "The virtual export does not exist",
    "A Playback export can only bind an input node",
    "A Microphone export can only bind an output node",
    "The virtual export and graph node have different channel counts",
    // preparation in src/service/graph/preparation.rs,
    "The Source node is not bound to an input source",
    "Multiple Source nodes are bound to the same physical device",
    "The application source is missing a stable process identity",
    "Multiple Source nodes are bound to the same application source",
    "The Sink node is not bound to an output device",
    "Multiple Sink nodes are bound to the same device",
    "The graph needs a Source node",
    "The graph needs a Sink node",
    "Node state count does not match the graph, or the graph exceeds 4096 nodes",
    "Project node states do not match the topology identity",
    // managed driver faults in src/service/managed/driver.rs,
    "This channel is missing a usable processor configuration",
    "Delete this channel and re-add one of the same type",
    "The channel's configuration does not match its kind",
    "Check the channel's parameter settings, or delete and re-add it",
    "The configuration points at a channel that no longer exists",
    "The channel's channel count disagrees with its wiring",
    "Adjust the channel count, or reconnect the route",
    "The input/output device configuration is invalid",
    "Choose another device, or check the sample rate/precision settings",
    "Channel parameters are outside their valid range",
    "Parameters are outside their valid range",
    // edit hints in src/service/graph.rs `edit_hint`, and
    "Choose an input source at the top of the channel, or delete the channel",
    "Choose an output device at the top of the channel, or delete the channel",
    "Faders cover −∞ to +24 dB; cycle the channel-mode button back to a valid setting",
    "These two switches only apply to Balance channels",
    "Each device or application source can feed one channel; unbind the other one first",
    // remaining service errors in src/service/managed/parameters.rs,
    // src/service/managed/lifecycle.rs, and src/service/mod.rs.
    "The live parameter queue is busy; the candidate value is kept",
    "The apply request identity space is exhausted",
    "No native audio backend is configured",
];

fn manifest_dir() -> PathBuf {
    PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo test sets CARGO_MANIFEST_DIR"))
}

fn ledger_path() -> PathBuf {
    manifest_dir().join("locales").join("zh-CN.yml")
}

fn read_ledger() -> Ledger {
    parse_ledger(&fs::read_to_string(ledger_path()).expect("readable ledger"))
}

/// Parse the two-level ledger. The format is exactly three lines per entry:
/// `key:` (quoted when the key contains YAML-special characters), then
/// `  en: value`, then `  zh-CN: value`; `_version` is a format marker, not
/// a key. Any deviation fails loudly so the file cannot silently restructure.
fn parse_ledger(text: &str) -> Ledger {
    let lines: Vec<&str> = text
        .split('\n')
        .map(|line| line.trim_end_matches('\r'))
        .collect();
    let mut keys = Vec::new();
    let mut entries = BTreeMap::new();
    let mut duplicates = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        index += 1;
        if line.is_empty() || line.starts_with('#') || line.starts_with("_version:") {
            continue;
        }
        let line_number = index;
        let key = strip_key_line(line)
            .unwrap_or_else(|| panic!("{line_number}: not a ledger key line: {line:?}"));
        let en_line = lines
            .get(index)
            .unwrap_or_else(|| panic!("{line_number}: entry {key:?} has no `en` line"));
        let zh_line = lines
            .get(index + 1)
            .unwrap_or_else(|| panic!("{line_number}: entry {key:?} has no `zh-CN` line"));
        index += 2;
        // The `en` line must mirror the key; both lines must exist as fields.
        let en_value = field_value(en_line, "en")
            .unwrap_or_else(|| panic!("{line_number}: entry {key:?} has a malformed `en` line"));
        assert_eq!(
            en_value, key,
            "{line_number}: `en` line must mirror the key {key:?}"
        );
        let zh_value = field_value(zh_line, "zh-CN")
            .unwrap_or_else(|| panic!("{line_number}: entry {key:?} has a malformed `zh-CN` line"));
        if entries.contains_key(&key) {
            duplicates.push(key.clone());
        }
        keys.push(key.clone());
        entries.insert(key, Entry { zh_cn: zh_value });
    }
    Ledger {
        keys,
        entries,
        duplicates,
    }
}

/// The key of a top-level `key:` line, quoted or bare.
fn strip_key_line(line: &str) -> Option<String> {
    let trimmed = line.strip_suffix(':')?;
    if line.starts_with(' ') {
        None
    } else {
        Some(unquote(trimmed))
    }
}

/// The value of an `  en:` / `  zh-CN:` field line.
fn field_value(line: &str, field: &str) -> Option<String> {
    let rest = line
        .strip_prefix("  ")?
        .strip_prefix(field)?
        .strip_prefix(':')?;
    Some(unquote(rest.trim()))
}

/// Remove the surrounding double quotes and unescape `\"` / `\\`.
fn unquote(value: &str) -> String {
    let Some(inner) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) else {
        return value.to_string();
    };
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// Collect the Rust sources the reachability scan covers.
fn scanned_sources() -> Vec<PathBuf> {
    let manifest = manifest_dir();
    let roots = [
        manifest.join("src"),
        manifest.join("..").join("moiren-engine").join("src"),
    ];
    let mut files = Vec::new();
    for root in roots {
        collect_rust_files(&root, &root, &mut files);
    }
    files.sort();
    files
}

fn collect_rust_files(dir: &Path, root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(children) = fs::read_dir(dir) else {
        return;
    };
    for child in children.flatten() {
        let path = child.path();
        if path.is_dir() {
            collect_rust_files(&path, root, out);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            let relative = path.strip_prefix(root).expect("walked under root");
            if !is_test_only(relative) {
                out.push(path);
            }
        }
    }
}

/// Test-only sources: dedicated test modules (`tests.rs`, `*_tests.rs`, or a
/// `tests/` directory) and the `gpui-test` capture harness. Inline
/// `#[cfg(test)]` items are stripped from the remaining files instead.
fn is_test_only(relative: &Path) -> bool {
    let mut components = relative.components();
    let file_name = components.next_back().expect("file has a name").as_os_str();
    let file = file_name.to_string_lossy();
    if file == "tests.rs" || file.ends_with("_tests.rs") {
        return true;
    }
    if relative.starts_with("ui/shell/capture") {
        return true;
    }
    components.any(|component| component.as_os_str() == "tests")
}

/// Every literal `t!("...")` key in the scanned sources, with its file.
fn literal_usages() -> Vec<Usage> {
    let mut usages = Vec::new();
    for path in scanned_sources() {
        let source = fs::read_to_string(&path).expect("readable Rust source");
        for key in literal_keys(&strip_test_modules(&source)) {
            usages.push(Usage {
                key,
                file: path.display().to_string(),
            });
        }
    }
    usages
}

/// Strip `#[cfg(test)]` items, replacing each body with blank lines so the
/// line structure of the remaining text still matches the file. Covers
/// `mod name;` declarations and brace-delimited bodies (`mod`, `fn`, `impl`,
/// braced `use` trees), skipping strings, chars, lifetimes, and comments
/// while scanning for the opener and balancing braces.
fn strip_test_modules(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = String::with_capacity(source.len());
    let mut cursor = 0;
    while let Some(found) = source[cursor..].find("#[cfg(test)]") {
        let attribute = cursor + found;
        out.push_str(&source[cursor..attribute]);
        let after_attribute = attribute + "#[cfg(test)]".len();
        let Some(opener) = item_opener(source, after_attribute) else {
            // Malformed tail (cannot happen in valid Rust): keep it verbatim.
            out.push_str(&source[attribute..]);
            return out;
        };
        let end = if bytes[opener] == b';' {
            opener
        } else {
            match matching_brace(source, opener) {
                Some(close) => close,
                None => {
                    out.push_str(&source[attribute..]);
                    return out;
                }
            }
        };
        out.push_str(&"\n".repeat(source[attribute..=end].matches('\n').count()));
        cursor = end + 1;
    }
    out.push_str(&source[cursor..]);
    out
}

/// Index of the first item-level `{` or `;` after `from` — the body opener of
/// a braced item or the terminator of a declaration.
fn item_opener(source: &str, from: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut index = from;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => index = end_of_string(source, index)? - 1,
            b'\'' => index = end_of_char(source, index) - 1,
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index = skip_block_comment(source, index) - 1;
            }
            b'{' | b';' => return Some(index),
            _ => {}
        }
        index += 1;
    }
    None
}

/// Index of the first byte that is neither whitespace nor part of a comment.
fn skip_ws_and_comments(source: &str, mut index: usize) -> usize {
    let bytes = source.as_bytes();
    while index < bytes.len() {
        match bytes[index] {
            b' ' | b'\t' | b'\r' | b'\n' => index += 1,
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index = skip_block_comment(source, index);
            }
            _ => break,
        }
    }
    index
}

/// Index just past the block comment starting at `start`.
fn skip_block_comment(source: &str, start: usize) -> usize {
    let bytes = source.as_bytes();
    let mut index = start + 2;
    while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/') {
        index += 1;
    }
    (index + 2).min(bytes.len())
}

/// Index of the `}` closing the `{` at `open`, skipping Rust strings, char
/// literals, lifetimes, and comments; `None` when unbalanced.
fn matching_brace(source: &str, open: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut depth = 0usize;
    let mut index = open;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => index = end_of_string(source, index)? - 1,
            b'\'' => index = end_of_char(source, index) - 1,
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index = skip_block_comment(source, index) - 1;
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

/// Index just past the string literal starting at `start` (its opening quote).
fn end_of_string(source: &str, start: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut index = start + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 1,
            b'"' => return Some(index + 1),
            _ => {}
        }
        index += 1;
    }
    None
}

/// Index just past the char literal or lifetime starting at `start`.
fn end_of_char(source: &str, start: usize) -> usize {
    let bytes = source.as_bytes();
    // Lifetimes and loop labels (`'a`, `'_`, `'outer) have no closing quote.
    let is_lifetime = matches!(
        bytes.get(start + 1),
        Some(b'_') | Some(b'a'..=b'z') | Some(b'A'..=b'Z')
    ) && bytes.get(start + 2) != Some(&b'\'');
    if is_lifetime {
        return start + 2;
    }
    let mut index = start + 1;
    while index + 1 < bytes.len() {
        if bytes[index] == b'\\' {
            index += 1;
        } else if bytes[index] == b'\'' {
            return index + 1;
        }
        index += 1;
    }
    bytes.len()
}

/// Keys of every `t!(` call whose first argument is a string literal,
/// tolerating whitespace, comments, newlines, and adjacent-literal string
/// continuation inside the call. Non-literal arguments (runtime table
/// lookups) are skipped here and covered by [`ALLOWED_RUNTIME_KEYS`].
fn literal_keys(source: &str) -> Vec<String> {
    let bytes = source.as_bytes();
    let mut keys = Vec::new();
    let mut cursor = 0;
    while let Some(found) = source[cursor..].find("t!(") {
        let start = cursor + found;
        cursor = start + "t!(".len();
        // Reject longer macro names ending in `t!` (`assert!`-style matches
        // would otherwise poison the ledger with false keys).
        let previous = start.checked_sub(1).map(|index| bytes[index]);
        if previous.is_some_and(|byte| {
            byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'"' || byte == b'\''
        }) {
            continue;
        }
        let mut index = skip_ws_and_comments(source, cursor);
        let mut key = String::new();
        while let Some(literal) = string_literal_at(source, index) {
            key.push_str(&literal.value);
            index = skip_ws_and_comments(source, literal.end);
        }
        if !key.is_empty() {
            keys.push(key);
        }
    }
    keys
}

struct Literal {
    value: String,
    end: usize,
}

/// The string literal starting at `index`, if any (normal or raw, with any
/// number of `#`s). `end` points just past the closing quote. Only `index`
/// values on UTF-8 char boundaries reach here (after whitespace or a quote).
fn string_literal_at(source: &str, index: usize) -> Option<Literal> {
    let rest = &source[index..];
    let mut chars = rest.char_indices();
    match chars.next()?.1 {
        '"' => {
            let mut value = String::new();
            while let Some((offset, ch)) = chars.next() {
                match ch {
                    '\\' => {
                        let (_, escaped) = chars.next()?;
                        value.push(match escaped {
                            'n' => '\n',
                            't' => '\t',
                            'r' => '\r',
                            other => other,
                        });
                    }
                    '"' => {
                        return Some(Literal {
                            value,
                            end: index + offset + '"'.len_utf8(),
                        });
                    }
                    other => value.push(other),
                }
            }
            None
        }
        // Raw strings: r"..." or r##"..."##.
        'r' => {
            let after_r = rest.get(1..)?;
            if after_r.starts_with('#') || after_r.starts_with('"') {
                let hashes = after_r.bytes().take_while(|byte| *byte == b'#').count();
                let opening = 2 + hashes; // 'r', '#'s, '"'
                let body = rest.get(opening..)?;
                let closing = format!("\"{}", "#".repeat(hashes));
                let end = body.find(&closing)? + closing.len();
                Some(Literal {
                    value: body[..end - closing.len()].to_string(),
                    end: index + opening + end,
                })
            } else {
                None
            }
        }
        _ => None,
    }
}

/// The `%{placeholder}` names a string interpolates, as a set.
fn placeholders(text: &str) -> BTreeSet<String> {
    text.split("%{")
        .skip(1)
        .filter_map(|rest| rest.split('}').next().map(str::to_string))
        .collect()
}

#[test]
fn every_literal_translation_key_has_a_chinese_entry() {
    let ledger = read_ledger();
    let mut missing = Vec::new();
    for usage in literal_usages() {
        let translated = ledger
            .entries
            .get(&usage.key)
            .is_some_and(|entry| !entry.zh_cn.trim().is_empty());
        if !translated {
            missing.push(format!("{}: {:?}", usage.file, usage.key));
        }
    }
    assert!(
        missing.is_empty(),
        "literal t! keys without a zh-CN ledger entry (add the entry or fix the key):\n{}",
        missing.join("\n")
    );
}

#[test]
fn ledger_keys_are_unique() {
    let ledger = read_ledger();
    assert!(
        ledger.duplicates.is_empty(),
        "duplicate ledger keys: {:?}",
        ledger.duplicates
    );
}

#[test]
fn every_ledger_key_is_reachable() {
    let ledger = read_ledger();
    // Cross-table repetition (a key that is both a node title and a source
    // mode, say) is expected; the set deduplicates it.
    let allowed: BTreeSet<&str> = ALLOWED_RUNTIME_KEYS.iter().copied().collect();
    let literal: BTreeSet<String> = literal_usages()
        .into_iter()
        .map(|usage| usage.key)
        .collect();
    let dead: Vec<&str> = ledger
        .keys
        .iter()
        .filter(|key| !literal.contains(*key) && !allowed.contains(key.as_str()))
        .map(String::as_str)
        .collect();
    assert!(
        dead.is_empty(),
        "ledger keys no literal t! call or allowlisted runtime table can reach \
         (delete them, or extend the scanner/allowlist with the code that uses them):\n{}",
        dead.join("\n")
    );
}

#[test]
fn chinese_values_keep_the_keys_placeholders() {
    let ledger = read_ledger();
    let mismatched: Vec<String> = ledger
        .entries
        .iter()
        .filter(|(key, entry)| placeholders(key) != placeholders(&entry.zh_cn))
        .map(|(key, entry)| {
            format!(
                "{key:?} interpolates {:?} but zh-CN interpolates {:?}",
                placeholders(key),
                placeholders(&entry.zh_cn)
            )
        })
        .collect();
    assert!(
        mismatched.is_empty(),
        "zh-CN values whose placeholders diverge from their keys:\n{}",
        mismatched.join("\n")
    );
}
