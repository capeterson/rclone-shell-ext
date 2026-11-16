// SPDX-License-Identifier: GPL-3.0-only
//! Keeps `i18n/en/rclone_shell_ext.ftl` and the code that uses it in step. The four rules live in
//! `docs/architecture.md` ("UI text"):
//!
//! 1. every id referenced from Rust, QML or C++ exists;
//! 2. no id is unreferenced;
//! 3. ids read through QML or C++ (which cannot pass arguments) have no variables;
//! 4. no hard-coded user-visible literals in QML, iced widgets or C++ labels, unless the line
//!    says `i18n-ignore: <reason>`.
//!
//! Rule 4 skips the files in `pending_literal_paths.txt` (not converted yet) and fails if a
//! listed file no longer has any violation, so the list can only shrink.

use regex::Regex;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const ID: &str = r"[a-z][a-z0-9]*(?:-[a-z0-9]+)*";
/// The one place this scan must not look: this file's own samples.
const SELF_PATH: &str = "crates/i18n/tests/strings_sync.rs";
/// The start of a QML text-ish property assignment.
const QML_PROP: &str =
    r"(?:^|[^A-Za-z0-9_.])(?:ToolTip\.)?(?:text|title|placeholderText|subtitle|label)";
const SKIP_DIRS: [&str; 5] = ["target", ".git", "node_modules", "build", ".cache"];

// ---- scanning helpers ----

/// A reference to a message id found in a source file.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone)]
struct Reference {
    id: String,
    path: String,
    line: usize,
}

/// A hard-coded user-visible literal.
#[derive(Debug, PartialEq, Eq, Clone)]
struct Violation {
    path: String,
    line: usize,
    text: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Lang {
    Rust,
    Qml,
    Cpp,
}

fn lang_of(path: &Path) -> Option<Lang> {
    match path.extension()?.to_str()? {
        "rs" => Some(Lang::Rust),
        "qml" => Some(Lang::Qml),
        "cpp" | "cc" | "h" | "hpp" => Some(Lang::Cpp),
        _ => None,
    }
}

/// The text with whole-line comments blanked (line numbers stay the same). Trailing comments on
/// code lines stay, because `i18n-ignore:` lives there.
fn blank_comment_lines(text: &str) -> String {
    text.lines()
        .map(|l| {
            let t = l.trim_start();
            if t.starts_with("//") || t.starts_with("/*") || t.starts_with('*') {
                ""
            } else {
                l
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn line_of(text: &str, offset: usize) -> usize {
    text[..offset].bytes().filter(|b| *b == b'\n').count() + 1
}

/// Ids a file refers to: Rust `fl!("id")`, QML `Strings.t("id")` / `s["id"]`, C++ `text("id")`.
fn references(lang: Lang, path: &str, source: &str) -> Vec<Reference> {
    let re = match lang {
        Lang::Rust => format!(r#"\bfl!\(\s*"({ID})""#),
        Lang::Qml => format!(r#"\bStrings\.t\(\s*["']({ID})["']|\bs\[\s*["']({ID})["']"#),
        Lang::Cpp => format!(r#"\btext\(\s*"({ID})""#),
    };
    let re = Regex::new(&re).unwrap();
    let code = blank_comment_lines(source);
    re.captures_iter(&code)
        .map(|c| {
            let m = c.get(1).or_else(|| c.get(2)).unwrap();
            Reference {
                id: m.as_str().to_string(),
                path: path.to_string(),
                line: line_of(&code, m.start()),
            }
        })
        .collect()
}

/// Hard-coded user-visible literals in a file (rule 4), before the pending list is applied.
fn literals(lang: Lang, path: &str, source: &str) -> Vec<Violation> {
    // A literal only counts if it has a letter in it: "…" and "—" are glyphs, not words.
    let lit = r#""[^"\n]*[A-Za-z][^"\n]*""#;
    let patterns: Vec<String> = match lang {
        Lang::Rust => vec![
            format!(r"\btext\(\s*{lit}"),
            format!(r"::text::[a-z0-9_]+\(\s*{lit}"),
            format!(r"\.title\(\s*{lit}"),
            format!(r"\.description\(\s*{lit}"),
            format!(r"\bbutton::(?:standard|suggested|destructive)\(\s*{lit}"),
            format!(r"\bitem::builder\(\s*{lit}"),
            // Window names Qt shows in the title bar and task switcher.
            format!(r"set_application_display_name\(\s*&?QString::from\(\s*{lit}"),
        ],
        Lang::Qml => vec![
            format!(r"{QML_PROP}\s*:\s*{lit}"),
            // Plasma's own `i18n("…")` / `qsTr("…")` wrappers are still hard-coded wording.
            format!(r"\b(?:i18n[a-z]*|qsTr)\(\s*{lit}"),
            // `text: busy ? "A" : "B"`: a literal right after the `?` or the `:` of a ternary.
            format!(r"{QML_PROP}\s*:\s*[^\n]*?\?\s*{lit}"),
            format!(r"{QML_PROP}\s*:\s*[^\n]*?\?[^\n]*?:\s*{lit}"),
        ],
        Lang::Cpp => vec![format!(r"\bi18n\(\s*{lit}"), format!(r"\btr\(\s*{lit}")],
    };
    let code = blank_comment_lines(source);
    let lines: Vec<&str> = source.lines().collect();
    let ignored = |line: usize| {
        lines
            .get(line - 1)
            .is_some_and(|l| l.contains("i18n-ignore:"))
    };
    let mut out = Vec::new();
    let mut push = |start: usize, end: usize| {
        let line = line_of(&code, start);
        // The ignore comment may sit on the line where the literal ends.
        if !ignored(line) && !ignored(line_of(&code, end)) {
            out.push(Violation {
                path: path.to_string(),
                line,
                text: code[start..end].trim().to_string(),
            });
        }
    };
    for p in patterns {
        let re = Regex::new(&format!("(?m){p}")).unwrap();
        for m in re.find_iter(&code) {
            push(m.start(), m.end());
        }
    }
    if lang == Lang::Cpp {
        // `QStringLiteral("…")` is everywhere (D-Bus names, icon names); it is a label only when
        // it feeds a widget text, which is what these calls take.
        let icon = Regex::new(
            r"QIcon::fromTheme\(\s*(?:QStringLiteral|QString::fromUtf8)\(\s*\x22[^\x22]*\x22\s*\)\s*\)",
        )
        .unwrap();
        let label_call =
            Regex::new(r"\b(?:setText|setToolTip|setWindowTitle|setTitle|addAction|QAction)\(")
                .unwrap();
        let sl = Regex::new(&format!(r"(?:QStringLiteral|QString::fromUtf8)\(\s*{lit}")).unwrap();
        let mut offset = 0;
        for line in code.split('\n') {
            let stripped = icon.replace_all(line, "");
            if label_call.is_match(&stripped) && sl.is_match(&stripped) {
                push(offset, offset + line.len());
            }
            offset += line.len() + 1;
        }
    }
    out.sort_by_key(|v| v.line);
    out.dedup_by_key(|v| v.line);
    out
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn source_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if p.is_dir() {
            if !SKIP_DIRS.contains(&name.as_str()) {
                source_files(&p, out);
            }
        } else if lang_of(&p).is_some() {
            out.push(p);
        }
    }
}

struct Scan {
    refs: Vec<(Lang, Reference)>,
    violations: Vec<Violation>,
    scanned: BTreeSet<String>,
}

fn scan_repo() -> Scan {
    let root = repo_root();
    let mut files = Vec::new();
    source_files(&root, &mut files);
    files.sort();
    let mut scan = Scan {
        refs: vec![],
        violations: vec![],
        scanned: BTreeSet::new(),
    };
    for f in files {
        let rel = f
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if rel == SELF_PATH {
            continue;
        }
        let lang = lang_of(&f).unwrap();
        let Ok(text) = std::fs::read_to_string(&f) else {
            continue;
        };
        scan.scanned.insert(rel.clone());
        scan.refs
            .extend(references(lang, &rel, &text).into_iter().map(|r| (lang, r)));
        scan.violations.extend(literals(lang, &rel, &text));
    }
    scan
}

fn pending_paths() -> BTreeSet<String> {
    let text = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/pending_literal_paths.txt"),
    )
    .unwrap();
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

// ---- the checks on the real repository ----

#[test]
fn every_referenced_id_exists() {
    let known: BTreeSet<_> = rclone_ui_i18n::message_ids().into_iter().collect();
    let missing: Vec<_> = scan_repo()
        .refs
        .into_iter()
        .filter(|(_, r)| !known.contains(&r.id))
        .map(|(_, r)| format!("{}:{}: unknown message id `{}`", r.path, r.line, r.id))
        .collect();
    assert!(missing.is_empty(), "\n{}", missing.join("\n"));
}

#[test]
fn every_id_is_referenced() {
    let used: BTreeSet<_> = scan_repo().refs.into_iter().map(|(_, r)| r.id).collect();
    let unused: Vec<_> = rclone_ui_i18n::message_ids()
        .into_iter()
        .filter(|id| !used.contains(id))
        .collect();
    assert!(
        unused.is_empty(),
        "ids in i18n/en/rclone_shell_ext.ftl that nothing uses (delete them or use them):\n{}",
        unused.join("\n")
    );
}

#[test]
fn ids_read_from_qml_and_cpp_have_no_variables() {
    let exported = rclone_ui_i18n::static_messages();
    let known: BTreeSet<_> = rclone_ui_i18n::message_ids().into_iter().collect();
    let bad: Vec<_> = scan_repo()
        .refs
        .into_iter()
        .filter(|(lang, r)| {
            matches!(lang, Lang::Qml | Lang::Cpp)
                && known.contains(&r.id)
                && !exported.contains_key(&r.id)
        })
        .map(|(_, r)| {
            format!(
                "{}:{}: `{}` has variables; format it in Rust and pass it finished",
                r.path, r.line, r.id
            )
        })
        .collect();
    assert!(bad.is_empty(), "\n{}", bad.join("\n"));
}

#[test]
fn no_hard_coded_user_visible_literals() {
    let scan = scan_repo();
    let pending = pending_paths();
    for p in &pending {
        assert!(
            scan.scanned.contains(p),
            "pending_literal_paths.txt lists `{p}`, which does not exist"
        );
    }
    let new: Vec<_> = scan
        .violations
        .iter()
        .filter(|v| !pending.contains(&v.path))
        .map(|v| {
            format!(
                "{}:{}: {}\n    use a message id, or end the line with `i18n-ignore: <reason>`",
                v.path, v.line, v.text
            )
        })
        .collect();
    assert!(new.is_empty(), "\n{}", new.join("\n"));
    let stale: Vec<_> = pending
        .iter()
        .filter(|p| !scan.violations.iter().any(|v| &v.path == *p))
        .collect();
    assert!(
        stale.is_empty(),
        "these files are converted; remove them from pending_literal_paths.txt: {stale:?}"
    );
}

// ---- the scanners themselves ----

fn ids(lang: Lang, src: &str) -> Vec<String> {
    references(lang, "x", src)
        .into_iter()
        .map(|r| r.id)
        .collect()
}

fn hits(lang: Lang, src: &str) -> usize {
    literals(lang, "x", src).len()
}

#[test]
fn finds_rust_ids_across_lines_and_skips_comments() {
    let src = "let a = fl!(\"one-id\");\nlet b = fl!(\n    \"two-id\",\n    count = 1\n);\n// fl!(\"commented\")\n/// `fl!(\"doc-id\")`\n";
    assert_eq!(ids(Lang::Rust, src), ["one-id", "two-id"]);
}

#[test]
fn finds_qml_ids_both_ways() {
    let src = "text: Strings.t(\"details-open\")\nlabel: s[\"panel-open\"]\nx: someVars[\"nope-id\"]\n// Strings.t(\"gone\")\n";
    assert_eq!(ids(Lang::Qml, src), ["details-open", "panel-open"]);
}

#[test]
fn finds_cpp_ids() {
    let src = "m_client->text(\"dolphin-details\");\nsetText(i18n(\"x\"));\n";
    assert_eq!(ids(Lang::Cpp, src), ["dolphin-details"]);
}

#[test]
fn qml_literals_are_flagged() {
    assert_eq!(hits(Lang::Qml, "Button { text: \"Open\" }"), 1);
    assert_eq!(hits(Lang::Qml, "  title: \"Settings\""), 1);
    assert_eq!(hits(Lang::Qml, "  ToolTip.text: \"Help\""), 1);
    assert_eq!(hits(Lang::Qml, "  placeholderText: \"Search\""), 1);
    assert_eq!(hits(Lang::Qml, "  subtitle: \"x y\""), 1);
    assert_eq!(hits(Lang::Qml, "  label: \"Name\""), 1);
    assert_eq!(hits(Lang::Qml, "  x: i18n(\"Open\")"), 1);
}

#[test]
fn qml_non_literals_are_not_flagged() {
    assert_eq!(hits(Lang::Qml, "  text: Strings.t(\"a-b\")"), 0);
    assert_eq!(hits(Lang::Qml, "  text: \"…\""), 0);
    assert_eq!(hits(Lang::Qml, "  text: \"\""), 0);
    assert_eq!(hits(Lang::Qml, "  subtext: \"Open\""), 0);
    assert_eq!(hits(Lang::Qml, "  // text: \"Open\""), 0);
    assert_eq!(
        hits(Lang::Qml, "  text: \"Open\" // i18n-ignore: test fixture"),
        0
    );
    assert_eq!(
        hits(
            Lang::Qml,
            "  text: \"Open\" /* i18n-ignore: test fixture */"
        ),
        0
    );
}

#[test]
fn iced_literals_are_flagged() {
    assert_eq!(hits(Lang::Rust, "widget::text(\"Hello\")"), 1);
    assert_eq!(hits(Lang::Rust, "button::text(\"Go\")"), 1);
    assert_eq!(hits(Lang::Rust, "w.title(\"Name\")"), 1);
    assert_eq!(hits(Lang::Rust, "widget::text::body(\"Some text\")"), 1);
    assert_eq!(hits(Lang::Rust, "text(\n    \"split\"\n)"), 1);
}

#[test]
fn more_iced_and_qt_host_literals_are_flagged() {
    assert_eq!(hits(Lang::Rust, "widget::button::standard(\"Cancel\")"), 1);
    assert_eq!(hits(Lang::Rust, "button::suggested(\"Save\")"), 1);
    assert_eq!(hits(Lang::Rust, "button::destructive(\"Delete\")"), 1);
    assert_eq!(hits(Lang::Rust, "settings::item::builder(\"Folder\")"), 1);
    assert_eq!(hits(Lang::Rust, "row.description(\"Some words\")"), 1);
    assert_eq!(
        hits(
            Lang::Rust,
            "app.set_application_display_name(&QString::from(\"rclone\"))"
        ),
        1
    );
    assert_eq!(
        hits(
            Lang::Rust,
            "x.set_application_display_name(&QString::from(name))"
        ),
        0
    );
    assert_eq!(
        hits(Lang::Rust, "button::standard(fl!(\"common-cancel\"))"),
        0
    );
    assert_eq!(hits(Lang::Rust, "row.description(when)"), 0);
}

#[test]
fn qml_ternary_literals_are_flagged() {
    assert_eq!(hits(Lang::Qml, "  text: busy ? \"Working\" : \"Idle\""), 1);
    assert_eq!(hits(Lang::Qml, "  text: busy ? s[\"a-b\"] : \"Idle\""), 1);
    assert_eq!(hits(Lang::Qml, "  title: ok ? \"Done\" : s[\"a-b\"]"), 1);
    assert_eq!(hits(Lang::Qml, "  text: busy ? s[\"a-b\"] : s[\"c-d\"]"), 0);
    assert_eq!(
        hits(Lang::Qml, "  text: busy ? Strings.t(\"a-b\") : \"\""),
        0
    );
    assert_eq!(hits(Lang::Qml, "  text: busy ? \"…\" : \"\""), 0);
    assert_eq!(
        hits(
            Lang::Qml,
            "  text: busy ? \"Working\" : \"Idle\" // i18n-ignore: fixture"
        ),
        0
    );
    assert_eq!(hits(Lang::Qml, "  visible: busy ? \"yes\" : \"no\""), 0);
}

#[test]
fn cpp_tr_and_from_utf8_labels_are_flagged() {
    assert_eq!(hits(Lang::Cpp, "action->setText(tr(\"Open\"));"), 1);
    assert_eq!(hits(Lang::Cpp, "auto s = QObject::tr(\"Open\");"), 1);
    assert_eq!(
        hits(Lang::Cpp, "action->setText(QString::fromUtf8(\"Open\"));"),
        1
    );
    assert_eq!(
        hits(Lang::Cpp, "menu->addAction(QString::fromUtf8(\"Open\"));"),
        1
    );
    assert_eq!(
        hits(
            Lang::Cpp,
            "menu->addAction(QIcon::fromTheme(QString::fromUtf8(\"folder\")), m_client->text(\"a-b\"));"
        ),
        0
    );
    assert_eq!(
        hits(Lang::Cpp, "auto id = QString::fromUtf8(\"com.example\");"),
        0
    );
    assert_eq!(hits(Lang::Cpp, "str(\"not tr\")"), 0);
}

#[test]
fn iced_non_literals_are_not_flagged() {
    assert_eq!(hits(Lang::Rust, "widget::text(fl!(\"a-b\"))"), 0);
    assert_eq!(hits(Lang::Rust, "widget::text(\"…\")"), 0);
    assert_eq!(
        hits(
            Lang::Rust,
            "widget::text(\"Hello\") // i18n-ignore: debug overlay"
        ),
        0
    );
    assert_eq!(hits(Lang::Rust, "format_text(\"Hello\")"), 0);
    assert_eq!(hits(Lang::Rust, "// widget::text(\"Hello\")"), 0);
}

#[test]
fn cpp_literals_are_flagged() {
    assert_eq!(hits(Lang::Cpp, "menu->addAction(i18n(\"Details…\"));"), 1);
    assert_eq!(
        hits(Lang::Cpp, "action->setText(QStringLiteral(\"Open\"));"),
        1
    );
    assert_eq!(
        hits(
            Lang::Cpp,
            "new QAction(QStringLiteral(\"Keep offline\"), parent);"
        ),
        1
    );
}

#[test]
fn cpp_non_labels_are_not_flagged() {
    assert_eq!(
        hits(
            Lang::Cpp,
            "QString kService = QStringLiteral(\"com.example.App\");"
        ),
        0
    );
    assert_eq!(
        hits(Lang::Cpp, "menu->addAction(QIcon::fromTheme(QStringLiteral(\"folder\")), m_client->text(\"a-b\"));"),
        0
    );
    assert_eq!(hits(Lang::Cpp, "#define i18n(s) translate(s)"), 0);
    assert_eq!(
        hits(
            Lang::Cpp,
            "x->setText(i18n(\"Open\")); // i18n-ignore: fixture"
        ),
        0
    );
}
