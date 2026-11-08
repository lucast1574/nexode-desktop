//! Static extraction planning for top-level modules, including weighted cycles.
use anyhow::Result;
use clap::Parser;
use regex::Regex;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};
type Edges = BTreeMap<String, u64>;
#[derive(Default)]
struct Module {
    files: Vec<PathBuf>,
    entry: PathBuf,
    loc: u64,
    class: String,
    refs: Edges,
    blockers: Vec<String>,
    inbound: usize,
}
fn split(body: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (i, c) in body.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                let p = body[start..i].trim();
                if !p.is_empty() {
                    parts.push(p);
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    let tail = body[start..].trim();
    if !tail.is_empty() {
        parts.push(tail);
    }
    parts
}
fn imports(text: &str) -> (Edges, BTreeSet<String>) {
    let uses = Regex::new(r"\buse\s+crate::([^;]*);").unwrap();
    let ident = Regex::new(r"^[a-z_][a-z0-9_]*$").unwrap();
    let mut edges = Edges::new();
    let mut aliases = BTreeSet::new();
    for c in uses.captures_iter(text) {
        let rest = c[1].trim();
        let grouped = rest.starts_with('{');
        let entries = if grouped {
            split(&rest[1..rest.rfind('}').unwrap_or(rest.len())])
        } else {
            vec![rest]
        };
        for entry in entries {
            if matches!(entry, "self" | "*") {
                continue;
            }
            let head = entry
                .split("::")
                .next()
                .unwrap()
                .split(" as ")
                .next()
                .unwrap()
                .trim();
            if !ident.is_match(head) {
                continue;
            }
            if grouped {
                *edges.entry(head.to_owned()).or_default() += 1;
            }
            if !entry.contains("::")
                || Regex::new(&format!(r"^{}::\{{\s*self\b", regex::escape(head)))
                    .unwrap()
                    .is_match(entry)
            {
                aliases.insert(head.to_owned());
            }
        }
    }
    (edges, aliases)
}
fn refs(files: &[PathBuf], name: &str) -> Result<Edges> {
    let direct = Regex::new(r"\bcrate::([a-z_][a-z0-9_]*)")?;
    let mut counts = Edges::new();
    for file in files {
        let base = file.file_name().unwrap().to_string_lossy();
        if base == "tests.rs" || base.ends_with("_tests.rs") {
            continue;
        }
        let text = fs::read_to_string(file)?;
        let mut depth = 0_i64;
        let mut test_depth = None;
        let mut pending = false;
        let mut code = Vec::new();
        for line in text.split_inclusive('\n') {
            if line.trim().starts_with("#[cfg(test)]") {
                pending = true;
                continue;
            }
            if pending && line.contains('{') {
                test_depth = Some(depth);
                pending = false;
            }
            let delta = line.matches('{').count() as i64 - line.matches('}').count() as i64;
            depth += delta;
            if let Some(start) = test_depth {
                if depth <= start {
                    test_depth = None;
                }
                continue;
            }
            code.push(line);
            for c in direct.captures_iter(line) {
                *counts.entry(c[1].into()).or_default() += 1;
            }
        }
        let (group, mut aliases) = imports(&code.concat());
        for (k, v) in group {
            *counts.entry(k).or_default() += v;
        }
        aliases.remove(name);
        if !aliases.is_empty() {
            let regex = Regex::new(&format!(
                r"(?:^|[^\w:])({})::",
                aliases
                    .iter()
                    .map(|s| regex::escape(s))
                    .collect::<Vec<_>>()
                    .join("|")
            ))?;
            for line in code {
                if line.trim_start().starts_with("use ") {
                    continue;
                }
                for c in regex.captures_iter(line) {
                    *counts.entry(c[1].into()).or_default() += 1;
                }
            }
        }
    }
    counts.remove(name);
    Ok(counts)
}
fn modules(src: &Path) -> Result<BTreeMap<String, Module>> {
    let mut modules: BTreeMap<String, Module> = BTreeMap::new();
    let mut entries: Vec<_> = fs::read_dir(src)?.collect::<std::io::Result<_>>()?;
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        let kind = e.file_type()?;
        if kind.is_file() && p.extension().is_some_and(|x| x == "rs") {
            let n = p.file_stem().unwrap().to_string_lossy().into_owned();
            if matches!(n.as_str(), "lib" | "main") {
                continue;
            }
            let m = modules.entry(n).or_default();
            m.files.push(p.clone());
            m.entry = p;
        } else if kind.is_dir() {
            let mut files = Vec::new();
            crate::safety_scan::rust_files(&p, &mut files)?;
            if files.is_empty() {
                continue;
            }
            let m = modules
                .entry(e.file_name().to_string_lossy().into_owned())
                .or_default();
            if m.entry.as_os_str().is_empty() {
                m.entry = if p.join("mod.rs").is_file() {
                    p.join("mod.rs")
                } else {
                    files[0].clone()
                };
            }
            m.files.extend(files);
        }
    }
    let facade = Regex::new(r"pub use nexode_[a-z0-9_]+::")?;
    for (n, m) in &mut modules {
        for f in &m.files {
            m.loc += fs::read_to_string(f)?.lines().count() as u64;
        }
        let text = fs::read_to_string(&m.entry)?;
        let lines: Vec<_> = text
            .lines()
            .map(str::trim)
            .filter(|l| {
                !l.is_empty()
                    && !l.starts_with("//")
                    && !l.starts_with("#![")
                    && !l.starts_with("#[")
            })
            .collect();
        let facades = lines.iter().filter(|l| facade.is_match(l)).count();
        m.class = if facades == 0 || m.loc > 600 {
            "none"
        } else if facades * 2 >= lines.len() || lines.len() <= 8.max(facades + 4) {
            "fully"
        } else {
            "thick"
        }
        .into();
        m.refs = refs(&m.files, n)?;
    }
    let blockers: BTreeSet<_> = modules
        .iter()
        .filter(|(_, m)| m.class == "none")
        .map(|(n, _)| n.clone())
        .collect();
    let mut inbound: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (n, m) in &modules {
        for dep in m.refs.keys() {
            if modules.contains_key(dep) && dep != n {
                inbound.entry(dep.clone()).or_default().insert(n.clone());
            }
        }
    }
    for (n, m) in &mut modules {
        m.blockers = m
            .refs
            .keys()
            .filter(|d| blockers.contains(*d) && *d != n)
            .cloned()
            .collect();
        m.inbound = inbound.get(n).map_or(0, BTreeSet::len);
    }
    Ok(modules)
}
fn reachable(node: &str, graph: &BTreeMap<String, Edges>, seen: &mut BTreeSet<String>) {
    if !seen.insert(node.into()) {
        return;
    }
    if let Some(edges) = graph.get(node) {
        for n in edges.keys() {
            reachable(n, graph, seen);
        }
    }
}
fn cycles(graph: &BTreeMap<String, Edges>) -> Vec<Vec<String>> {
    let reach: BTreeMap<_, _> = graph
        .keys()
        .map(|n| {
            let mut seen = BTreeSet::new();
            reachable(n, graph, &mut seen);
            (n.clone(), seen)
        })
        .collect();
    let mut remaining: BTreeSet<_> = graph.keys().cloned().collect();
    let mut result = Vec::new();
    while let Some(n) = remaining.pop_first() {
        let mut component = vec![n.clone()];
        let members: Vec<_> = remaining
            .iter()
            .filter(|m| reach[&n].contains(*m) && reach[*m].contains(&n))
            .cloned()
            .collect();
        for m in members {
            remaining.remove(&m);
            component.push(m);
        }
        if component.len() > 1 {
            result.push(component);
        }
    }
    result
}
fn order(graph: &BTreeMap<String, Edges>) -> Vec<String> {
    let mut remaining: BTreeSet<_> = graph.keys().cloned().collect();
    let mut left = Vec::new();
    let mut right = Vec::new();
    while !remaining.is_empty() {
        loop {
            let mut changed = false;
            for n in remaining.clone() {
                if graph[&n].keys().all(|v| !remaining.contains(v)) {
                    right.insert(0, n.clone());
                    remaining.remove(&n);
                    changed = true;
                }
            }
            for n in remaining.clone() {
                if remaining.iter().all(|u| !graph[u].contains_key(&n)) {
                    left.push(n.clone());
                    remaining.remove(&n);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        if remaining.is_empty() {
            break;
        }
        let pick = remaining
            .iter()
            .max_by_key(|n| {
                let out: i128 = graph[*n]
                    .iter()
                    .filter(|(v, _)| remaining.contains(*v))
                    .map(|(_, v)| *v as i128)
                    .sum();
                let input: i128 = remaining
                    .iter()
                    .map(|u| graph[u].get(*n).copied().unwrap_or(0) as i128)
                    .sum();
                out - input
            })
            .unwrap()
            .clone();
        left.push(pick.clone());
        remaining.remove(&pick);
    }
    left.extend(right);
    left
}
#[derive(Parser)]
struct Args {
    #[arg(long)]
    json: bool,
    #[arg(long)]
    full: bool,
}
pub fn run(root: &Path, args: impl Iterator<Item = String>) -> Result<()> {
    let args = Args::parse_from(std::iter::once("analyze-root-crate".into()).chain(args));
    let info = modules(&root.join("src"))?;
    let mut extractable: Vec<_> = info
        .iter()
        .filter(|(n, m)| m.class == "none" && !n.ends_with("_tests") && m.blockers.is_empty())
        .map(|(n, _)| n.clone())
        .collect();
    extractable.sort_by_key(|n| std::cmp::Reverse(info[n].loc));
    if args.json {
        let modules:BTreeMap<String,Value>=info.iter().map(|(n,m)|(n.clone(),json!({"loc":m.loc,"facade_class":m.class,"inbound":m.inbound,"in_root_blockers":m.blockers}))).collect();
        println!(
            "{}",
            serde_json::to_string_pretty(
                &json!({"modules":modules,"extractable_now":extractable})
            )?
        );
        return Ok(());
    }
    let total: u64 = info.values().map(|m| m.loc).sum();
    let local: u64 = info
        .values()
        .filter(|m| m.class == "none")
        .map(|m| m.loc)
        .sum();
    let thick: u64 = info
        .values()
        .filter(|m| m.class == "thick")
        .map(|m| m.loc)
        .sum();
    println!(
        "root crate total loc (top-level modules): {total}\n  fully in-root (no crate yet):           {local}\n  thick facades (crate + residual local): {thick}\n  fully-facade loc:                       {}\n",
        total - local - thick
    );
    println!(
        "{:24} {:>7} {:>5} {:>4}  in-root blockers",
        "module", "loc", "fac", "in"
    );
    println!("{}", "-".repeat(90));
    let mut names: Vec<_> = info.keys().collect();
    names.sort_by_key(|n| std::cmp::Reverse(info[*n].loc));
    for n in names {
        let m = &info[n];
        if m.class == "fully" || n.ends_with("_tests") {
            continue;
        }
        println!(
            "{n:24} {:>7} {:>5} {:>4}  {}",
            m.loc,
            m.class,
            m.inbound,
            if m.blockers.is_empty() {
                "-- (none: extractable now)".into()
            } else {
                m.blockers.join(", ")
            }
        );
    }
    println!("\n=== Extractable now (no in-root blockers), largest first ===");
    for n in extractable {
        println!(
            "  {n:24} {:>7} loc, inbound={}",
            info[&n].loc, info[&n].inbound
        );
    }
    let graph: BTreeMap<_, _> = info
        .iter()
        .filter(|(_, m)| m.class != "fully")
        .map(|(n, m)| {
            (
                n.clone(),
                m.refs
                    .iter()
                    .filter(|(d, _)| info.get(*d).is_some_and(|m| m.class != "fully") && *d != n)
                    .map(|(d, w)| (d.clone(), *w))
                    .collect(),
            )
        })
        .collect();
    let mut cycles = cycles(&graph);
    cycles.sort_by_key(|c| std::cmp::Reverse(c.iter().map(|n| info[n].loc).sum::<u64>()));
    println!("\n=== Dependency cycles (SCCs > 1 node) — must break before clean split ===");
    if cycles.is_empty() {
        println!("  (none: the in-root module graph is already a DAG)");
    }
    for c in &cycles {
        let loc: u64 = c.iter().map(|n| info[n].loc).sum();
        let mut c = c.clone();
        c.sort_by_key(|n| std::cmp::Reverse(info[n].loc));
        println!("  [{} modules, {loc} loc] {}", c.len(), c.join(", "));
    }
    if let Some(big) = cycles.first() {
        let sub: BTreeMap<_, _> = big
            .iter()
            .map(|n| {
                (
                    n.clone(),
                    graph[n]
                        .iter()
                        .filter(|(d, _)| big.contains(d))
                        .map(|(d, w)| (d.clone(), *w))
                        .collect::<Edges>(),
                )
            })
            .collect();
        let order = order(&sub);
        let positions: BTreeMap<_, _> = order
            .iter()
            .enumerate()
            .map(|(i, n)| (n.clone(), i))
            .collect();
        let mut backwards = Vec::new();
        for (u, edges) in &sub {
            for (v, w) in edges {
                if positions[v] < positions[u] {
                    backwards.push((*w, u, v));
                }
            }
        }
        backwards.sort();
        println!(
            "\n=== Feedback arc set for the {}-module cycle ({} back-edges, {} refs to invert) ===",
            big.len(),
            backwards.len(),
            backwards.iter().map(|(w, _, _)| w).sum::<u64>()
        );
        let limit = if args.full { 1000 } else { 30 };
        for (w, u, v) in backwards.iter().take(limit) {
            println!("  {u} -> {v}   ({w} refs)");
        }
        if backwards.len() > limit {
            println!(
                "  ... and {} more (use --full to list all)",
                backwards.len() - limit
            );
        }
        let mut evict: Vec<_> = sub
            .iter()
            .map(|(n, edges)| {
                let mut edges: Vec<_> = edges.iter().map(|(d, w)| (*w, d)).collect();
                edges.sort();
                (
                    edges.len(),
                    edges.iter().map(|(w, _)| w).sum::<u64>(),
                    n,
                    edges,
                )
            })
            .collect();
        evict.sort_by_key(|(len, sum, n, _)| (*len, *sum, std::cmp::Reverse(info[*n].loc)));
        println!(
            "\n=== Cheapest modules to evict next from the cycle (fewest in-cycle out-edges first) ==="
        );
        let limit = if args.full { 1000 } else { 12 };
        for (len, sum, n, edges) in evict.iter().take(limit) {
            println!(
                "  {n:20} {:>7} loc  {len} edges / {sum} refs -> {}",
                info[*n].loc,
                edges
                    .iter()
                    .map(|(w, d)| format!("{d}({w})"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        if evict.len() > limit {
            println!(
                "  ... and {} more (use --full to list all)",
                evict.len() - limit
            );
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grouped_imports_and_aliases() {
        let (e, a) = imports(
            "use crate::{a, b::Thing, c::{self, X}, d as renamed};\nuse crate::plain;\nuse crate::other::Thing;",
        );
        assert_eq!(
            e,
            Edges::from([
                ("a".into(), 1),
                ("b".into(), 1),
                ("c".into(), 1),
                ("d".into(), 1)
            ])
        );
        assert_eq!(
            a,
            BTreeSet::from(["a".into(), "c".into(), "d".into(), "plain".into()])
        );
        assert_eq!(split("a, b::{X,Y},c"), vec!["a", "b::{X,Y}", "c"]);
    }
    #[test]
    fn weighted_cycle_and_acyclic_components() {
        let graph = BTreeMap::from([
            ("a".into(), Edges::from([("b".into(), 5)])),
            ("b".into(), Edges::from([("a".into(), 1), ("c".into(), 2)])),
            ("c".into(), Edges::new()),
        ]);
        assert_eq!(cycles(&graph), vec![vec!["a".to_owned(), "b".to_owned()]]);
        let order = order(&graph);
        assert_eq!(order.len(), 3);
        assert_eq!(order.last().unwrap(), "c");
    }
    #[test]
    fn sibling_facade_with_large_local_tree_is_not_extracted() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path();
        fs::create_dir(src.join("thing")).unwrap();
        fs::write(src.join("thing.rs"), "pub use nexode_base::*;\n").unwrap();
        fs::write(src.join("thing/local.rs"), "fn f() {}\n".repeat(601)).unwrap();
        let m = modules(src).unwrap();
        assert_eq!(m["thing"].loc, 602);
        assert_eq!(m["thing"].class, "none");
    }
}
