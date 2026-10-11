use crate::index::{Def, DefKind, Index, Role};
use crate::project::Loaded;
use crate::syntax::{AtomKind, FORMS, FormKind, List, Node, Span, form_kind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Keyword,
    Term,
    Type,
    Variant,
    Variable,
    Const,
}

#[derive(Debug)]
pub struct Item {
    pub label: String,
    pub kind: ItemKind,
    pub detail: String,
    pub snippet: Option<String>,
}

#[derive(Debug)]
pub struct Signature {
    pub label: String,
    pub params: Vec<(usize, usize)>,
    pub active: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Lhs,
    Pattern,
    Expr,
    Clause,
    Bindings,
    Binding,
    Types,
    Other,
}

struct Cursor<'a> {
    text: &'a str,
    path: Vec<&'a List>,
    offset: usize,
}

pub fn complete(index: &Index, loaded: &Loaded, file: usize, offset: usize) -> Vec<Item> {
    let source = &loaded.sources[file];
    let mut path = Vec::new();
    descend(&source.file.items, offset, source.text.len(), &mut path);
    let cursor = Cursor { text: &source.text, path, offset };
    let Some(&list) = cursor.path.last() else { return Vec::new() };
    let slot = cursor.slot(list);

    if cursor.path.len() == 1 {
        return match form_kind(cursor.text, list) {
            None if slot == 0 => keywords(),
            Some(FormKind::Rule) => {
                let lhs = rule_lhs_index(list);
                if slot > lhs { values(index, &cursor, file, None) } else { Vec::new() }
            }
            Some(FormKind::Decl) if slot > 1 => types(index),
            Some(FormKind::Extern) if cursor.head_is(list, "extern") && slot == 3 => types(index),
            _ => Vec::new(),
        };
    }

    match cursor.mode(cursor.path.len() - 1) {
        Mode::Lhs if slot == 0 => terms(index, None, Filter::Any),
        Mode::Pattern if slot == 0 => terms(index, cursor.expected(index, cursor.path.len() - 1).as_deref(), Filter::Extractor),
        Mode::Expr if slot == 0 => {
            if cursor.head_is(list, "let") {
                return Vec::new();
            }
            let mut items = terms(index, cursor.expected(index, cursor.path.len() - 1).as_deref(), Filter::Constructor);
            items.push(keyword("let"));
            items
        }
        Mode::Expr => {
            let expected = cursor.argument_type(index, list, slot);
            values(index, &cursor, file, expected.as_deref())
        }
        Mode::Clause if slot == 0 => vec![keyword("if-let"), keyword("if")],
        Mode::Binding if slot == 1 => types(index),
        Mode::Binding if slot == 2 => values(index, &cursor, file, None),
        Mode::Types => types(index),
        _ => Vec::new(),
    }
}

pub fn signature(index: &Index, loaded: &Loaded, file: usize, offset: usize) -> Option<Signature> {
    let source = &loaded.sources[file];
    let mut path = Vec::new();
    descend(&source.file.items, offset, source.text.len(), &mut path);
    let cursor = Cursor { text: &source.text, path, offset };
    let (list, head) = cursor.path.iter().enumerate().rev().find_map(|(depth, list)| {
        let head = list.head()?;
        let name = head.span.text(cursor.text);
        let callable = depth > 0 && offset > head.span.end && name != "and" && name != "let";
        callable.then_some((*list, name))
    })?;
    let def = lookup(index, head)?;
    let args = &def.args;
    let mut label = format!("({}", def.name);
    let mut params = Vec::new();
    for arg in args {
        label.push(' ');
        params.push((label.len(), label.len() + arg.len()));
        label.push_str(arg);
    }
    label.push(')');
    if let Some(ret) = &def.ret {
        label.push_str(&format!(" -> {ret}"));
    }
    let active = list.items.iter().skip(1).filter(|node| node.span().end < offset).count();
    Some(Signature { label, params, active })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter {
    Any,
    Extractor,
    Constructor,
}

fn keyword(word: &str) -> Item {
    Item { label: word.to_owned(), kind: ItemKind::Keyword, detail: String::new(), snippet: None }
}

fn keywords() -> Vec<Item> {
    FORMS.iter().map(|(word, _)| keyword(word)).collect()
}

fn types(index: &Index) -> Vec<Item> {
    index
        .defs
        .iter()
        .filter(|def| def.kind == DefKind::Type)
        .map(|def| Item { label: def.name.clone(), kind: ItemKind::Type, detail: "type".to_owned(), snippet: None })
        .collect()
}

fn is_extractor(index: &Index, id: usize) -> bool {
    let def = &index.defs[id];
    match def.kind {
        DefKind::Variant | DefKind::Macro => true,
        DefKind::Term => def.impls.iter().any(|imp| imp.ends_with("extractor")),
        _ => false,
    }
}

fn is_constructor(index: &Index, id: usize) -> bool {
    let def = &index.defs[id];
    match def.kind {
        DefKind::Variant | DefKind::Macro => true,
        DefKind::Term => {
            let defined = index.references(id).any(|r| r.role == Role::RuleHead);
            let external = def.impls.contains(&"external constructor");
            defined || external || def.impls.is_empty()
        }
        _ => false,
    }
}

fn terms(index: &Index, expected: Option<&str>, filter: Filter) -> Vec<Item> {
    let mut items = Vec::new();
    for (id, def) in index.defs.iter().enumerate() {
        if !matches!(def.kind, DefKind::Term | DefKind::Variant | DefKind::Macro) {
            continue;
        }
        let fits = match filter {
            Filter::Any => true,
            Filter::Extractor => is_extractor(index, id),
            Filter::Constructor => is_constructor(index, id),
        };
        let type_ok = match (expected, &def.ret) {
            (Some(want), Some(have)) => want == have,
            _ => true,
        };
        if !fits || !type_ok || items.iter().any(|item: &Item| item.label == def.name) {
            continue;
        }
        let snippet = (!def.args.is_empty()).then(|| {
            let stops: Vec<String> = def.args.iter().enumerate().map(|(i, ty)| format!("${{{}:{ty}}}", i + 1)).collect();
            format!("{} {}", def.name, stops.join(" "))
        });
        let detail = match &def.ret {
            Some(ret) => format!("({}) -> {ret}", def.args.join(" ")),
            None => String::new(),
        };
        let kind = if def.kind == DefKind::Variant { ItemKind::Variant } else { ItemKind::Term };
        items.push(Item { label: def.name.clone(), kind, detail, snippet });
    }
    items
}

fn values(index: &Index, cursor: &Cursor, file: usize, expected: Option<&str>) -> Vec<Item> {
    let mut items = Vec::new();
    let form = cursor.path.first().map(|list| list.span);
    for def in &index.defs {
        let type_ok = |ty: &Option<String>| match (expected, ty) {
            (Some(want), Some(have)) => want == have,
            _ => true,
        };
        match def.kind {
            DefKind::Variable if def.file == file && form.is_some_and(|form| contains(form, def)) => {
                if def.name_span.end <= cursor.offset && type_ok(&def.ty) && !items.iter().any(|i: &Item| i.label == def.name) {
                    let detail = def.ty.clone().unwrap_or_default();
                    items.push(Item { label: def.name.clone(), kind: ItemKind::Variable, detail, snippet: None });
                }
            }
            DefKind::Const if type_ok(&def.ty) => {
                let detail = def.ty.clone().unwrap_or_default();
                items.push(Item { label: def.name.clone(), kind: ItemKind::Const, detail, snippet: None });
            }
            _ => {}
        }
    }
    items
}

fn contains(form: Span, def: &Def) -> bool {
    form.start <= def.name_span.start && def.name_span.end <= form.end
}

fn lookup<'i>(index: &'i Index, name: &str) -> Option<&'i Def> {
    index
        .defs
        .iter()
        .find(|def| matches!(def.kind, DefKind::Term | DefKind::Variant | DefKind::Macro) && def.name == name)
}

fn rule_lhs_index(list: &List) -> usize {
    let mut idx = 1;
    if matches!(list.items.get(idx), Some(Node::Atom(atom)) if atom.kind == AtomKind::Symbol) {
        idx += 1;
    }
    if matches!(list.items.get(idx), Some(Node::Atom(atom)) if atom.kind == AtomKind::Int) {
        idx += 1;
    }
    idx
}

fn descend<'a>(items: &'a [Node], offset: usize, limit: usize, path: &mut Vec<&'a List>) {
    for (i, node) in items.iter().enumerate() {
        let Node::List(list) = node else { continue };
        let high = if list.closed {
            list.span.end
        } else {
            items.get(i + 1).map_or(limit, |next| next.span().start)
        };
        let inside = list.span.start < offset && if list.closed { offset < high } else { offset <= high };
        if inside {
            path.push(list);
            descend(&list.items, offset, high, path);
            return;
        }
    }
}

impl<'a> Cursor<'a> {
    fn head_is(&self, list: &List, word: &str) -> bool {
        list.head().is_some_and(|head| head.span.text(self.text) == word)
    }

    fn slot(&self, list: &List) -> usize {
        let partial = list.items.iter().position(|node| node.span().start < self.offset && self.offset <= node.span().end);
        partial.unwrap_or_else(|| list.items.iter().filter(|node| node.span().end <= self.offset).count())
    }

    fn position_in_parent(&self, depth: usize) -> usize {
        let parent = self.path[depth - 1];
        parent.items.iter().position(|node| node.span() == self.path[depth].span).unwrap_or(0)
    }

    fn mode(&self, depth: usize) -> Mode {
        let parent = self.path[depth - 1];
        let position = self.position_in_parent(depth);
        if depth == 1 {
            return match form_kind(self.text, parent) {
                Some(FormKind::Rule) => {
                    let lhs = rule_lhs_index(parent);
                    if position == lhs {
                        Mode::Lhs
                    } else if position + 1 == parent.items.len() {
                        Mode::Expr
                    } else {
                        Mode::Clause
                    }
                }
                Some(FormKind::Decl) => Mode::Types,
                Some(FormKind::Extractor) if position == 2 => Mode::Pattern,
                Some(FormKind::Macro) if position >= 2 => Mode::Expr,
                _ => Mode::Other,
            };
        }
        match self.mode(depth - 1) {
            Mode::Lhs | Mode::Pattern => Mode::Pattern,
            Mode::Expr if self.head_is(parent, "let") && position == 1 => Mode::Bindings,
            Mode::Expr => Mode::Expr,
            Mode::Bindings => Mode::Binding,
            Mode::Binding if position == 2 => Mode::Expr,
            Mode::Clause if self.head_is(parent, "if-let") && position == 1 => Mode::Pattern,
            Mode::Clause if position >= 1 && (position == 2 || self.head_is(parent, "if")) => Mode::Expr,
            _ => Mode::Other,
        }
    }

    fn call_def<'i>(&self, index: &'i Index, list: &List) -> Option<&'i Def> {
        let head = list.head()?;
        lookup(index, head.span.text(self.text))
    }

    fn argument_type(&self, index: &Index, list: &List, slot: usize) -> Option<String> {
        self.call_def(index, list)?.args.get(slot.checked_sub(1)?).cloned()
    }

    fn expected(&self, index: &Index, depth: usize) -> Option<String> {
        let parent = self.path[depth - 1];
        if depth == 1 {
            let lhs = parent.items.get(rule_lhs_index(parent))?;
            let Node::List(lhs) = lhs else { return None };
            return self.call_def(index, lhs)?.ret.clone();
        }
        let position = self.position_in_parent(depth);
        self.argument_type(index, parent, position)
    }
}
