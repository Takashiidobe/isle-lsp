use std::path::{Path, PathBuf};

use tower_lsp_server::ls_types::{DiagnosticSeverity, NumberOrString, Position, Range, Uri};

use crate::actions;
use crate::completion;
use crate::diagnostics;
use crate::document::Document;
use crate::filetest;
use crate::hierarchy;
use crate::hover;
use crate::index;
use crate::inlay;
use crate::lexer::{self, TokenKind};
use crate::lints;
use crate::navigation;
use crate::parser;
use crate::project;
use crate::refactor;
use crate::rules;
use crate::semantic;
use crate::symbols;
use crate::syntax::{self, AtomKind, FormKind, Node, Span};
use crate::view;

pub struct Query {
    pub line: usize,
    pub col: usize,
    pub kind: String,
    pub args: String,
}

pub struct Input<'a> {
    pub source: &'a str,
    pub queries: &'a [Query],
    pub path: &'a Path,
    pub root: &'a Path,
}

pub fn render(mode: &str, input: &Input) -> Result<String, String> {
    match mode {
        "project" => Ok(render_project(input)),
        "diagnostics" => Ok(render_diagnostics(input)),
        "resolve" => Ok(render_resolve(input)),
        "navigate" => Ok(render_navigate(input)),
        "document" => render_document(input),
        "tokens" => Ok(render_tokens(&plain_text(input.source))),
        "syntax-tree" => Ok(render_syntax_tree(&plain_text(input.source))),
        "source" => Ok(plain_text(input.source)),
        _ => Err(format!("unknown mode `{mode}`")),
    }
}

fn load_project(input: &Input) -> (project::Project, project::Loaded) {
    let project = project::discover(input.path, Some(input.root));
    let overrides: Vec<(PathBuf, String)> = input
        .queries
        .iter()
        .filter(|q| q.kind == "open")
        .map(|q| {
            let (name, text) = q.args.split_once(' ').unwrap_or((q.args.as_str(), ""));
            (project::normalize(&input.root.join(name)), text.replace("\\n", "\n"))
        })
        .collect();
    let loaded = project.load(|path| overrides.iter().find(|(p, _)| p == path).map(|(_, text)| text.clone()));
    (project, loaded)
}

fn shown(root: &Path, path: &Path) -> String {
    if let Ok(relative) = path.strip_prefix(root) {
        return relative.display().to_string();
    }
    match path.strip_prefix(project::bundled_dir()) {
        Ok(relative) => format!("<bundled>/{}", relative.display()),
        Err(_) => path.display().to_string(),
    }
}

fn render_diagnostics(input: &Input) -> String {
    let relative = |path: &Path| shown(input.root, path);
    let (project, loaded) = load_project(input);
    let text_of = |path: &Path| loaded.sources.iter().find(|s| s.path == path).map(|s| s.text.as_str()).unwrap_or("");
    let range = |path: &Path, range: Range| {
        let text = text_of(path);
        let (start, end) = (plain_position(text, range.start), plain_position(text, range.end));
        format!("{}:{}..{}:{}", start.0, start.1, end.0, end.1)
    };
    let root_prefix = format!("{}/", input.root.display());
    let mut out = String::new();
    for file in diagnostics::all(&project, &loaded) {
        for diagnostic in &file.diagnostics {
            let severity = match diagnostic.severity {
                Some(DiagnosticSeverity::ERROR) => "error",
                Some(DiagnosticSeverity::WARNING) => "warning",
                Some(DiagnosticSeverity::INFORMATION) => "info",
                _ => "hint",
            };
            let code = match &diagnostic.code {
                Some(NumberOrString::String(code)) => code.as_str(),
                _ => "",
            };
            out.push_str(&format!(
                "{} {} {severity}[{code}] {}\n",
                relative(&file.path),
                range(&file.path, diagnostic.range),
                diagnostic.message.replace(&root_prefix, "")
            ));
            if let Some(uri) = Uri::from_file_path(&file.path) {
                for action in actions::quick_fixes(&uri, std::slice::from_ref(diagnostic)) {
                    let mut targets: Vec<_> = action.edit.iter().filter_map(|edit| edit.changes.as_ref()).flatten().collect();
                    targets.sort_by_key(|(target, _)| target.to_string());
                    for (target, edits) in targets {
                        let path = target.to_file_path().map(|p| p.into_owned()).unwrap_or_default();
                        let elsewhere = if path == file.path { String::new() } else { format!("{} ", relative(&path)) };
                        for edit in edits {
                            out.push_str(&format!(
                                "  fix {} {elsewhere}{} => {:?}\n",
                                action.title,
                                range(&path, edit.range),
                                edit.new_text
                            ));
                        }
                    }
                }
            }
            for related in diagnostic.related_information.iter().flatten() {
                let path = related.location.uri.to_file_path().map(|p| p.into_owned()).unwrap_or_default();
                out.push_str(&format!(
                    "  related {} {} {}\n",
                    relative(&path),
                    range(&path, related.location.range),
                    related.message
                ));
            }
        }
    }
    if out.is_empty() {
        out.push_str("no diagnostics\n");
    }
    out
}

fn locate(input: &Input, loaded: &project::Loaded, file: usize, offset: usize) -> String {
    let source = &loaded.sources[file];
    let line = source.text[..offset].matches('\n').count();
    let skipped = source.text.lines().take(line).filter(|l| is_directive(l) || is_caret(l)).count();
    let column = offset - source.text[..offset].rfind('\n').map_or(0, |newline| newline + 1);
    let path = shown(input.root, &source.path);
    format!("{path} {}:{column}", line - skipped)
}

fn locate_span(input: &Input, loaded: &project::Loaded, place: navigation::Place) -> String {
    let start = locate(input, loaded, place.file, place.span.start);
    let end = locate(input, loaded, place.file, place.span.end);
    let (_, end) = end.split_once(' ').unwrap_or_default();
    format!("{start}..{end}")
}

fn render_navigate(input: &Input) -> String {
    let (_, loaded) = load_project(input);
    let index = index::Index::build(&loaded);
    let here = project::normalize(input.path);
    let Some(file) = loaded.sources.iter().position(|source| source.path == here) else {
        return "fixture file is not part of its project\n".to_owned();
    };
    let mut line_starts = vec![0];
    line_starts.extend(input.source.match_indices('\n').map(|(i, _)| i + 1));
    let mut out = String::new();
    for query in input.queries.iter().filter(|q| q.kind != "open") {
        let offset = line_starts[query.line] + query.col;
        let skipped = input.source.lines().take(query.line).filter(|l| is_directive(l) || is_caret(l)).count();
        out.push_str(&format!("{} {}:{}", query.kind, query.line - skipped, query.col));
        if !query.args.is_empty() {
            out.push_str(&format!(" {}", query.args));
        }
        out.push('\n');
        let places = |places: Vec<navigation::Place>, out: &mut String| {
            if places.is_empty() {
                out.push_str("  none\n");
            }
            for place in places {
                out.push_str(&format!("  {}\n", locate_span(input, &loaded, place)));
            }
        };
        match query.kind.as_str() {
            "definition" => places(navigation::definition(&index, file, offset).into_iter().collect(), &mut out),
            "references" => places(navigation::references(&index, file, offset, true), &mut out),
            "uses" => places(navigation::references(&index, file, offset, false), &mut out),
            "highlight" => places(navigation::highlights(&index, file, offset), &mut out),
            "refactor" => {
                let end = query.args.trim().parse::<usize>().map_or(offset, |col| line_starts[query.line] + col);
                let found = refactor::all(&loaded, &index, file, offset, end);
                if found.is_empty() {
                    out.push_str("  none\n");
                }
                for refactoring in found {
                    out.push_str(&format!("  {}\n", refactoring.title));
                    let mut files: Vec<usize> = refactoring.edits.iter().map(|edit| edit.place.file).collect();
                    files.dedup();
                    for changed in files {
                        let mut text = loaded.sources[changed].text.clone();
                        let mut edits: Vec<_> = refactoring.edits.iter().filter(|edit| edit.place.file == changed).collect();
                        edits.sort_by_key(|edit| std::cmp::Reverse(edit.place.span.start));
                        for edit in edits {
                            text.replace_range(edit.place.span.start..edit.place.span.end, &edit.new_text);
                        }
                        out.push_str(&format!("  file {}\n", shown(input.root, &loaded.sources[changed].path)));
                        let before: Vec<&str> = loaded.sources[changed].text.lines().collect();
                        let after: Vec<&str> = text.lines().collect();
                        let head = before.iter().zip(&after).take_while(|(a, b)| a == b).count();
                        let tail = before.iter().rev().zip(after.iter().rev()).take(before.len().min(after.len()) - head).take_while(|(a, b)| a == b).count();
                        after[head..after.len() - tail].iter().for_each(|line| out.push_str(&format!("  | {line}\n")));
                    }
                }
            }
            "incoming" | "outgoing" => {
                let Some(def) = hierarchy::prepare(&index, file, offset) else {
                    out.push_str("  none\n");
                    continue;
                };
                let calls = if query.kind == "incoming" {
                    hierarchy::incoming(&index, &loaded, def)
                } else {
                    hierarchy::outgoing(&index, &loaded, def)
                };
                out.push_str(&format!("  {} {}\n", index.defs[def].kind.label(), index.defs[def].name));
                if calls.is_empty() {
                    out.push_str("  none\n");
                }
                for call in calls {
                    let other = &index.defs[call.def];
                    let at = navigation::Place { file: other.file, span: other.name_span };
                    out.push_str(&format!("  {} {} {}\n", other.kind.label(), other.name, locate_span(input, &loaded, at)));
                    for site in call.sites {
                        out.push_str(&format!("    call {}\n", locate_span(input, &loaded, site)));
                    }
                }
            }
            "prepare" => places(navigation::prepare_rename(&index, &loaded, file, offset).into_iter().collect(), &mut out),
            "outline" => {
                fn show(input: &Input, loaded: &project::Loaded, file: usize, symbols: &[symbols::Symbol], depth: usize, out: &mut String) {
                    for symbol in symbols {
                        let range = locate_span(input, loaded, navigation::Place { file, span: symbol.range });
                        let detail = if symbol.detail.is_empty() { String::new() } else { format!(" [{}]", symbol.detail) };
                        out.push_str(&format!("  {}{} {}{detail} {range}\n", "  ".repeat(depth), symbol.kind.label(), symbol.name));
                        show(input, loaded, file, &symbol.children, depth + 1, out);
                    }
                }
                show(input, &loaded, file, &symbols::document_symbols(&index, &loaded, file), 0, &mut out);
            }
            "symbols" => {
                let found = symbols::workspace_symbols(&index, query.args.trim());
                if found.is_empty() {
                    out.push_str("  none\n");
                }
                for symbol in found {
                    let place = navigation::Place { file: symbol.file, span: symbol.span };
                    out.push_str(&format!("  {} {} {}\n", symbol.kind.label(), symbol.name, locate_span(input, &loaded, place)));
                }
            }
            "folding" => {
                for span in symbols::folding(&loaded, file) {
                    out.push_str(&format!("  {}\n", locate_span(input, &loaded, navigation::Place { file, span })));
                }
            }
            "semantic" => {
                for token in semantic::tokens(&index, &loaded, file) {
                    let place = navigation::Place { file, span: token.span };
                    let modifiers: Vec<&str> = semantic::MODIFIERS
                        .iter()
                        .enumerate()
                        .filter(|(bit, _)| token.modifiers & (1 << bit) != 0)
                        .map(|(_, name)| *name)
                        .collect();
                    let text = token.span.text(&loaded.sources[file].text);
                    out.push_str(&format!(
                        "  {} {} {} {}\n",
                        locate_span(input, &loaded, place),
                        semantic::TYPES[token.kind],
                        modifiers.join("+"),
                        text
                    ));
                }
            }
            "unused" => match actions::unused_decl(&index, &loaded, file, offset) {
                Some((title, place)) => out.push_str(&format!("  {title} {}\n", locate_span(input, &loaded, place))),
                None => out.push_str("  none\n"),
            },
            "filetest" => match filetest::generate(&loaded, file, offset) {
                Some(generated) => {
                    out.push_str(&format!("  {}\n", generated.file_name));
                    for line in generated.text.lines() {
                        out.push_str(&format!("  | {line}\n"));
                    }
                }
                None => out.push_str("  none\n"),
            },
            "twin" => match lints::twin_fix(&loaded, file, offset) {
                Some(fix) => {
                    for edit in &fix.edits {
                        let place = navigation::Place { file: edit.file, span: edit.span };
                        out.push_str(&format!("  {} {} => {:?}\n", fix.title, locate_span(input, &loaded, place), edit.new_text));
                    }
                }
                None => out.push_str("  none\n"),
            },
            "inlay" => {
                let hints = inlay::hints(&index, &loaded, file);
                if hints.is_empty() {
                    out.push_str("  none\n");
                }
                for hint in hints {
                    out.push_str(&format!("  {} {}\n", locate(input, &loaded, file, hint.offset), hint.label));
                }
            }
            "complete" => {
                let items = completion::complete(&index, &loaded, file, offset);
                if items.is_empty() {
                    out.push_str("  none\n");
                }
                for item in items {
                    let snippet = item.snippet.map(|s| format!(" snippet={s}")).unwrap_or_default();
                    out.push_str(&format!("  {} {:?} {}{snippet}\n", item.label, item.kind, item.detail));
                }
            }
            "signature" => match completion::signature(&index, &loaded, file, offset) {
                Some(signature) => {
                    let params: Vec<&str> = signature.params.iter().map(|&(s, e)| &signature.label[s..e]).collect();
                    out.push_str(&format!("  {} params={params:?} active={}\n", signature.label, signature.active));
                }
                None => out.push_str("  none\n"),
            },
            "hover" => match hover::hover(&index, &loaded, file, offset, |file, offset| {
                let locator = locate(input, &loaded, file, offset);
                locator.rsplit(' ').next().and_then(|at| at.split(':').next()?.parse().ok()).unwrap_or(0)
            }) {
                Some(hover) => {
                    let place = navigation::Place { file, span: hover.span };
                    out.push_str(&format!("  {}\n", locate_span(input, &loaded, place)));
                    for line in hover.markdown.lines() {
                        out.push_str(&format!("  | {line}\n"));
                    }
                }
                None => out.push_str("  none\n"),
            },
            "rename" => match navigation::rename(&index, &loaded, file, offset, query.args.trim()) {
                Ok(edits) => {
                    for edit in edits {
                        out.push_str(&format!("  {} => {:?}\n", locate_span(input, &loaded, edit.place), edit.new_text));
                    }
                }
                Err(message) => out.push_str(&format!("  error: {message}\n")),
            },
            "rules" => {
                let ranked = index
                    .resolve(file, offset)
                    .filter(|&id| index.defs[id].kind == index::DefKind::Term)
                    .map(|id| rules::for_term(&loaded, &index.defs[id].name))
                    .unwrap_or_default();
                ranked_rules(input, &loaded, &ranked, &mut out);
            }
            "view" | "page" => {
                let (name, explicit) = query.args.split_once(char::is_whitespace).unwrap_or((query.args.as_str(), ""));
                let under_cursor = index
                    .resolve(file, offset)
                    .filter(|&id| index.defs[id].kind == index::DefKind::Term)
                    .map(|id| index.defs[id].name.clone());
                let term = Some(explicit.trim().to_owned()).filter(|t| !t.is_empty()).or(under_cursor);
                match view::render(&loaded, |path| shown(input.root, path), name, term.as_deref()) {
                    Ok(rendered) => flatten_view(&rendered, query.kind == "page", &mut out),
                    Err(message) => out.push_str(&format!("  error: {message}\n")),
                }
            }
            "clif" => {
                let ranked = rules::for_clif(&loaded, &index, &query.args.replace("\\n", "\n"));
                ranked_rules(input, &loaded, &ranked, &mut out);
            }
            other => out.push_str(&format!("  unknown query `{other}`\n")),
        }
    }
    out
}

fn flatten_view(view: &view::View, page: bool, out: &mut String) {
    fn tree(nodes: &[view::TreeNode], depth: usize, out: &mut String) {
        for node in nodes {
            out.push_str(&format!("  {}{}\n", "  ".repeat(depth), node.label));
            tree(&node.children, depth + 1, out);
        }
    }
    out.push_str(&format!("  title: {}\n", view.title));
    for section in &view.sections {
        match section {
            view::Section::Text { title, text } => {
                out.push_str(&format!("  == {title} ==\n"));
                text.lines().for_each(|line| out.push_str(&format!("  | {line}\n")));
            }
            view::Section::Html { title, html } if page => {
                out.push_str(&format!("  == {title} ==\n"));
                html.lines().for_each(|line| out.push_str(&format!("  | {line}\n")));
            }
            view::Section::Html { title, .. } => out.push_str(&format!("  == {title} ==\n  (html page)\n")),
            view::Section::Table { title, columns, rows } => {
                out.push_str(&format!("  == {title} ==\n  {}\n", columns.join(" | ")));
                rows.iter().for_each(|row| out.push_str(&format!("  {}\n", row.join(" | "))));
            }
            view::Section::Tree { title, nodes } => {
                out.push_str(&format!("  == {title} ==\n"));
                tree(nodes, 0, out);
            }
        }
    }
}

fn ranked_rules(input: &Input, loaded: &project::Loaded, ranked: &[rules::Ranked], out: &mut String) {
    if ranked.is_empty() {
        out.push_str("  none\n");
    }
    for rule in ranked {
        let place = navigation::Place { file: rule.file, span: rule.span };
        out.push_str(&format!("  priority {} {}\n", rule.priority, locate_span(input, loaded, place)));
        for &(file, span) in &rule.wins_over {
            out.push_str(&format!("    wins over {}\n", locate_span(input, loaded, navigation::Place { file, span })));
        }
    }
}

fn render_resolve(input: &Input) -> String {
    let (_, loaded) = load_project(input);
    let index = index::Index::build(&loaded);
    let here = project::normalize(input.path);
    let Some(file) = loaded.sources.iter().position(|source| source.path == here) else {
        return "fixture file is not part of its project\n".to_owned();
    };
    let at = |file: usize, offset: usize| locate(input, &loaded, file, offset);
    let mut line_starts = vec![0];
    line_starts.extend(input.source.match_indices('\n').map(|(i, _)| i + 1));
    let mut out = String::new();
    for query in input.queries.iter().filter(|q| q.kind == "resolve") {
        let offset = line_starts[query.line] + query.col;
        let skipped = input.source.lines().take(query.line).filter(|l| is_directive(l) || is_caret(l)).count();
        let label = format!("resolve {}:{}", query.line - skipped, query.col);
        match index.resolve(file, offset) {
            None => out.push_str(&format!("{label} -> unresolved\n")),
            Some(id) => {
                let def = &index.defs[id];
                out.push_str(&format!("{label} -> {} {} @ {}\n", def.kind.label(), def.name, at(def.file, def.name_span.start)));
                for reference in index.references(id) {
                    out.push_str(&format!("  {} {}\n", reference.role.label(), at(reference.file, reference.span.start)));
                }
            }
        }
    }
    out
}

fn plain_position(text: &str, position: Position) -> (usize, u32) {
    let plain_line = text.lines().take(position.line as usize).filter(|l| !is_directive(l) && !is_caret(l)).count();
    (plain_line, position.character)
}

fn render_project(input: &Input) -> String {
    let relative = |path: &Path| shown(input.root, path);
    let (project, loaded) = load_project(input);

    let origin = match &project.origin {
        project::Origin::Config(path) => format!("config {}", relative(path)),
        project::Origin::CraneliftOpt => "cranelift-opt".to_owned(),
        project::Origin::CraneliftLower => "cranelift-lower".to_owned(),
        project::Origin::Siblings => "siblings".to_owned(),
        project::Origin::Single => "single".to_owned(),
    };
    let mut out = format!("origin: {origin}\n");
    if let Some(name) = &project.name {
        out.push_str(&format!("set: {name}\n"));
    }
    out.push_str("files:\n");
    for path in &project.files {
        out.push_str(&format!("  {}\n", relative(path)));
    }
    for path in &loaded.missing {
        out.push_str(&format!("missing: {}\n", relative(path)));
    }
    for note in &project.notes {
        out.push_str(&format!("note: {note}\n"));
    }
    out.push_str("decls:\n");
    for source in &loaded.sources {
        for node in &source.file.items {
            let Node::List(list) = node else { continue };
            if syntax::form_kind(&source.text, list) != Some(FormKind::Decl) {
                continue;
            }
            let name = list.items[1..].iter().find_map(|item| match item {
                Node::Atom(atom) if atom.kind == AtomKind::Symbol => {
                    Some(atom.span).filter(|s| !matches!(s.text(&source.text), "pure" | "multi" | "partial"))
                }
                _ => None,
            });
            if let Some(name) = name {
                let line = source.text[..name.start].lines().filter(|l| !is_directive(l) && !is_caret(l)).count();
                out.push_str(&format!("  {} ({}:{line})\n", name.text(&source.text), relative(&source.path)));
            }
        }
    }
    out
}

struct Positions {
    line_starts: Vec<usize>,
}

impl Positions {
    fn new(source: &str) -> Positions {
        let mut line_starts = vec![0];
        line_starts.extend(source.match_indices('\n').map(|(i, _)| i + 1));
        Positions { line_starts }
    }

    fn line(&self, byte: usize) -> usize {
        self.line_starts.partition_point(|&s| s <= byte) - 1
    }

    fn at(&self, byte: usize) -> String {
        let line = self.line(byte);
        format!("{line}:{}", byte - self.line_starts[line])
    }

    fn range(&self, span: Span) -> String {
        format!("{}..{}", self.at(span.start), self.at(span.end))
    }
}

fn render_tokens(source: &str) -> String {
    let lexed = lexer::lex(source);
    let pos = Positions::new(source);
    let mut out = String::new();
    for token in lexed.tokens.iter().filter(|t| !matches!(t.kind, TokenKind::Whitespace | TokenKind::Newline)) {
        out.push_str(&format!("{} {:?} {:?}\n", pos.at(token.start), token.kind, token.text(source)));
    }
    for error in &lexed.errors {
        out.push_str(&format!("error {} {}\n", pos.range(error.span), error.message));
    }
    out
}

fn render_syntax_tree(source: &str) -> String {
    let file = parser::parse(source);
    let pos = Positions::new(source);
    let mut out = String::new();
    for node in &file.items {
        if let Node::List(list) = node {
            let kind = syntax::form_kind(source, list).map_or("unknown", FormKind::keyword);
            let state = if list.closed { "closed" } else { "unclosed" };
            out.push_str(&format!("form {kind} @{} {state}\n", pos.range(list.span)));
            for item in &list.items {
                render_node(source, &pos, item, 1, &mut out);
            }
        } else {
            render_node(source, &pos, node, 0, &mut out);
        }
    }
    for error in &file.errors {
        out.push_str(&format!("error {} {}\n", pos.range(error.span), error.message));
    }
    out.push_str(&format!("comments {}\n", file.comments.len()));
    out
}

fn render_node(source: &str, pos: &Positions, node: &Node, depth: usize, out: &mut String) {
    let indent = "  ".repeat(depth);
    match node {
        Node::Atom(atom) => {
            let kind = match atom.kind {
                AtomKind::Symbol => "symbol",
                AtomKind::Int => "int",
                AtomKind::At => "at",
            };
            out.push_str(&format!("{indent}{kind} {} @{}\n", atom.span.text(source), pos.at(atom.span.start)));
        }
        Node::List(list) => {
            let state = if list.closed { "" } else { " unclosed" };
            out.push_str(&format!("{indent}list @{}{state}\n", pos.range(list.span)));
            for item in &list.items {
                render_node(source, pos, item, depth + 1, out);
            }
        }
    }
}

const DIRECTIVES: [&str; 8] = ["mode:", "lsp:", "check:", "sameln:", "nextln:", "unordered:", "not:", "regex:"];

fn is_directive(comment: &str) -> bool {
    let body = comment.trim_start().strip_prefix(';').map(str::trim_start);
    body.is_some_and(|c| DIRECTIVES.iter().any(|d| c.starts_with(d)))
}

fn is_caret(line: &str) -> bool {
    line.trim_start().strip_prefix(';').is_some_and(|rest| rest.trim_start().starts_with('^'))
}

pub fn plain_text(source: &str) -> String {
    source.lines().filter(|l| !is_directive(l) && !is_caret(l)).collect::<Vec<_>>().join("\n")
}

fn render_document(input: &Input) -> Result<String, String> {
    let mut out = String::new();
    let raw = Document::new(input.source);
    let mut line_starts = vec![0];
    line_starts.extend(input.source.match_indices('\n').map(|(i, _)| i + 1));
    let header = input.source.lines().take_while(|l| is_directive(l)).count();
    for query in input.queries.iter().filter(|q| q.kind == "at") {
        let offset = line_starts[query.line] + query.col;
        let lsp = raw.position(offset);
        let back = raw.offset(lsp) - line_starts[query.line];
        out.push_str(&format!(
            "at {}:{} -> utf16 {} -> byte {back}\n",
            query.line as isize - header as isize,
            query.col,
            lsp.character
        ));
    }

    let edits: Vec<&Query> = input.queries.iter().filter(|q| q.kind == "edit").collect();
    if !edits.is_empty() {
        let mut document = Document::new(&plain_text(input.source));
        for edit in edits {
            let (range, text) = parse_edit(&edit.args)?;
            document.apply(Some(range), &text);
            out.push_str(&format!("edit {}\n", edit.args));
        }
        out.push_str(&format!("text:\n{}\n", document.text()));
    }
    Ok(out)
}

pub fn parse_edit(args: &str) -> Result<(Range, String), String> {
    let bad = || format!("bad edit `{args}`, expected `L:C-L:C text`");
    let (range, text) = args.split_once(' ').unwrap_or((args, ""));
    let (start, end) = range.split_once('-').ok_or_else(bad)?;
    let position = |s: &str| -> Result<Position, String> {
        let (line, col) = s.split_once(':').ok_or_else(bad)?;
        Ok(Position::new(line.parse().map_err(|_| bad())?, col.parse().map_err(|_| bad())?))
    };
    Ok((Range::new(position(start)?, position(end)?), text.replace("\\n", "\n")))
}
