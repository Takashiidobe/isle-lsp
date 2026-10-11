use std::collections::HashMap;

use crate::project::Loaded;
use crate::syntax::{Atom, AtomKind, FormKind, List, Node, Span, form_kind};

pub type DefId = usize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefKind {
    Type,
    Variant,
    Term,
    Const,
    Macro,
    Rule,
    Variable,
}

impl DefKind {
    pub fn label(self) -> &'static str {
        match self {
            DefKind::Type => "type",
            DefKind::Variant => "variant",
            DefKind::Term => "term",
            DefKind::Const => "const",
            DefKind::Macro => "macro",
            DefKind::Rule => "rule",
            DefKind::Variable => "variable",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Type,
    Call,
    Match,
    RuleHead,
    Impl,
    Attr,
    Const,
    Variable,
}

impl Role {
    pub fn label(self) -> &'static str {
        match self {
            Role::Type => "type",
            Role::Call => "call",
            Role::Match => "match",
            Role::RuleHead => "rule-head",
            Role::Impl => "impl",
            Role::Attr => "attr",
            Role::Const => "const",
            Role::Variable => "variable",
        }
    }
}

#[derive(Debug)]
pub struct Def {
    pub kind: DefKind,
    pub name: String,
    pub file: usize,
    pub name_span: Span,
}

#[derive(Debug)]
pub struct Reference {
    pub file: usize,
    pub span: Span,
    pub role: Role,
    pub def: Option<DefId>,
}

#[derive(Debug, Default)]
pub struct Index {
    pub defs: Vec<Def>,
    pub refs: Vec<Reference>,
}

impl Index {
    pub fn build(loaded: &Loaded) -> Index {
        let mut builder = Builder::default();
        for (file, source) in loaded.sources.iter().enumerate() {
            builder.file = file;
            builder.text = source.text.as_str();
            for node in &source.file.items {
                if let Node::List(list) = node {
                    builder.declare(list);
                }
            }
        }
        for (file, source) in loaded.sources.iter().enumerate() {
            builder.file = file;
            builder.text = source.text.as_str();
            for node in &source.file.items {
                if let Node::List(list) = node {
                    builder.walk_form(list);
                }
            }
        }
        builder.index
    }

    pub fn resolve(&self, file: usize, offset: usize) -> Option<DefId> {
        let covers = |span: Span| span.start <= offset && offset <= span.end;
        if let Some(id) = self.defs.iter().position(|def| def.file == file && covers(def.name_span)) {
            return Some(id);
        }
        self.refs.iter().find(|r| r.file == file && covers(r.span)).and_then(|r| r.def)
    }

    pub fn references(&self, def: DefId) -> impl Iterator<Item = &Reference> {
        self.refs.iter().filter(move |r| r.def == Some(def))
    }
}

#[derive(Default)]
struct Builder<'a> {
    index: Index,
    file: usize,
    text: &'a str,
    types: HashMap<String, DefId>,
    terms: HashMap<String, DefId>,
    consts: HashMap<String, DefId>,
    scope: Vec<(String, DefId)>,
}

impl<'a> Builder<'a> {
    fn define(&mut self, kind: DefKind, name: &str, name_span: Span) -> DefId {
        self.index.defs.push(Def { kind, name: name.to_owned(), file: self.file, name_span });
        self.index.defs.len() - 1
    }

    fn reference(&mut self, span: Span, role: Role, def: Option<DefId>) {
        self.index.refs.push(Reference { file: self.file, span, role, def });
    }

    fn symbol(&self, node: &Node) -> Option<(&'a str, Span)> {
        match node {
            Node::Atom(atom) if atom.kind == AtomKind::Symbol => Some((atom.span.text(self.text), atom.span)),
            _ => None,
        }
    }

    fn head_text(&self, list: &List) -> Option<&'a str> {
        list.head().map(|head| head.span.text(self.text))
    }

    fn declare(&mut self, list: &List) {
        let Some(kind) = form_kind(self.text, list) else { return };
        match kind {
            FormKind::Type => self.declare_type(list),
            FormKind::Decl => {
                if let Some((_, name, span)) = self.decl_name(list) {
                    let id = self.define(DefKind::Term, name, span);
                    self.terms.entry(name.to_owned()).or_insert(id);
                }
            }
            FormKind::Macro => {
                if let Some(Node::List(signature)) = list.items.get(1)
                    && let Some(head) = signature.head()
                {
                    let name = head.span.text(self.text);
                    let id = self.define(DefKind::Macro, name, head.span);
                    self.terms.entry(name.to_owned()).or_insert(id);
                }
            }
            FormKind::Extern => {
                if self.head_after(list, 1) == Some("const")
                    && let Some((name, span)) = list.items.get(2).and_then(|node| self.symbol(node))
                {
                    let id = self.define(DefKind::Const, name, span);
                    self.consts.entry(name.to_owned()).or_insert(id);
                }
            }
            FormKind::Rule => {
                if let Some((name, span)) = list.items.get(1).and_then(|node| self.symbol(node)) {
                    self.define(DefKind::Rule, name, span);
                }
            }
            _ => {}
        }
    }

    fn head_after(&self, list: &List, idx: usize) -> Option<&'a str> {
        list.items.get(idx).and_then(|node| self.symbol(node)).map(|(text, _)| text)
    }

    fn declare_type(&mut self, list: &List) {
        let Some((name, span)) = list.items.get(1).and_then(|node| self.symbol(node)) else { return };
        let id = self.define(DefKind::Type, name, span);
        self.types.entry(name.to_owned()).or_insert(id);
        for item in list.items.iter().skip(2) {
            let Node::List(body) = item else { continue };
            if self.head_text(body) != Some("enum") {
                continue;
            }
            for variant in body.items.iter().skip(1) {
                let atom: &Atom = match variant {
                    Node::Atom(atom) if atom.kind == AtomKind::Symbol => atom,
                    Node::List(list) => match list.head() {
                        Some(head) => head,
                        None => continue,
                    },
                    Node::Atom(_) => continue,
                };
                let full = format!("{name}.{}", atom.span.text(self.text));
                let id = self.define(DefKind::Variant, &full, atom.span);
                self.terms.entry(full).or_insert(id);
            }
        }
    }

    fn decl_name(&self, list: &List) -> Option<(usize, &'a str, Span)> {
        let mut name = None;
        for (idx, node) in list.items.iter().enumerate().skip(1) {
            match self.symbol(node) {
                Some((text, span)) => name = Some((idx, text, span)),
                None => break,
            }
        }
        name
    }

    fn walk_form(&mut self, list: &List) {
        self.scope.clear();
        let Some(kind) = form_kind(self.text, list) else { return };
        match kind {
            FormKind::Type => self.walk_type(list),
            FormKind::Decl => self.walk_decl(list),
            FormKind::Attr => {
                if let Some((name, span)) = list.items.get(1).and_then(|node| self.symbol(node)) {
                    let def = self.terms.get(name).copied();
                    self.reference(span, Role::Attr, def);
                }
            }
            FormKind::Extern => self.walk_extern(list),
            FormKind::Extractor => self.walk_extractor(list),
            FormKind::Convert => {
                for node in list.items.iter().skip(1).take(2) {
                    self.type_ref(node);
                }
                if let Some((name, span)) = list.items.get(3).and_then(|node| self.symbol(node)) {
                    let def = self.terms.get(name).copied();
                    self.reference(span, Role::Call, def);
                }
            }
            FormKind::Macro => self.walk_macro(list),
            FormKind::Rule => self.walk_rule(list),
            _ => {}
        }
    }

    fn type_ref(&mut self, node: &Node) {
        if let Some((name, span)) = self.symbol(node) {
            let def = self.types.get(name).copied();
            self.reference(span, Role::Type, def);
        }
    }

    fn walk_type(&mut self, list: &List) {
        for item in list.items.iter().skip(2) {
            let Node::List(body) = item else { continue };
            if self.head_text(body) != Some("enum") {
                continue;
            }
            for variant in body.items.iter().skip(1) {
                let Node::List(variant) = variant else { continue };
                for field in variant.items.iter().skip(1) {
                    if let Node::List(field) = field
                        && let Some(ty) = field.items.get(1)
                    {
                        self.type_ref(ty);
                    }
                }
            }
        }
    }

    fn walk_decl(&mut self, list: &List) {
        let Some((idx, _, _)) = self.decl_name(list) else { return };
        if let Some(Node::List(args)) = list.items.get(idx + 1) {
            for arg in &args.items {
                self.type_ref(arg);
            }
        }
        if let Some(ret) = list.items.get(idx + 2) {
            self.type_ref(ret);
        }
    }

    fn walk_extern(&mut self, list: &List) {
        match self.head_after(list, 1) {
            Some("const") => {
                if let Some(ty) = list.items.get(3) {
                    self.type_ref(ty);
                }
            }
            Some("constructor" | "extractor") => {
                let named = list.items.iter().skip(2).find_map(|node| match self.symbol(node) {
                    Some(("infallible", _)) => None,
                    other => other,
                });
                if let Some((name, span)) = named {
                    let def = self.terms.get(name).copied();
                    self.reference(span, Role::Impl, def);
                }
            }
            _ => {}
        }
    }

    fn walk_extractor(&mut self, list: &List) {
        let Some(Node::List(signature)) = list.items.get(1) else { return };
        if let Some(head) = signature.head() {
            let def = self.terms.get(head.span.text(self.text)).copied();
            self.reference(head.span, Role::Impl, def);
        }
        for param in signature.items.iter().skip(1) {
            if let Some((name, span)) = self.symbol(param) {
                self.bind_new(name, span);
            }
        }
        if let Some(template) = list.items.get(2) {
            self.pattern(template);
        }
    }

    fn walk_macro(&mut self, list: &List) {
        let Some(Node::List(signature)) = list.items.get(1) else { return };
        for param in signature.items.iter().skip(1) {
            if let Some((name, span)) = self.symbol(param) {
                self.bind_new(name, span);
            }
        }
        for body in list.items.iter().skip(2) {
            self.expr(body);
        }
    }

    fn walk_rule(&mut self, list: &List) {
        let mut idx = 1;
        if self.symbol_at(list, idx).is_some() {
            idx += 1;
        }
        if matches!(list.items.get(idx), Some(Node::Atom(atom)) if atom.kind == AtomKind::Int) {
            idx += 1;
        }
        let Some(lhs) = list.items.get(idx) else { return };
        match lhs {
            Node::List(call) => {
                if let Some(head) = call.head() {
                    let def = self.terms.get(head.span.text(self.text)).copied();
                    self.reference(head.span, Role::RuleHead, def);
                }
                self.patterns(call.items.get(1..).unwrap_or_default());
            }
            other => self.pattern(other),
        }
        let rest = list.items.get(idx + 1..).unwrap_or_default();
        let Some((rhs, clauses)) = rest.split_last() else { return };
        for clause in clauses {
            let Node::List(clause) = clause else { continue };
            match self.head_text(clause) {
                Some("if-let") => {
                    if let Some(value) = clause.items.get(2) {
                        self.expr(value);
                    }
                    if let Some(pattern) = clause.items.get(1) {
                        self.pattern(pattern);
                    }
                }
                Some("if") => {
                    if let Some(condition) = clause.items.get(1) {
                        self.expr(condition);
                    }
                }
                _ => {}
            }
        }
        self.expr(rhs);
    }

    fn symbol_at(&self, list: &List, idx: usize) -> Option<(&'a str, Span)> {
        list.items.get(idx).and_then(|node| self.symbol(node))
    }

    fn bind_new(&mut self, name: &str, span: Span) -> DefId {
        let id = self.define(DefKind::Variable, name, span);
        self.scope.push((name.to_owned(), id));
        id
    }

    fn lookup(&self, name: &str) -> Option<DefId> {
        self.scope.iter().rev().find(|(candidate, _)| candidate == name).map(|&(_, id)| id)
    }

    fn bind(&mut self, name: &str, span: Span) {
        match self.lookup(name) {
            Some(id) => self.reference(span, Role::Variable, Some(id)),
            None => {
                self.bind_new(name, span);
            }
        }
    }

    fn patterns(&mut self, nodes: &[Node]) {
        let mut idx = 0;
        while idx < nodes.len() {
            let binds_subpattern = matches!(nodes.get(idx + 1), Some(Node::Atom(atom)) if atom.kind == AtomKind::At);
            if binds_subpattern && let Some((name, span)) = self.symbol(&nodes[idx]) {
                self.bind(name, span);
                if let Some(sub) = nodes.get(idx + 2) {
                    self.pattern(sub);
                }
                idx += 3;
            } else {
                self.pattern(&nodes[idx]);
                idx += 1;
            }
        }
    }

    fn pattern(&mut self, node: &Node) {
        match node {
            Node::Atom(atom) if atom.kind == AtomKind::Symbol => {
                let name = atom.span.text(self.text);
                if name == "_" || name.starts_with('#') {
                    return;
                }
                if name.starts_with('$') {
                    let def = self.consts.get(name).copied();
                    self.reference(atom.span, Role::Const, def);
                } else if let Some(&def) = self.terms.get(name)
                    && self.index.defs[def].kind == DefKind::Variant
                {
                    self.reference(atom.span, Role::Match, Some(def));
                } else {
                    self.bind(name, atom.span);
                }
            }
            Node::Atom(_) => {}
            Node::List(list) => {
                let Some(head) = list.head() else { return };
                let name = head.span.text(self.text);
                if name != "and" {
                    let def = self.terms.get(name).copied();
                    self.reference(head.span, Role::Match, def);
                }
                self.patterns(&list.items[1..]);
            }
        }
    }

    fn expr(&mut self, node: &Node) {
        match node {
            Node::Atom(atom) if atom.kind == AtomKind::Symbol => {
                let name = atom.span.text(self.text);
                if name.starts_with('#') {
                    return;
                }
                if name.starts_with('$') {
                    let def = self.consts.get(name).copied();
                    self.reference(atom.span, Role::Const, def);
                } else if let Some(id) = self.lookup(name) {
                    self.reference(atom.span, Role::Variable, Some(id));
                } else {
                    let def = self.terms.get(name).copied();
                    let role = if def.is_some() { Role::Call } else { Role::Variable };
                    self.reference(atom.span, role, def);
                }
            }
            Node::Atom(_) => {}
            Node::List(list) => {
                let Some(head) = list.head() else { return };
                if head.span.text(self.text) == "let" {
                    self.let_expr(list);
                    return;
                }
                let def = self.terms.get(head.span.text(self.text)).copied();
                self.reference(head.span, Role::Call, def);
                for arg in &list.items[1..] {
                    self.expr(arg);
                }
            }
        }
    }

    fn let_expr(&mut self, list: &List) {
        let mark = self.scope.len();
        if let Some(Node::List(bindings)) = list.items.get(1) {
            for binding in &bindings.items {
                let Node::List(binding) = binding else { continue };
                if let Some(ty) = binding.items.get(1) {
                    self.type_ref(ty);
                }
                if let Some(value) = binding.items.get(2) {
                    self.expr(value);
                }
                if let Some((name, span)) = self.symbol_at(binding, 0) {
                    self.bind_new(name, span);
                }
            }
        }
        for body in list.items.iter().skip(2) {
            self.expr(body);
        }
        self.scope.truncate(mark);
    }
}
