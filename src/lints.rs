use crate::project::Loaded;
use crate::rules::{self, Rule, Shape};
use crate::syntax::{Node, Span};

pub const COSTLIER_RHS: &str = "costlier-rhs";
pub const NO_OP: &str = "no-op-rewrite";
pub const SHADOWED: &str = "shadowed-by-priority";
pub const LOOP: &str = "rewrite-loop";
pub const TWIN: &str = "missing-commutative-twin";

const COMMUTATIVE: [&str; 8] = ["iadd", "imul", "band", "bor", "bxor", "smin", "smax", "umin"];

pub struct Fix {
    pub title: String,
    pub span: Span,
    pub new_text: String,
}

pub struct Lint {
    pub file: usize,
    pub span: Span,
    pub code: &'static str,
    pub message: String,
    pub related: Vec<(usize, Span)>,
    pub fix: Option<Fix>,
}

struct Info<'a> {
    rule: &'a Rule<'a>,
    lhs: Option<Shape>,
    rhs: Shape,
}

fn infos<'a>(all: &'a [Rule<'a>]) -> Vec<Info<'a>> {
    all.iter()
        .map(|rule| Info {
            rule,
            lhs: (rule.head == "simplify" && rule.args.len() == 1)
                .then(|| rules::shape(rule.text, &rule.args[0]))
                .filter(|shape| matches!(shape, Shape::Call(..))),
            rhs: rules::shape(rule.text, rule.rhs),
        })
        .collect()
}

pub fn run(loaded: &Loaded, disabled: &[String]) -> Vec<Lint> {
    let all = rules::rules(loaded);
    let infos = infos(&all);
    let mut found = Vec::new();
    no_op_and_cost(&infos, &mut found);
    shadowed(&infos, &mut found);
    loops(&infos, &mut found);
    twins(&infos, &mut found);
    found.retain(|lint| !disabled.iter().any(|code| code == lint.code));
    found
}

pub fn twin_fix(loaded: &Loaded, file: usize, offset: usize) -> Option<Fix> {
    let all = rules::rules(loaded);
    let infos = infos(&all);
    let mut found = Vec::new();
    twins(&infos, &mut found);
    found
        .into_iter()
        .find(|lint| lint.file == file && lint.span.start <= offset && offset <= lint.span.end)
        .and_then(|lint| lint.fix)
}

fn lint(info: &Info, code: &'static str, message: String) -> Lint {
    Lint { file: info.rule.file, span: info.rule.form, code, message, related: Vec::new(), fix: None }
}

fn no_op_and_cost(infos: &[Info], found: &mut Vec<Lint>) {
    for info in infos {
        let Some(lhs) = &info.lhs else { continue };
        if *lhs == info.rhs {
            found.push(lint(info, NO_OP, "heuristic: this rule rewrites an expression to itself".to_owned()));
            continue;
        }
        if matches!(info.rhs, Shape::Other) {
            continue;
        }
        let (left, right) = (rules::cost(lhs), rules::cost(&info.rhs));
        if right > left {
            let message = format!("heuristic: the right-hand side has more operations than the left-hand side ({right} against {left})");
            found.push(lint(info, COSTLIER_RHS, message));
        }
    }
}

fn shadowed(infos: &[Info], found: &mut Vec<Lint>) {
    let call = |info: &Info| Shape::Call(info.rule.head.to_owned(), rules::shapes(info.rule.text, info.rule.args));
    for info in infos {
        let own = call(info);
        let winner = infos.iter().find(|other| {
            other.rule.head == info.rule.head
                && other.rule.priority > info.rule.priority
                && other.rule.clauses.is_empty()
                && rules::subsumes(&call(other), &own)
        });
        if let Some(winner) = winner {
            let mut lint = lint(
                info,
                SHADOWED,
                "heuristic: this rule can never fire, a rule with a higher priority matches everything it matches".to_owned(),
            );
            lint.related.push((winner.rule.file, winner.rule.form));
            found.push(lint);
        }
    }
}

fn loops(infos: &[Info], found: &mut Vec<Lint>) {
    let leads_to = |from: usize, to: usize| {
        let (Some(lhs), Shape::Call(..)) = (&infos[to].lhs, &infos[from].rhs) else { return false };
        if !infos[from].rule.clauses.is_empty() || !infos[to].rule.clauses.is_empty() {
            return false;
        }
        if !rules::subsumes(lhs, &infos[from].rhs) {
            return false;
        }
        from != to
            || infos[from].lhs.as_ref().is_some_and(|own| *own != infos[from].rhs && rules::cost(&infos[from].rhs) >= rules::cost(own))
    };
    for from in 0..infos.len() {
        if leads_to(from, from) {
            found.push(lint(
                &infos[from],
                LOOP,
                "heuristic: the result of this rule matches its own left-hand side, so it may rewrite forever".to_owned(),
            ));
            continue;
        }
        let partner = (0..infos.len()).find(|&to| to != from && leads_to(from, to) && leads_to(to, from));
        if let Some(to) = partner {
            let mut lint = lint(
                &infos[from],
                LOOP,
                "heuristic: the result of this rule can be rewritten back by another rule, which may loop".to_owned(),
            );
            lint.related.push((infos[to].rule.file, infos[to].rule.form));
            found.push(lint);
        }
    }
}

fn is_constant(shape: &Shape) -> bool {
    match shape {
        Shape::Int(_) => true,
        Shape::Call(head, _) => head.contains("iconst"),
        _ => false,
    }
}

fn twins(infos: &[Info], found: &mut Vec<Lint>) {
    for info in infos {
        let Some(lhs @ Shape::Call(op, args)) = &info.lhs else { continue };
        let Some(Node::List(list)) = info.rule.args.first() else { continue };
        if !COMMUTATIVE.contains(&op.as_str()) || args.len() < 2 || list.items.len() != args.len() + 1 {
            continue;
        }
        let last = args.len() - 1;
        if is_constant(&args[last - 1]) == is_constant(&args[last]) {
            continue;
        }
        let mut flipped = args.clone();
        flipped.swap(last - 1, last);
        let flipped = Shape::Call(op.clone(), flipped);
        if rules::same_pattern(lhs, &flipped) {
            continue;
        }
        let exists = infos.iter().any(|other| other.lhs.as_ref().is_some_and(|theirs| rules::same_pattern(theirs, &flipped)));
        if exists {
            continue;
        }
        let items = &list.items;
        let (a, b) = (items[items.len() - 2].span(), items[items.len() - 1].span());
        let mut lint = lint(info, TWIN, format!("heuristic: no rule matches `{op}` with its operands in the other order"));
        lint.fix = Some(Fix {
            title: "Add the flipped rule".to_owned(),
            span: Span { start: info.rule.form.end, end: info.rule.form.end },
            new_text: format!("\n{}", flipped_text(info.rule, a, b)),
        });
        found.push(lint);
    }
}

fn flipped_text(rule: &Rule, a: Span, b: Span) -> String {
    let text = rule.text;
    let mut parts = vec![(a, b.text(text).to_owned()), (b, a.text(text).to_owned())];
    match rule.priority_span {
        Some(span) => parts.push((span, (rule.priority + 1).to_string())),
        None => {
            let after = rule.name.or(rule.list.head().map(|head| head.span)).map_or(rule.form.start, |span| span.end);
            parts.push((Span { start: after, end: after }, " 1".to_owned()));
        }
    }
    if let Some(name) = rule.name {
        parts.push((name, format!("{}_flipped", name.text(text))));
    }
    parts.sort_by_key(|(span, _)| span.start);
    let mut out = String::new();
    let mut at = rule.form.start;
    for (span, replacement) in parts {
        out.push_str(&text[at..span.start]);
        out.push_str(&replacement);
        at = span.end;
    }
    out.push_str(&text[at..rule.form.end]);
    out
}
