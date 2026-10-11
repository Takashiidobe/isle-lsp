use std::panic::{self, AssertUnwindSafe};
use std::slice::from_ref;
use std::path::PathBuf;

use cranelift_isle::error::{Error, Span as IsleSpan};
use cranelift_isle::lexer::Lexer;
use cranelift_isle::{overlap, parser as isle_parser, recursion, sema};
use tower_lsp_server::ls_types::{
    Diagnostic, DiagnosticRelatedInformation, DiagnosticSeverity, Location, NumberOrString, Range, Uri,
};

use crate::document::position_in;
use crate::project::{Loaded, Project, Source};
use crate::syntax::{Node, Span};

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
        Error::OverlapError { msg, rules } => {
            let (first, rest) = rules.split_first().expect("overlap errors name at least one rule");
            ("overlap", msg, first.clone(), rest.to_vec(), "overlaps with this rule")
        }
        Error::ShadowedError { shadowed, mask } => {
            for span in shadowed {
                let message = "this rule can never match: a higher-priority, more general rule always matches first";
                push_isle(loaded, found, "shadowed", message.to_owned(), span, from_ref(&mask), "shadowed by this rule");
            }
            return;
        }
    };
    push_isle(loaded, found, code, message, primary, &others, related_message);
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
