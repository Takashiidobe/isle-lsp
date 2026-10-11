use std::collections::HashMap;

use crate::index::{DefId, DefKind, Index, Role};
use crate::project::Loaded;
use crate::syntax::{AtomKind, FormKind, Node, Span, form_kind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SymKind {
    Type,
    Variant,
    Term,
    Const,
    Macro,
    Rule,
    Group,
}

impl SymKind {
    pub fn label(self) -> &'static str {
        match self {
            SymKind::Type => "type",
            SymKind::Variant => "variant",
            SymKind::Term => "term",
            SymKind::Const => "const",
            SymKind::Macro => "macro",
            SymKind::Rule => "rule",
            SymKind::Group => "rules",
        }
    }
}

#[derive(Debug)]
pub struct Symbol {
    pub name: String,
    pub detail: String,
    pub kind: SymKind,
    pub range: Span,
    pub selection: Span,
    pub children: Vec<Symbol>,
}

#[derive(Debug)]
pub struct WorkspaceSymbol {
    pub name: String,
    pub kind: SymKind,
    pub file: usize,
    pub span: Span,
}

pub fn document_symbols(index: &Index, loaded: &Loaded, file: usize) -> Vec<Symbol> {
    let source = &loaded.sources[file];
    let inside = |outer: Span, inner: Span| outer.start <= inner.start && inner.end <= outer.end;
    let mut out: Vec<Symbol> = Vec::new();
    let mut term_symbols: HashMap<DefId, usize> = HashMap::new();
    let mut rules = Vec::new();

    for node in &source.file.items {
        let Node::List(list) = node else { continue };
        let Some(kind) = form_kind(&source.text, list) else { continue };
        let form = list.span;
        let defs_in = |wanted: DefKind| {
            index.defs.iter().enumerate().filter(move |(_, def)| def.file == file && def.kind == wanted && inside(form, def.name_span))
        };
        match kind {
            FormKind::Type => {
                let Some((_, def)) = defs_in(DefKind::Type).next() else { continue };
                let children = defs_in(DefKind::Variant)
                    .map(|(_, variant)| Symbol {
                        name: variant.name.clone(),
                        detail: variant.args.join(" "),
                        kind: SymKind::Variant,
                        range: variant.form,
                        selection: variant.name_span,
                        children: Vec::new(),
                    })
                    .collect();
                out.push(Symbol {
                    name: def.name.clone(),
                    detail: String::new(),
                    kind: SymKind::Type,
                    range: form,
                    selection: def.name_span,
                    children,
                });
            }
            FormKind::Decl => {
                let Some((id, def)) = defs_in(DefKind::Term).next() else { continue };
                let detail = format!("({}) -> {}", def.args.join(" "), def.ret.clone().unwrap_or_default());
                term_symbols.insert(id, out.len());
                out.push(Symbol {
                    name: def.name.clone(),
                    detail,
                    kind: SymKind::Term,
                    range: form,
                    selection: def.name_span,
                    children: Vec::new(),
                });
            }
            FormKind::Macro => {
                if let Some((_, def)) = defs_in(DefKind::Macro).next() {
                    out.push(Symbol {
                        name: def.name.clone(),
                        detail: String::new(),
                        kind: SymKind::Macro,
                        range: form,
                        selection: def.name_span,
                        children: Vec::new(),
                    });
                }
            }
            FormKind::Extern => {
                if let Some((_, def)) = defs_in(DefKind::Const).next() {
                    out.push(Symbol {
                        name: def.name.clone(),
                        detail: def.ty.clone().unwrap_or_default(),
                        kind: SymKind::Const,
                        range: form,
                        selection: def.name_span,
                        children: Vec::new(),
                    });
                }
            }
            FormKind::Rule => rules.push(list),
            _ => {}
        }
    }

    let mut groups: Vec<(String, Vec<Symbol>)> = Vec::new();
    for list in rules {
        let mut at = 1;
        let mut name = None;
        if let Some(Node::Atom(atom)) = list.items.get(at)
            && atom.kind == AtomKind::Symbol
        {
            name = Some(atom.span.text(&source.text).to_owned());
            at += 1;
        }
        let mut detail = String::new();
        if let Some(Node::Atom(atom)) = list.items.get(at)
            && atom.kind == AtomKind::Int
        {
            detail = format!("priority {}", atom.span.text(&source.text));
            at += 1;
        }
        let Some(lhs) = list.items.get(at) else { continue };
        let lhs_text = lhs.span().text(&source.text).split_whitespace().collect::<Vec<_>>().join(" ");
        let symbol = Symbol {
            name: name.unwrap_or(lhs_text),
            detail,
            kind: SymKind::Rule,
            range: list.span,
            selection: lhs.span(),
            children: Vec::new(),
        };
        let head = index.refs.iter().find(|r| r.file == file && r.role == Role::RuleHead && lhs.span().start <= r.span.start && r.span.end <= lhs.span().end);
        let Some(head) = head else {
            out.push(symbol);
            continue;
        };
        match head.def.and_then(|def| term_symbols.get(&def)) {
            Some(&slot) => out[slot].children.push(symbol),
            None => {
                let head_name = head.span.text(&source.text).to_owned();
                match groups.iter_mut().find(|(name, _)| *name == head_name) {
                    Some((_, members)) => members.push(symbol),
                    None => groups.push((head_name, vec![symbol])),
                }
            }
        }
    }
    for (name, members) in groups {
        let range = Span {
            start: members.iter().map(|m| m.range.start).min().unwrap_or(0),
            end: members.iter().map(|m| m.range.end).max().unwrap_or(0),
        };
        let selection = members[0].selection;
        out.push(Symbol { name, detail: "rules".to_owned(), kind: SymKind::Group, range, selection, children: members });
    }
    out
}

pub fn workspace_symbols(index: &Index, query: &str) -> Vec<WorkspaceSymbol> {
    let query = query.to_lowercase();
    index
        .defs
        .iter()
        .filter_map(|def| {
            let kind = match def.kind {
                DefKind::Type => SymKind::Type,
                DefKind::Variant => SymKind::Variant,
                DefKind::Term => SymKind::Term,
                DefKind::Const => SymKind::Const,
                DefKind::Macro => SymKind::Macro,
                DefKind::Rule => SymKind::Rule,
                DefKind::Variable => return None,
            };
            subsequence(&query, &def.name.to_lowercase())
                .then(|| WorkspaceSymbol { name: def.name.clone(), kind, file: def.file, span: def.name_span })
        })
        .collect()
}

fn subsequence(needle: &str, haystack: &str) -> bool {
    let mut rest = haystack.chars();
    needle.chars().all(|wanted| rest.any(|c| c == wanted))
}

pub fn folding(loaded: &Loaded, file: usize) -> Vec<Span> {
    let source = &loaded.sources[file];
    let multiline = |span: &Span| source.text[span.start..span.end].contains('\n');
    let mut found: Vec<Span> = source.file.items.iter().map(Node::span).filter(multiline).collect();
    found.extend(source.file.comments.iter().copied().filter(multiline));
    found.sort_by_key(|span| span.start);
    found
}
