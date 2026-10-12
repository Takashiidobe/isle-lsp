use std::panic::{self, AssertUnwindSafe};
use std::slice::from_ref;
use std::path::PathBuf;

use cranelift_isle::error::{Error, Span as IsleSpan};
use cranelift_isle::lexer::Lexer;
use cranelift_isle::{overlap, parser as isle_parser, recursion, sema};
use tower_lsp_server::ls_types::{
    Diagnostic, DiagnosticRelatedInformation, DiagnosticSeverity, DiagnosticTag, Location, NumberOrString, Range, Uri,
};

use crate::document::position_in;
use crate::lints;
use crate::project::{Loaded, Project, Source};
use crate::index::{DefKind, Index};
use crate::rules;
use crate::syntax::{AtomKind, List, Node, Span};

const SOURCE: &str = "isle-lsp";

pub struct FileDiagnostics {
    pub path: PathBuf,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn all(project: &Project, loaded: &Loaded) -> Vec<FileDiagnostics> {
    let mut found: Vec<Vec<Diagnostic>> = vec![Vec::new(); loaded.sources.len()];

    for (idx, source) in loaded.sources.iter().enumerate() {
        for error in &source.file.errors {
            found[idx].push(diagnostic(source, error.span, DiagnosticSeverity::ERROR, "syntax", &error.message));
        }
    }

    if found.iter().all(Vec::is_empty) {
        match skip_reason(project, loaded) {
            Some(reason) => {
                for (idx, source) in loaded.sources.iter().enumerate() {
                    found[idx].push(diagnostic(
                        source,
                        Span { start: 0, end: 0 },
                        DiagnosticSeverity::WARNING,
                        "incomplete-project",
                        &reason,
                    ));
                }
            }
            None => {
                for error in semantic(loaded) {
                    place(loaded, &mut found, error);
                }
                unused_decls(loaded, &mut found);
                rule_lints(loaded, &project.disabled_lints, &mut found);
            }
        }
    }

    loaded
        .sources
        .iter()
        .zip(found)
        .map(|(source, diagnostics)| FileDiagnostics { path: source.path.clone(), diagnostics })
        .collect()
}

fn unused_decls(loaded: &Loaded, found: &mut [Vec<Diagnostic>]) {
    let index = Index::build(loaded);
    for (id, def) in index.defs.iter().enumerate() {
        if def.kind != DefKind::Term || index.references(id).next().is_some() {
            continue;
        }
        let source = &loaded.sources[def.file];
        let mut diag = diagnostic(
            source,
            def.name_span,
            DiagnosticSeverity::HINT,
            "unused",
            &format!("`{}` is declared but never used", def.name),
        );
        diag.tags = Some(vec![DiagnosticTag::UNNECESSARY]);
        found[def.file].push(diag);
    }
}

fn rule_lints(loaded: &Loaded, disabled: &[String], found: &mut [Vec<Diagnostic>]) {
    for lint in lints::run(loaded, disabled) {
        let source = &loaded.sources[lint.file];
        let mut diag = diagnostic(source, lint.span, DiagnosticSeverity::WARNING, lint.code, &lint.message);
        let related: Vec<DiagnosticRelatedInformation> = lint
            .related
            .iter()
            .filter_map(|&(file, span)| {
                let other = &loaded.sources[file];
                let uri = Uri::from_file_path(&other.path)?;
                Some(DiagnosticRelatedInformation {
                    location: Location::new(uri, range_of(other, span)),
                    message: "related rule".to_owned(),
                })
            })
            .collect();
        if !related.is_empty() {
            diag.related_information = Some(related);
        }
        if let Some(fix) = lint.fix {
            let edits: Vec<serde_json::Value> = fix
                .edits
                .iter()
                .filter_map(|edit| {
                    let target = &loaded.sources[edit.file];
                    let uri = Uri::from_file_path(&target.path)?;
                    Some(serde_json::json!({ "uri": uri, "range": range_of(target, edit.span), "newText": edit.new_text }))
                })
                .collect();
            diag.data = Some(serde_json::json!({ "fixes": [{ "title": fix.title, "edits": edits }] }));
        }
        found[lint.file].push(diag);
    }
}

fn skip_reason(project: &Project, loaded: &Loaded) -> Option<String> {
    if let Some(missing) = loaded.missing.first() {
        return Some(format!("semantic checks skipped: project file {} was not found", missing.display()));
    }
    project.incomplete.then(|| format!("semantic checks skipped: {}", project.notes.join("; ")))
}

fn semantic(loaded: &Loaded) -> Vec<Error> {
    match panic::catch_unwind(AssertUnwindSafe(|| run_isle(loaded))) {
        Ok(errors) => errors,
        Err(_) => vec![Error::TypeError {
            msg: "internal error in cranelift-isle while checking this project".to_owned(),
            span: IsleSpan::new_single(cranelift_isle::lexer::Pos::new(0, 0)),
        }],
    }
}

fn run_isle(loaded: &Loaded) -> Vec<Error> {
    let mut defs = Vec::new();
    let mut errors = Vec::new();
    for (idx, source) in loaded.sources.iter().enumerate() {
        match Lexer::new(idx, &source.text).and_then(isle_parser::parse) {
            Ok(mut parsed) => defs.append(&mut parsed),
            Err(error) => errors.push(error),
        }
    }
    if !errors.is_empty() {
        return errors;
    }
    let mut type_env = match sema::TypeEnv::from_ast(&defs) {
        Ok(env) => env,
        Err(errors) => return errors,
    };
    let term_env = match sema::TermEnv::from_ast(&mut type_env, &defs, true) {
        Ok(env) => env,
        Err(errors) => return errors,
    };
    let terms = match overlap::check(&term_env) {
        Ok(terms) => terms,
        Err(errors) => return errors,
    };
    recursion::check(&terms, &term_env).err().unwrap_or_default()
}

fn place(loaded: &Loaded, found: &mut [Vec<Diagnostic>], error: Error) {
    let (code, message, primary, others, related_message) = match error {
        Error::IoError { context, .. } => {
            let source = &loaded.sources[0];
            found[0].push(diagnostic(source, Span { start: 0, end: 0 }, DiagnosticSeverity::ERROR, "io", &context));
            return;
        }
        Error::ParseError { msg, span } => ("parse", msg, span, Vec::new(), ""),
        Error::TypeError { msg, span } => ("type", msg, span, Vec::new(), ""),
        Error::UnreachableError { msg, span } => ("unreachable", msg, span, Vec::new(), ""),
        Error::RecursionError { msg, span } => ("recursion", msg, span, Vec::new(), ""),
        Error::OverlapError { rules, .. } => {
            let on = common_input(loaded, &rules).map_or_else(String::new, |pattern| format!(" on `{pattern}`"));
            for (idx, rule) in rules.iter().enumerate() {
                let others: Vec<IsleSpan> =
                    rules.iter().enumerate().filter(|(other, _)| *other != idx).map(|(_, span)| span.clone()).collect();
                let fix = loaded.sources.get(rule.from.file).and_then(|source| priority_fix(source, rule));
                let message = match &fix {
                    Some(fix) => format!(
                        "rules are overlapping{on}: give one of them a higher priority, for example `(rule {} ...)`",
                        fix.priority
                    ),
                    None => format!("rules are overlapping{on}: give one of them a higher priority"),
                };
                let before = found[rule.from.file].len();
                push_isle(loaded, found, "overlap", message, rule.clone(), &others, "overlaps with this rule");
                if let (Some(fix), Some(diag)) = (fix, found[rule.from.file].get_mut(before)) {
                    diag.data = Some(serde_json::json!({
                        "fixes": [{ "title": format!("Give this rule priority {}", fix.priority), "range": fix.range, "newText": fix.new_text }]
                    }));
                }
            }
            return;
        }
        Error::ShadowedError { shadowed, mask } => {
            for span in shadowed {
                let message = "this rule can never match: a higher-priority, more general rule always matches first";
                push_isle(loaded, found, "shadowed", message.to_owned(), span, from_ref(&mask), "shadowed by this rule");
            }
            return;
        }
    };
    let file = primary.from.file;
    let fixes = (code == "type").then(|| unknown_term_fixes(loaded, &message, primary.from.offset, file)).flatten();
    let before = found.get(file).map_or(0, Vec::len);
    push_isle(loaded, found, code, message, primary, &others, related_message);
    if let (Some(fixes), Some(diag)) = (fixes, found.get_mut(file).and_then(|diags| diags.get_mut(before))) {
        diag.data = Some(serde_json::json!({ "fixes": fixes }));
    }
}

fn common_input(loaded: &Loaded, spans: &[IsleSpan]) -> Option<String> {
    let all = rules::rules(loaded);
    let found: Vec<&rules::Rule> = spans
        .iter()
        .map(|span| {
            let source = loaded.sources.get(span.from.file)?;
            let form = enclosing_form(source, Span { start: span.from.offset, end: span.to.offset });
            all.iter().find(|rule| rule.file == span.from.file && rule.form == form)
        })
        .collect::<Option<_>>()?;
    let (first, rest) = found.split_first()?;
    let mut common = rules::shapes(first.text, first.args);
    for rule in rest {
        let theirs = rules::shapes(rule.text, rule.args);
        if rule.head != first.head || theirs.len() != common.len() {
            return None;
        }
        common = common.iter().zip(&theirs).map(|(a, b)| rules::meet(a, b)).collect::<Option<_>>()?;
    }
    Some(rules::render(&rules::Shape::Call(first.head.to_owned(), common)))
}

fn unknown_term_fixes(loaded: &Loaded, message: &str, offset: usize, file: usize) -> Option<Vec<serde_json::Value>> {
    let rest = message.strip_prefix("Unknown term in pattern: '").or_else(|| message.strip_prefix("Unknown term in expression: '"))?;
    let name = rest.split('\'').next()?;
    let source = loaded.sources.get(file)?;
    let node = node_starting_at(&source.file.items, offset)?;
    let list = source.file.items.iter().find_map(|item| find_list(item, node));
    let head = match list {
        Some(list) => list.head()?.span,
        None => node,
    };
    let index = Index::build(loaded);
    let mut fixes = Vec::new();
    if let Some(closest) = closest_term(&index, name) {
        fixes.push(serde_json::json!({
            "title": format!("Replace with `{closest}`"),
            "range": range_of(source, head),
            "newText": closest,
        }));
    }
    let form = enclosing_form(source, node);
    let arity = list.map_or(0, |list| list.items.len().saturating_sub(1));
    let ty = index.defs.iter().find(|def| def.kind == DefKind::Type).map_or("T", |def| def.name.as_str());
    let args = vec![ty; arity].join(" ");
    fixes.push(serde_json::json!({
        "title": format!("Declare term `{name}`"),
        "range": range_of(source, Span { start: form.start, end: form.start }),
        "newText": format!("(decl {name} ({args}) {ty})\n\n"),
    }));
    Some(fixes)
}

fn find_list(node: &Node, span: Span) -> Option<&List> {
    let Node::List(list) = node else { return None };
    if list.span == span {
        return Some(list);
    }
    if list.span.start <= span.start && span.end <= list.span.end {
        return list.items.iter().find_map(|item| find_list(item, span));
    }
    None
}

fn closest_term<'a>(index: &'a Index, name: &str) -> Option<&'a str> {
    let limit = (name.len() / 3).max(2);
    index
        .defs
        .iter()
        .filter(|def| matches!(def.kind, DefKind::Term | DefKind::Variant | DefKind::Macro))
        .map(|def| (edit_distance(name, &def.name), def.name.as_str()))
        .filter(|&(distance, candidate)| distance > 0 && distance <= limit && candidate != name)
        .min_by_key(|&(distance, _)| distance)
        .map(|(_, candidate)| candidate)
}

fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (above + 1).min(row[j] + 1).min(diagonal + usize::from(ca != cb));
            diagonal = above;
        }
    }
    row[b.len()]
}

fn push_isle(
    loaded: &Loaded,
    found: &mut [Vec<Diagnostic>],
    code: &str,
    message: String,
    primary: IsleSpan,
    others: &[IsleSpan],
    related_message: &str,
) {
    let Some(source) = loaded.sources.get(primary.from.file) else { return };
    let widen = matches!(code, "overlap" | "shadowed" | "unreachable");
    let span_in = |source: &Source, span: &IsleSpan| {
        let span = Span { start: span.from.offset, end: span.to.offset };
        if widen {
            enclosing_form(source, span)
        } else if span.end - span.start <= 1 {
            node_starting_at(&source.file.items, span.start).unwrap_or(span)
        } else {
            span
        }
    };
    let mut diag = diagnostic(source, span_in(source, &primary), DiagnosticSeverity::ERROR, code, &message);
    let related: Vec<DiagnosticRelatedInformation> = others
        .iter()
        .filter_map(|other| {
            let other_source = loaded.sources.get(other.from.file)?;
            let uri = Uri::from_file_path(&other_source.path)?;
            let range = range_of(other_source, span_in(other_source, other));
            Some(DiagnosticRelatedInformation { location: Location::new(uri, range), message: related_message.to_owned() })
        })
        .collect();
    if !related.is_empty() {
        diag.related_information = Some(related);
    }
    found[primary.from.file].push(diag);
}

struct PriorityFix {
    priority: i64,
    range: Range,
    new_text: String,
}

fn priority_fix(source: &Source, rule: &IsleSpan) -> Option<PriorityFix> {
    let form = enclosing_form(source, Span { start: rule.from.offset, end: rule.to.offset });
    let Node::List(list) = source.file.items.iter().find(|node| node.span() == form)? else { return None };
    let head = list.head()?;
    if head.span.text(&source.text) != "rule" {
        return None;
    }
    match list.items.get(1) {
        Some(Node::Atom(atom)) if atom.kind == AtomKind::Int => {
            let current: i64 = atom.span.text(&source.text).replace('_', "").parse().ok()?;
            let priority = current + 1;
            Some(PriorityFix { priority, range: range_of(source, atom.span), new_text: priority.to_string() })
        }
        _ => {
            let at = Span { start: head.span.end, end: head.span.end };
            Some(PriorityFix { priority: 1, range: range_of(source, at), new_text: " 1".to_owned() })
        }
    }
}

fn enclosing_form(source: &Source, span: Span) -> Span {
    source
        .file
        .items
        .iter()
        .map(|node| node.span())
        .find(|form| form.start <= span.start && span.start < form.end)
        .unwrap_or(span)
}

fn node_starting_at(nodes: &[Node], offset: usize) -> Option<Span> {
    for node in nodes {
        let span = node.span();
        if span.start == offset {
            return Some(span);
        }
        if let Node::List(list) = node
            && span.start < offset
            && offset < span.end
        {
            return node_starting_at(&list.items, offset);
        }
    }
    None
}

fn range_of(source: &Source, span: Span) -> Range {
    let len = source.text.len();
    let start = source.text.floor_char_boundary(span.start.min(len));
    let mut end = span.end.max(start).min(len);
    while !source.text.is_char_boundary(end) {
        end += 1;
    }
    Range::new(position_in(&source.text, start), position_in(&source.text, end))
}

fn diagnostic(source: &Source, span: Span, severity: DiagnosticSeverity, code: &str, message: &str) -> Diagnostic {
    Diagnostic {
        range: range_of(source, span),
        severity: Some(severity),
        code: Some(NumberOrString::String(code.to_owned())),
        source: Some(SOURCE.to_owned()),
        message: message.to_owned(),
        ..Diagnostic::default()
    }
}
