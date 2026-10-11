use cranelift_isle::trie_again::{Overlap, RuleSet};

use crate::compiled;
use crate::lints::{Fix, FixEdit, Lint};
use crate::project::Loaded;
use crate::rules::{self, Rule};
use crate::syntax::Span;

pub const UNNEEDED: &str = "unneeded-priority";
pub const GAP: &str = "priority-gap";

const RULE_LIMIT: usize = 3000;

pub fn run(loaded: &Loaded) -> Vec<Lint> {
    let Ok(compiled) = compiled::compile(loaded, |path| path.display().to_string()) else { return Vec::new() };
    let all = rules::rules(loaded);
    let mut found = Vec::new();
    for (term, set) in &compiled.rule_sets {
        if set.rules.len() < 2 || set.rules.len() > RULE_LIMIT {
            continue;
        }
        let owners: Vec<Option<&Rule>> = set
            .rules
            .iter()
            .map(|rule| all.iter().find(|r| r.file == rule.pos.file && r.form.start <= rule.pos.offset && rule.pos.offset < r.form.end))
            .collect();
        let (levels, constrained) = levels(set);
        let renumber: Vec<FixEdit> = owners
            .iter()
            .zip(&levels)
            .filter_map(|(owner, &level)| owner.filter(|rule| rule.priority != level).map(|rule| edit(rule, level)))
            .collect();
        let name = compiled.term_name(*term);
        for ((owner, &level), constrained) in owners.iter().zip(&levels).zip(constrained) {
            let Some(rule) = owner else { continue };
            let Some(span) = rule.priority_span.filter(|_| rule.priority > level) else { continue };
            let (code, message) = if constrained {
                (GAP, format!("heuristic: priority {} could be {level}, which keeps the order of every overlapping rule of `{name}`", rule.priority))
            } else {
                (UNNEEDED, format!("heuristic: priority {} does not decide the order of any overlapping rules of `{name}`", rule.priority))
            };
            let edits = renumber.iter().map(|e| FixEdit { file: e.file, span: e.span, new_text: e.new_text.clone() }).collect();
            found.push(Lint {
                file: rule.file,
                span,
                code,
                message,
                related: Vec::new(),
                fix: Some(Fix { title: format!("Renumber the priorities of `{name}` minimally"), edits }),
            });
        }
    }
    found
}

fn levels(set: &RuleSet) -> (Vec<i64>, Vec<bool>) {
    let count = set.rules.len();
    let mut below: Vec<Vec<usize>> = vec![Vec::new(); count];
    let mut constrained = vec![false; count];
    for (at, a) in set.rules.iter().enumerate() {
        for (other, b) in set.rules.iter().enumerate().skip(at + 1) {
            if a.prio == b.prio || !matches!(a.may_overlap(b), Overlap::Yes { .. }) {
                continue;
            }
            let (high, low) = if a.prio > b.prio { (at, other) } else { (other, at) };
            below[high].push(low);
            constrained[high] = true;
            constrained[low] = true;
        }
    }
    let mut order: Vec<usize> = (0..count).collect();
    order.sort_by_key(|&at| set.rules[at].prio);
    let mut levels = vec![0i64; count];
    for at in order {
        levels[at] = below[at].iter().map(|&low| levels[low] + 1).max().unwrap_or(0);
    }
    (levels, constrained)
}

fn edit(rule: &Rule, level: i64) -> FixEdit {
    let text = rule.text;
    let (span, new_text) = match rule.priority_span {
        Some(span) if level == 0 => {
            let gap = text[span.end..].len() - text[span.end..].trim_start().len();
            (Span { start: span.start, end: span.end + gap }, String::new())
        }
        Some(span) => (span, level.to_string()),
        None => {
            let after = rule.name.or(rule.list.head().map(|head| head.span)).map_or(rule.form.start, |span| span.end);
            (Span { start: after, end: after }, format!(" {level}"))
        }
    };
    FixEdit { file: rule.file, span, new_text }
}
