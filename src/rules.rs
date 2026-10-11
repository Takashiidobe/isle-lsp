use std::collections::HashMap;

use crate::project::Loaded;
use crate::syntax::{AtomKind, FormKind, List, Node, Span, form_kind};

pub struct Rule<'a> {
    pub file: usize,
    pub form: Span,
    pub list: &'a List,
    pub name: Option<Span>,
    pub priority: i64,
    pub head: &'a str,
    pub head_span: Span,
    pub args: &'a [Node],
    pub clauses: &'a [Node],
    pub rhs: &'a Node,
    pub text: &'a str,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Wild,
    Var(String),
    Int(String),
    Lit(String),
    And(Vec<Shape>),
    Call(String, Vec<Shape>),
    Other,
}

pub fn rules(loaded: &Loaded) -> Vec<Rule<'_>> {
    let mut found = Vec::new();
    for (file, source) in loaded.sources.iter().enumerate() {
        for node in &source.file.items {
            let Node::List(list) = node else { continue };
            if form_kind(&source.text, list) != Some(FormKind::Rule) {
                continue;
            }
            if let Some(rule) = parse(file, &source.text, list) {
                found.push(rule);
            }
        }
    }
    found
}

fn parse<'a>(file: usize, text: &'a str, list: &'a List) -> Option<Rule<'a>> {
    let mut idx = 1;
    let mut name = None;
    if let Some(Node::Atom(atom)) = list.items.get(idx)
        && atom.kind == AtomKind::Symbol
    {
        name = Some(atom.span);
        idx += 1;
    }
    let mut priority = 0;
    if let Some(Node::Atom(atom)) = list.items.get(idx)
        && atom.kind == AtomKind::Int
    {
        priority = atom.span.text(text).replace('_', "").parse().unwrap_or(0);
        idx += 1;
    }
    let Some(Node::List(lhs)) = list.items.get(idx) else { return None };
    let head = lhs.head()?;
    let (rhs, clauses) = list.items.get(idx + 1..)?.split_last()?;
    if !list.closed {
        return None;
    }
    Some(Rule {
        file,
        form: list.span,
        list,
        name,
        priority,
        head: head.span.text(text),
        head_span: head.span,
        args: lhs.items.get(1..).unwrap_or_default(),
        clauses,
        rhs,
        text,
    })
}

pub fn shape(text: &str, node: &Node) -> Shape {
    match node {
        Node::Atom(atom) => {
            let word = atom.span.text(text);
            match atom.kind {
                AtomKind::Int => Shape::Int(word.replace('_', "")),
                AtomKind::At => Shape::Other,
                AtomKind::Symbol if word == "_" => Shape::Wild,
                AtomKind::Symbol if word.starts_with(['$', '#']) || word.contains('.') => Shape::Lit(word.to_owned()),
                AtomKind::Symbol => Shape::Var(word.to_owned()),
            }
        }
        Node::List(list) => {
            let Some(head) = list.head() else { return Shape::Other };
            let name = head.span.text(text);
            let args = shapes(text, list.items.get(1..).unwrap_or_default());
            match name {
                "and" => Shape::And(args),
                "let" => Shape::Other,
                _ => Shape::Call(name.to_owned(), args),
            }
        }
    }
}

pub fn shapes(text: &str, nodes: &[Node]) -> Vec<Shape> {
    let mut found = Vec::new();
    let mut idx = 0;
    while idx < nodes.len() {
        let binds = matches!(nodes.get(idx + 1), Some(Node::Atom(atom)) if atom.kind == AtomKind::At);
        if binds && let Some(sub) = nodes.get(idx + 2) {
            found.push(shape(text, sub));
            idx += 3;
        } else {
            found.push(shape(text, &nodes[idx]));
            idx += 1;
        }
    }
    found
}

pub fn cost(shape: &Shape) -> usize {
    match shape {
        Shape::Call(_, args) => 1 + args.iter().map(cost).sum::<usize>(),
        Shape::And(parts) => parts.iter().map(cost).sum(),
        Shape::Other => 1,
        _ => 0,
    }
}

pub fn overlaps(a: &Shape, b: &Shape) -> bool {
    match (a, b) {
        (Shape::Wild | Shape::Var(_) | Shape::Other, _) | (_, Shape::Wild | Shape::Var(_) | Shape::Other) => true,
        (Shape::And(parts), other) | (other, Shape::And(parts)) => parts.iter().all(|part| overlaps(part, other)),
        (Shape::Int(x), Shape::Int(y)) | (Shape::Lit(x), Shape::Lit(y)) => x == y,
        (Shape::Call(x, xs), Shape::Call(y, ys)) => {
            x == y && xs.len() == ys.len() && xs.iter().zip(ys).all(|(x, y)| overlaps(x, y))
        }
        _ => false,
    }
}

pub fn subsumes(general: &Shape, specific: &Shape) -> bool {
    fn go<'a>(general: &Shape, specific: &'a Shape, bound: &mut HashMap<String, &'a Shape>) -> bool {
        match (general, specific) {
            (Shape::Wild, _) => true,
            (Shape::Var(name), _) => match bound.get(name) {
                Some(&previous) => previous == specific,
                None => {
                    bound.insert(name.clone(), specific);
                    true
                }
            },
            (Shape::And(parts), _) => parts.iter().all(|part| go(part, specific, bound)),
            (_, Shape::And(parts)) => parts.iter().any(|part| go(general, part, bound)),
            (Shape::Int(x), Shape::Int(y)) | (Shape::Lit(x), Shape::Lit(y)) => x == y,
            (Shape::Call(x, xs), Shape::Call(y, ys)) => {
                x == y && xs.len() == ys.len() && xs.iter().zip(ys).all(|(x, y)| go(x, y, bound))
            }
            _ => false,
        }
    }
    go(general, specific, &mut HashMap::new())
}

pub fn same_pattern(a: &Shape, b: &Shape) -> bool {
    subsumes(a, b) && subsumes(b, a)
}

pub struct Clif {
    pub opcode: String,
    pub operands: Vec<Operand>,
}

pub enum Operand {
    Free,
    Imm(String),
    Value(Box<Clif>),
}

pub fn parse_clif(snippet: &str) -> Option<Clif> {
    let mut defined: HashMap<String, Clif> = HashMap::new();
    let mut last = None;
    let mut returned = None;
    for line in snippet.lines() {
        let line = line.split(';').next().unwrap_or("").trim();
        let (result, body) = match line.split_once('=') {
            Some((result, body)) => (Some(result.trim().to_owned()), body.trim()),
            None => (None, line),
        };
        let mut words = body.splitn(2, char::is_whitespace);
        let Some(op) = words.next().filter(|op| op.chars().next().is_some_and(|c| c.is_ascii_lowercase())) else { continue };
        let rest = words.next().unwrap_or("");
        if op == "return" {
            returned = rest.split(',').next().map(|v| v.trim().to_owned());
            continue;
        }
        if matches!(op, "function" | "block0" | "jump" | "brif" | "trap") || op.starts_with("block") {
            continue;
        }
        let opcode = op.split('.').next().unwrap_or(op).to_owned();
        let operands = rest
            .split(',')
            .map(str::trim)
            .filter(|word| !word.is_empty())
            .map(|word| match defined.get(word) {
                Some(inner) => Operand::Value(Box::new(clone_clif(inner))),
                None if word.starts_with('v') && word[1..].chars().all(|c| c.is_ascii_digit()) => Operand::Free,
                None => Operand::Imm(word.to_owned()),
            })
            .collect();
        let clif = Clif { opcode, operands };
        if let Some(result) = result {
            defined.insert(result.clone(), clone_clif(&clif));
            last = Some(result);
        }
    }
    let root = returned.filter(|name| defined.contains_key(name)).or(last)?;
    defined.remove(&root)
}

fn clone_clif(clif: &Clif) -> Clif {
    Clif {
        opcode: clif.opcode.clone(),
        operands: clif
            .operands
            .iter()
            .map(|operand| match operand {
                Operand::Free => Operand::Free,
                Operand::Imm(word) => Operand::Imm(word.clone()),
                Operand::Value(inner) => Operand::Value(Box::new(clone_clif(inner))),
            })
            .collect(),
    }
}

pub fn could_match(shape: &Shape, clif: &Operand, opcodes: &dyn Fn(&str) -> bool) -> bool {
    match (shape, clif) {
        (Shape::Wild | Shape::Var(_) | Shape::Other | Shape::Lit(_), _) => true,
        (Shape::And(parts), _) => parts.iter().all(|part| could_match(part, clif, opcodes)),
        (Shape::Int(wanted), Operand::Imm(found)) => wanted == found,
        (Shape::Int(wanted), Operand::Value(inner)) => {
            inner.opcode != "iconst" || inner.operands.iter().any(|o| matches!(o, Operand::Imm(found) if found == wanted))
        }
        (Shape::Int(_), Operand::Free) => true,
        (Shape::Call(..), Operand::Free) => true,
        (Shape::Call(..), Operand::Imm(_)) => false,
        (Shape::Call(head, args), Operand::Value(inner)) => {
            if head == "has_type" {
                return args.get(1).is_none_or(|arg| could_match(arg, clif, opcodes));
            }
            if *head == inner.opcode {
                let operands = &inner.operands;
                let skip = usize::from(args.len() == operands.len() + 1);
                return args[skip..].iter().zip(operands).all(|(arg, operand)| could_match(arg, operand, opcodes));
            }
            if head.contains("iconst") {
                if inner.opcode != "iconst" {
                    return false;
                }
                return match (args.last(), inner.operands.first()) {
                    (Some(Shape::Int(wanted)), Some(Operand::Imm(found))) => wanted == found,
                    _ => true,
                };
            }
            !opcodes(head)
        }
    }
}

pub struct Ranked {
    pub file: usize,
    pub span: Span,
    pub priority: i64,
    pub wins_over: Vec<(usize, Span)>,
}

pub fn for_term(loaded: &Loaded, term: &str) -> Vec<Ranked> {
    let all = rules(loaded);
    let same: Vec<(&Rule, Vec<Shape>)> =
        all.iter().filter(|rule| rule.head == term).map(|rule| (rule, shapes(rule.text, rule.args))).collect();
    let mut ranked: Vec<Ranked> = same
        .iter()
        .map(|(rule, own)| Ranked {
            file: rule.file,
            span: rule.form,
            priority: rule.priority,
            wins_over: same
                .iter()
                .filter(|(other, theirs)| {
                    other.priority < rule.priority
                        && own.len() == theirs.len()
                        && own.iter().zip(theirs).all(|(a, b)| overlaps(a, b))
                })
                .map(|(other, _)| (other.file, other.form))
                .collect(),
        })
        .collect();
    ranked.sort_by_key(|r| (std::cmp::Reverse(r.priority), r.file, r.span.start));
    ranked
}

pub fn for_clif(loaded: &Loaded, index: &crate::index::Index, snippet: &str) -> Vec<Ranked> {
    let Some(root) = parse_clif(snippet) else { return Vec::new() };
    let known: std::collections::HashSet<&str> = index
        .defs
        .iter()
        .filter(|def| {
            loaded.sources[def.file].path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("clif_"))
        })
        .map(|def| def.name.as_str())
        .collect();
    let opcodes = |name: &str| known.contains(name);
    let operand = Operand::Value(Box::new(root));
    let mut found: Vec<Ranked> = rules(loaded)
        .iter()
        .filter(|rule| shapes(rule.text, rule.args).first().is_some_and(|first| could_match(first, &operand, &opcodes)))
        .map(|rule| Ranked { file: rule.file, span: rule.form, priority: rule.priority, wins_over: Vec::new() })
        .collect();
    found.sort_by_key(|r| (std::cmp::Reverse(r.priority), r.file, r.span.start));
    found
}
