//! `cargo run -q -p xtask -- check-architecture [--write | --since <old baseline>]`: the layers,
//! the build paths and the file sizes of docs/architecture.md, held by a baseline that may only
//! shrink. `--since` also fails on any entry the baseline has that the given older copy of it does
//! not, so a line added to the baseline by hand is caught (CI passes the pull request's base).

mod layers;
mod ratchet;
mod rules;
mod workspace;

use std::path::Path;
use std::process::ExitCode;

const LAYERS: &str = "tools/xtask/layers.txt";
const BASELINE: &str = "tools/xtask/architecture-baseline.txt";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["check-architecture"] => check(false, None),
        ["check-architecture", "--write"] => check(true, None),
        ["check-architecture", "--since", old] => check(false, Some(Path::new(old))),
        _ => Err(
            "usage: cargo run -q -p xtask -- check-architecture [--write | --since <old baseline>]"
                .to_string(),
        ),
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("xtask: {e}");
            ExitCode::from(2)
        }
    }
}

/// Runs every rule; with `write`, rewrites the baseline (which may only shrink); with `since`,
/// also holds the baseline to that older copy of it.
fn check(write: bool, since: Option<&Path>) -> Result<bool, String> {
    let root = workspace::root();
    let read = |rel: &str| std::fs::read_to_string(root.join(rel)).ok();
    let placements =
        layers::Placements::parse(&read(LAYERS).ok_or_else(|| format!("{LAYERS} is missing"))?)?;
    let members = workspace::members(&root)?;

    let unplaced = rules::unplaced(&members, &placements);
    let mut findings = rules::layers(&members, &placements);
    findings.extend(rules::build_paths(&members, &placements, read));
    let (sized, soft) = rules::sizes(&line_counts(&root)?);
    findings.extend(sized);

    let existing = read(BASELINE);
    let baseline = ratchet::parse(existing.as_deref().unwrap_or(""))?;
    let verdict = ratchet::judge(&findings, &baseline);

    if write {
        if existing.is_some() && !verdict.new.is_empty() {
            eprintln!("The baseline may only shrink; these would raise it (fix them instead):");
            verdict
                .new
                .iter()
                .for_each(|f| eprintln!("  {} {}", f.rule, f.subject));
            return Ok(false);
        }
        std::fs::write(root.join(BASELINE), ratchet::render(&findings))
            .map_err(|e| format!("{BASELINE}: {e}"))?;
        println!("Wrote {BASELINE}: {} entries.", findings.len());
        return Ok(unplaced.is_empty());
    }

    report(&members, &unplaced, &verdict, soft);
    let added = match since {
        None => Vec::new(),
        Some(old) => {
            let old =
                std::fs::read_to_string(old).map_err(|e| format!("{}: {e}", old.display()))?;
            ratchet::added(&ratchet::parse(&old)?, &baseline)
        }
    };
    if !added.is_empty() {
        println!(
            "\nAdded to {BASELINE} since the base (the list may only shrink; fix these instead):"
        );
        added
            .iter()
            .for_each(|(rule, subject)| println!("  {rule} {subject}"));
    }
    Ok(unplaced.is_empty() && verdict.passes() && added.is_empty())
}

fn line_counts(root: &Path) -> Result<Vec<(String, usize)>, String> {
    let files = workspace::tracked(root, &["*.rs", "*.slint"])?;
    Ok(files
        .into_iter()
        .filter_map(|f| {
            let text = std::fs::read_to_string(root.join(&f)).ok()?;
            Some((f, text.lines().count()))
        })
        .collect())
}

fn report(members: &[workspace::Member], unplaced: &[String], v: &ratchet::Verdict, soft: usize) {
    println!(
        "Architecture: {} crates; {} known breaks kept in {BASELINE}; {} files over {} lines to split when next touched.",
        members.len(),
        v.kept,
        soft,
        rules::SOFT_LINES
    );
    for (f, was) in &v.grew {
        println!(
            "  grew: {} {} ({was} -> {} lines)",
            f.rule,
            f.subject,
            f.measure.unwrap_or(0)
        );
    }
    if !unplaced.is_empty() {
        println!("\nNot placed in {LAYERS} (give each a layer):");
        unplaced.iter().for_each(|d| println!("  {d}"));
    }
    if !v.new.is_empty() {
        println!("\nNew breaks of docs/architecture.md (fix them; the baseline does not take new entries):");
        for f in &v.new {
            match (f.rule, f.measure) {
                ("layer", _) => println!("  layer {}: a dependency that points up", f.subject),
                ("build-path", _) => println!(
                    "  build-path {}: reaches into another crate's folder",
                    f.subject
                ),
                (_, Some(n)) => println!(
                    "  size {}: {n} lines, over {}",
                    f.subject,
                    rules::HARD_LINES
                ),
                _ => println!("  {} {}", f.rule, f.subject),
            }
        }
    }
    if !v.fixed.is_empty() {
        println!("\nFixed, so the baseline must come down to keep the fix (cargo run -q -p xtask -- check-architecture --write):");
        v.fixed
            .iter()
            .for_each(|(rule, subject)| println!("  {rule} {subject}"));
    }
}
