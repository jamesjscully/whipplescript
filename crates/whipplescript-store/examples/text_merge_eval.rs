//! Score the text-merge engine against MINED, human-admitted merges
//! (spec/text-merge-spec.md §10's evaluation program; cases from
//! scripts/mine-merge-corpus.py).
//!
//! For each case the engine merges (base, ours, theirs) and is judged
//! against `result` — the body a human actually admitted:
//!
//! - composed-exact      clean merge, byte-identical to the human result
//! - composed-divergent  clean merge that DIFFERS from the human result —
//!   every one is inspected by hand: either a real failure mode (registry +
//!   fix) or a human choice the engine can't know (e.g. they dropped a hunk)
//! - escalated           honest conflict; the human resolved it by hand
//!
//! The zero-bad-merge gate lives in the inspection of composed-divergent:
//! a divergence that LOSES or MISPLACES one side's edit is a bad merge;
//! a divergence where the engine kept both edits and the human chose
//! otherwise is not.
//!
//! Usage: cargo run -p whipplescript-store --features native \
//!          --example text_merge_eval -- CASES.jsonl [DIVERGENT_DIR]

use std::collections::BTreeMap;
use std::io::BufRead;

use whipplescript_store::text_merge::{text_merge, TextMergeConfig, TextMergeOutcome};

fn main() {
    let mut args = std::env::args().skip(1);
    let cases_path = args
        .next()
        .expect("usage: text_merge_eval CASES.jsonl [DIVERGENT_DIR]");
    let divergent_dir = args.next();
    if let Some(dir) = &divergent_dir {
        std::fs::create_dir_all(dir).expect("divergent dir");
    }

    let config = TextMergeConfig::default();
    let file = std::fs::File::open(&cases_path).expect("open cases");
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut per_repo: BTreeMap<String, BTreeMap<&str, usize>> = BTreeMap::new();
    let mut divergent_index = 0usize;

    for line in std::io::BufReader::new(file).lines() {
        let line = line.expect("read line");
        if line.trim().is_empty() {
            continue;
        }
        let case: serde_json::Value = serde_json::from_str(&line).expect("case json");
        let field = |name: &str| case[name].as_str().expect(name).to_owned();
        let (base, ours, theirs, result) = (
            field("base"),
            field("ours"),
            field("theirs"),
            field("result"),
        );
        let verdict = if !config.within_size(&base, &ours, &theirs) {
            "oversized"
        } else {
            match text_merge(&base, &ours, &theirs, &config) {
                TextMergeOutcome::Clean { merged, .. } if merged == result => "composed-exact",
                TextMergeOutcome::Clean { merged, .. } => {
                    if let Some(dir) = &divergent_dir {
                        divergent_index += 1;
                        let stem = format!(
                            "{dir}/{divergent_index:03}-{}-{}",
                            case["repo"].as_str().unwrap_or("?"),
                            case["commit"].as_str().unwrap_or("?"),
                        );
                        std::fs::write(format!("{stem}.base"), &base)
                            .expect("write divergent base");
                        std::fs::write(format!("{stem}.ours"), &ours)
                            .expect("write divergent ours");
                        std::fs::write(format!("{stem}.theirs"), &theirs)
                            .expect("write divergent theirs");
                        std::fs::write(format!("{stem}.human"), &result)
                            .expect("write divergent human result");
                        std::fs::write(format!("{stem}.engine"), &merged)
                            .expect("write divergent engine result");
                    }
                    "composed-divergent"
                }
                TextMergeOutcome::Conflicted { .. } => "escalated",
            }
        };
        *counts.entry(verdict).or_default() += 1;
        *per_repo
            .entry(field("repo"))
            .or_default()
            .entry(verdict)
            .or_default() += 1;
        println!(
            "case\t{}\t{}\t{}\t{verdict}",
            case["repo"].as_str().unwrap_or("?"),
            case["commit"].as_str().unwrap_or("?"),
            case["file"].as_str().unwrap_or("?"),
        );
    }

    println!("== text-merge corpus eval ==");
    for (verdict, count) in &counts {
        println!("{verdict:>20}: {count}");
    }
    println!("-- per repo --");
    for (repo, verdicts) in &per_repo {
        let line: Vec<String> = verdicts
            .iter()
            .map(|(verdict, count)| format!("{verdict}={count}"))
            .collect();
        println!("{repo:>24}: {}", line.join(" "));
    }
}
