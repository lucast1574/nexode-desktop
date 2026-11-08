//! Lexical masking and test-only module classification for safety budgets.
use anyhow::Result;
use regex::Regex;
use std::sync::LazyLock;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};
fn mask(out: &mut [u8], start: usize, end: usize) {
    for b in &mut out[start..end] {
        if *b != b'\n' {
            *b = b' ';
        }
    }
}
pub fn code_only(text: &str) -> String {
    static RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"//[^\n]*|/\*|(?:br|r)(?P<hashes>\#*)"|b?"|b?'(?:\\.|[^'\\\n])'"#).unwrap()
    });
    let re = &*RE;
    let mut out = text.as_bytes().to_vec();
    let bytes = text.as_bytes();
    let mut pos = 0;
    while let Some(c) = re.captures_at(text, pos) {
        let m = c.get(0).unwrap();
        let mut end = m.end();
        if m.as_str() == "/*" {
            let mut depth = 1;
            while end < bytes.len() && depth > 0 {
                if bytes[end..].starts_with(b"/*") {
                    depth += 1;
                    end += 2;
                } else if bytes[end..].starts_with(b"*/") {
                    depth -= 1;
                    end += 2;
                } else {
                    end += 1;
                }
            }
        } else if let Some(hashes) = c.name("hashes") {
            let closing = format!("\"{}", hashes.as_str());
            end = text[end..]
                .find(&closing)
                .map_or(text.len(), |n| end + n + closing.len());
        } else if matches!(m.as_str(), "\"" | "b\"") {
            while end < bytes.len() {
                if bytes[end] == b'\\' {
                    end = (end + 2).min(bytes.len());
                } else if bytes[end] == b'"' {
                    end += 1;
                    break;
                } else {
                    end += 1;
                }
            }
        }
        mask(&mut out, m.start(), end);
        pos = end;
    }
    String::from_utf8(out).unwrap()
}
pub fn production(text: &str) -> String {
    let code = code_only(text);
    let mut out = code.as_bytes().to_vec();
    static CFG: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"#\s*\[\s*cfg\s*\(\s*(?:all\s*\(\s*)?test\s*[,)]").unwrap());
    let cfg = &*CFG;
    static ATTRS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^#\s*\[[^\]]*\]").unwrap());
    let attrs = &*ATTRS;
    static ITEM: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(?:pub(?:\([^)]*\))?\s+)?(?:(?:async|unsafe|const)\s+)*(?:mod|fn)\b").unwrap()
    });
    let item = &*ITEM;
    for m in cfg.find_iter(&code) {
        let Some(end) = code[m.end()..].find(']') else {
            continue;
        };
        let mut cursor = m.end() + end + 1;
        loop {
            cursor += code[cursor..].len() - code[cursor..].trim_start().len();
            if let Some(a) = attrs.find(&code[cursor..]) {
                cursor += a.end();
            } else {
                break;
            }
        }
        let Some(i) = item.find(&code[cursor..]) else {
            continue;
        };
        let Some(open) = code[cursor + i.end()..].find([';', '{']) else {
            continue;
        };
        let mut end = cursor + i.end() + open + 1;
        if code.as_bytes()[end - 1] == b'{' {
            let mut depth = 1;
            while end < code.len() && depth > 0 {
                match code.as_bytes()[end] {
                    b'{' => depth += 1,
                    b'}' => depth -= 1,
                    _ => {}
                }
                end += 1;
            }
            if depth != 0 {
                continue;
            }
        }
        mask(&mut out, m.start(), end);
    }
    String::from_utf8(out).unwrap()
}
pub fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let e = entry?;
        let path = e.path();
        let kind = e.file_type()?;
        if kind.is_dir() {
            if e.file_name() != "target" {
                rust_files(&path, out)?;
            }
        } else if kind.is_file() && path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    out.sort();
    Ok(())
}
fn modules() -> Regex {
    Regex::new(r"(?m)^[ \t]*(?P<attrs>(?:#\s*\[[^\]]*\]\s*)*)(?:pub(?:\([^)]*\))?\s+)?mod\s+(?P<name>\w+)\s*;").unwrap()
}
pub fn production_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for dir in ["src", "crates"] {
        rust_files(&root.join(dir), &mut files)?;
    }
    files.retain(|p| !super::size_budget::is_test(p.strip_prefix(root).unwrap()));
    let mut excluded = BTreeSet::new();
    let mut edges: BTreeMap<PathBuf, BTreeSet<PathBuf>> = BTreeMap::new();
    let module = modules();
    let alias = Regex::new(r#"#\s*\[\s*path\s*=\s*"([^"\n]+)"\s*\]"#).unwrap();
    for path in &files {
        let text = fs::read_to_string(path)?;
        let prod = production(&text);
        let names: BTreeSet<_> = module
            .captures_iter(&prod)
            .map(|c| c["name"].to_owned())
            .collect();
        let directory = if matches!(
            path.file_stem().and_then(|p| p.to_str()),
            Some("main" | "lib" | "mod")
        ) {
            path.parent().unwrap().to_owned()
        } else {
            path.with_extension("")
        };
        let children = edges.entry(path.canonicalize()?).or_default();
        for c in module.captures_iter(&code_only(&text)) {
            let attr = c.name("attrs").unwrap();
            let name = &c["name"];
            let candidates = if let Some(a) = alias.captures(&text[attr.start()..attr.end()]) {
                vec![path.parent().unwrap().join(&a[1])]
            } else {
                vec![
                    directory.join(format!("{name}.rs")),
                    directory.join(name).join("mod.rs"),
                ]
            };
            for child in candidates.into_iter().filter(|c| c.is_file()) {
                let child = child.canonicalize()?;
                if names.contains(name) {
                    children.insert(child);
                } else {
                    excluded.insert(child);
                }
            }
        }
    }
    let mut pending: Vec<_> = excluded.iter().cloned().collect();
    while let Some(p) = pending.pop() {
        if let Some(children) = edges.get(&p) {
            for c in children {
                if excluded.insert(c.clone()) {
                    pending.push(c.clone());
                }
            }
        }
    }
    let mut prod: BTreeSet<_> = edges
        .iter()
        .filter(|(p, _)| !excluded.contains(*p))
        .flat_map(|(_, c)| c.iter().cloned())
        .collect();
    let mut pending: Vec<_> = prod.iter().cloned().collect();
    while let Some(p) = pending.pop() {
        excluded.remove(&p);
        if let Some(children) = edges.get(&p) {
            for c in children {
                if prod.insert(c.clone()) {
                    pending.push(c.clone());
                }
            }
        }
    }
    let mut result = Vec::new();
    for p in files {
        if !excluded.contains(&p.canonicalize()?) {
            result.push(p);
        }
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn literals_nested_comments_lifetimes_and_test_signatures() {
        for signature in [
            "async fn",
            "pub(super) async fn",
            "unsafe fn",
            "const fn",
            "fn",
        ] {
            let source = format!(
                r###"// fake.unwrap(); {{
/* nested /* fake.ok() */ }} */
const TEXT: &str = r##"fake.unwrap(); {{"##;
#[cfg(test)] #[allow(dead_code)]
{signature} helper(
) {{ let c = '}}'; nested({{ fake.unwrap(); fake.ok(); }}); }}
fn real<'a>(value: &'a str) {{ real.unwrap(); real.ok(); }}
"###
            );
            let p = production(&source);
            assert!(!p.contains("fake"));
            assert!(p.contains("real.unwrap()"));
            assert!(p.contains("real.ok()"));
        }
    }
    #[test]
    fn malformed_or_non_test_cfg_cannot_hide_production() {
        for s in [
            "#[cfg(unix)] pub async fn real() { value.unwrap(); }",
            "#[cfg(test)] fn broken() { value.unwrap();",
        ] {
            assert!(production(s).contains("value.unwrap"));
        }
        let s = "#[cfg(test)] fn fake() {}\n#[cfg(test)] pub async fn multiline(\n) { fake.ok(); }\nfn real() { real.ok(); }";
        assert!(!production(s).contains("fake"));
        assert!(production(s).contains("real.ok"));
    }
    #[test]
    fn external_test_descendants_and_shared_production_references() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        for (p, s) in [
            (
                "src/lib.rs",
                "#[cfg(test)]\n#[path = \"renamed.rs\"]\nmod checks;\nmod production;",
            ),
            (
                "src/renamed.rs",
                "mod nested;\n#[path = \"shared.rs\"]\nmod shared;",
            ),
            ("src/renamed/nested.rs", "fn fake() { fake.unwrap(); }"),
            ("src/production.rs", "#[path = \"shared.rs\"]\nmod shared;"),
            ("src/shared.rs", "fn shared() { real.unwrap(); }"),
        ] {
            let p = root.join(p);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, s).unwrap();
        }
        let files = production_files(root).unwrap();
        assert!(!files.contains(&root.join("src/renamed.rs")));
        assert!(!files.contains(&root.join("src/renamed/nested.rs")));
        assert!(files.contains(&root.join("src/shared.rs")));
        assert!(files.contains(&root.join("src/production.rs")));
    }
}
