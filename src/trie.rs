use cranelift_isle::lexer::Pos;
use cranelift_isle::serialize::{self, Block, ControlFlow};
use cranelift_isle::trie_again::{Binding, BindingId, Constraint, RuleSet};

use crate::compiled::Compiled;
use crate::html::{self, escape};
use crate::view::{SCHEMA, Section, TreeNode, View};

#[derive(Clone, Copy)]
enum Kind {
    Let,
    Match,
    Arm,
    Equal,
    Loop,
    Leaf,
}

impl Kind {
    fn class(self) -> &'static str {
        match self {
            Kind::Let => "let",
            Kind::Match => "match",
            Kind::Arm => "arm",
            Kind::Equal => "equal",
            Kind::Loop => "loop",
            Kind::Leaf => "leaf",
        }
    }
}

struct Node {
    kind: Kind,
    label: String,
    children: Vec<Node>,
}

struct Walk<'a> {
    compiled: &'a Compiled,
    set: &'a RuleSet,
    rules: Vec<Vec<String>>,
}

fn var(id: BindingId) -> String {
    format!("v{}", id.index())
}

impl Walk<'_> {
    fn join(&self, ids: &[BindingId]) -> String {
        ids.iter().map(|&id| var(id)).collect::<Vec<_>>().join(", ")
    }

    fn describe(&self, id: BindingId) -> String {
        let compiled = self.compiled;
        match &self.set.bindings[id.index()] {
            Binding::ConstBool { val, .. } => val.to_string(),
            Binding::ConstInt { val, .. } => val.to_string(),
            Binding::ConstPrim { val } => compiled.types.syms[val.index()].clone(),
            Binding::Argument { index } => format!("arg{}", index.index()),
            Binding::Extractor { term, parameter } => format!("{}({})", compiled.term_name(*term), var(*parameter)),
            Binding::Constructor { term, parameters, .. } => {
                format!("{}({})", compiled.term_name(*term), self.join(parameters))
            }
            Binding::Iterator { source } => format!("each {}", var(*source)),
            Binding::MakeVariant { ty, variant, fields } => {
                format!("{}({})", compiled.variant_name(*ty, *variant), self.join(fields))
            }
            Binding::MatchVariant { source, field, .. } => format!("{}.{}", var(*source), field.index()),
            Binding::MakeStruct { ty, fields } => format!("{}({})", compiled.type_name(*ty), self.join(fields)),
            Binding::ExtractStruct { source, field } => format!("{}.{}", var(*source), field.index()),
            Binding::MakeSome { inner } => format!("Some({})", var(*inner)),
            Binding::MatchSome { source } => format!("{}.some", var(*source)),
            Binding::MatchTuple { source, field } => format!("{}.{}", var(*source), field.index()),
        }
    }

    fn constraint(&self, constraint: &Constraint) -> String {
        let compiled = self.compiled;
        match constraint {
            Constraint::Variant { ty, variant, .. } => compiled.variant_name(*ty, *variant),
            Constraint::Struct { ty, .. } => compiled.type_name(*ty),
            Constraint::ConstBool { val, .. } => val.to_string(),
            Constraint::ConstInt { val, .. } => val.to_string(),
            Constraint::ConstPrim { val } => compiled.types.syms[val.index()].clone(),
            Constraint::Some => "Some".to_owned(),
        }
    }

    fn block(&mut self, block: &Block) -> Vec<Node> {
        let mut out = Vec::new();
        for step in &block.steps {
            for &id in &step.bind_order {
                out.push(Node { kind: Kind::Let, label: format!("{} = {}", var(id), self.describe(id)), children: Vec::new() });
            }
            out.push(match &step.check {
                ControlFlow::Match { source, arms } => Node {
                    kind: Kind::Match,
                    label: format!("match {}", var(*source)),
                    children: arms
                        .iter()
                        .map(|arm| {
                            let bound: Vec<String> = arm.bindings.iter().flatten().map(|&id| var(id)).collect();
                            let binds = if bound.is_empty() { String::new() } else { format!(" binds {}", bound.join(", ")) };
                            Node {
                                kind: Kind::Arm,
                                label: format!("{}{binds}", self.constraint(&arm.constraint)),
                                children: self.block(&arm.body),
                            }
                        })
                        .collect(),
                },
                ControlFlow::Equal { a, b, body } => Node {
                    kind: Kind::Equal,
                    label: format!("{} == {}", var(*a), var(*b)),
                    children: self.block(body),
                },
                ControlFlow::Loop { result, body } => Node {
                    kind: Kind::Loop,
                    label: format!("for each {}", var(*result)),
                    children: self.block(body),
                },
                ControlFlow::Return { pos, result } => self.leaf(*pos, *result),
            });
        }
        out
    }

    fn leaf(&mut self, pos: Pos, result: BindingId) -> Node {
        let rule = self.set.rules.iter().find(|rule| rule.pos == pos);
        let name = rule.and_then(|rule| rule.name).map(|name| self.compiled.types.syms[name.index()].clone());
        let priority = rule.map_or(0, |rule| rule.prio);
        let location = pos.pretty_print_line(&self.compiled.files);
        self.rules.push(vec![
            (self.rules.len() + 1).to_string(),
            name.clone().unwrap_or_else(|| "-".to_owned()),
            priority.to_string(),
            location.clone(),
        ]);
        let label = format!(
            "return {} = {} [{}priority {priority}, {location}]",
            var(result),
            self.describe(result),
            name.map(|name| format!("{name}, ")).unwrap_or_default()
        );
        Node { kind: Kind::Leaf, label, children: Vec::new() }
    }
}

fn tree(nodes: &[Node]) -> Vec<TreeNode> {
    nodes.iter().map(|node| TreeNode { label: node.label.clone(), children: tree(&node.children) }).collect()
}

fn markup(nodes: &[Node], out: &mut String) {
    for node in nodes {
        let class = node.kind.class();
        let label = escape(&node.label);
        if node.children.is_empty() {
            out.push_str(&format!("<div class=\"n {class}\">{label}</div>\n"));
        } else {
            out.push_str(&format!("<details open><summary class=\"n {class}\">{label}</summary><div class=\"kids\">\n"));
            markup(&node.children, out);
            out.push_str("</div></details>\n");
        }
    }
}

const CSS: &str = r#"
.trie { padding: 4px 0; }
.n { padding: 0 8px; border: 1px solid var(--border); margin: 2px 0; width: fit-content; max-width: 100%; white-space: pre-wrap; }
summary.n { cursor: pointer; }
.kids { margin-left: 18px; padding-left: 10px; border-left: 1px solid var(--line); }
.n.let { color: var(--muted); border-style: dashed; }
.n.match { background: var(--match); font-weight: 600; }
.n.arm { background: var(--bg); border-left: 4px solid var(--hl); }
.n.equal { background: var(--equal); }
.n.loop { background: var(--loop); }
.n.leaf { background: var(--leaf); }
.tools { margin: 0 0 8px; }
.tools button { font: inherit; margin-right: 6px; }
"#;

const SCRIPT: &str = r#"
for (const button of document.querySelectorAll('[data-open]')) {
  button.addEventListener('click', () => {
    for (const d of document.querySelectorAll('details')) d.open = button.dataset.open === 'all';
  });
}
"#;

pub fn view(compiled: &Compiled, term: &str) -> Result<View, String> {
    let (_, set) = compiled.find(term).ok_or_else(|| format!("`{term}` has no rules in this project"))?;
    let root = serialize::serialize(set);
    let mut walk = Walk { compiled, set, rules: Vec::new() };
    let nodes = walk.block(&root);

    let title = format!("Decision trie of {term}");
    let mut body = String::from(
        "<p class=\"tools\"><button data-open=\"all\">expand all</button><button data-open=\"none\">collapse all</button></p>\n<div class=\"trie mono\">\n",
    );
    markup(&nodes, &mut body);
    body.push_str("</div>\n");
    let page = html::page(
        &title,
        &[("match", "match"), ("bg", "pattern"), ("equal", "equality test"), ("loop", "iteration"), ("leaf", "rule result")],
        CSS,
        &body,
        SCRIPT,
    );

    Ok(View {
        schema: SCHEMA,
        view: "trie".to_owned(),
        term: term.to_owned(),
        title,
        sections: vec![
            Section::Tree { title: "Trie".to_owned(), nodes: tree(&nodes) },
            Section::Html { title: "Trie page".to_owned(), html: page },
            Section::Table {
                title: "Rules in evaluation order".to_owned(),
                columns: ["#", "rule", "priority", "location"].map(str::to_owned).to_vec(),
                rows: walk.rules,
            },
        ],
    })
}
