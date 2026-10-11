use std::collections::HashMap;

use crate::lints;
use crate::project::Loaded;
use crate::rules::{self, Shape};

const OPCODES: [&str; 28] = [
    "iadd", "isub", "ineg", "imul", "udiv", "sdiv", "urem", "srem", "band", "bor", "bxor", "bnot", "band_not", "bor_not",
    "bxor_not", "ishl", "ushr", "sshr", "rotl", "rotr", "iabs", "umin", "umax", "smin", "smax", "clz", "ctz", "popcnt",
];

pub struct Filetest {
    pub file_name: String,
    pub text: String,
}

enum Tree {
    Param(usize),
    Const(i64),
    Op(String, Vec<Tree>),
}

#[derive(Default)]
struct Instantiate {
    params: HashMap<String, usize>,
    count: usize,
    consts: HashMap<String, i64>,
    next_const: i64,
}

impl Instantiate {
    fn param(&mut self) -> Tree {
        self.count += 1;
        Tree::Param(self.count - 1)
    }

    fn fresh_const(&mut self) -> i64 {
        self.next_const += 2;
        self.next_const
    }

    fn tree(&mut self, shape: &Shape, binding: bool) -> Option<Tree> {
        match shape {
            Shape::Wild if binding => Some(self.param()),
            Shape::Int(text) => text.parse().ok().map(Tree::Const),
            Shape::Var(name) => {
                if let Some(&value) = self.consts.get(name) {
                    return Some(Tree::Const(value));
                }
                if let Some(&index) = self.params.get(name) {
                    return Some(Tree::Param(index));
                }
                if !binding {
                    return None;
                }
                let param = self.param();
                if let Tree::Param(index) = param {
                    self.params.insert(name.clone(), index);
                }
                Some(param)
            }
            Shape::Call(head, args) if head.contains("iconst") => match args.last()? {
                Shape::Int(text) => text.parse().ok().map(Tree::Const),
                Shape::Var(name) => {
                    if let Some(&value) = self.consts.get(name) {
                        return Some(Tree::Const(value));
                    }
                    if !binding {
                        return None;
                    }
                    let value = self.fresh_const();
                    self.consts.insert(name.clone(), value);
                    Some(Tree::Const(value))
                }
                Shape::Wild if binding => Some(Tree::Const(self.fresh_const())),
                _ => None,
            },
            Shape::Call(head, args) => {
                if !OPCODES.contains(&head.as_str()) {
                    return None;
                }
                let typed = matches!(args.first(), Some(Shape::Var(name)) if name.starts_with("ty"));
                let operands = args.iter().skip(usize::from(typed));
                let kids = operands.map(|arg| self.tree(arg, binding)).collect::<Option<Vec<_>>>()?;
                Some(Tree::Op(head.clone(), kids))
            }
            _ => None,
        }
    }
}

fn emit(tree: &Tree, lines: &mut Vec<String>, next: &mut usize) -> String {
    match tree {
        Tree::Param(index) => format!("v{index}"),
        Tree::Const(value) => {
            let name = format!("v{next}");
            *next += 1;
            lines.push(format!("    {name} = iconst.i32 {value}"));
            name
        }
        Tree::Op(op, kids) => {
            let operands: Vec<String> = kids.iter().map(|kid| emit(kid, lines, next)).collect();
            let name = format!("v{next}");
            *next += 1;
            lines.push(format!("    {name} = {op} {}", operands.join(", ")));
            name
        }
    }
}

fn expectations(tree: &Tree) -> Option<Vec<String>> {
    let mut checks = Vec::new();
    match tree {
        Tree::Param(index) => checks.push(format!("return v{index}")),
        Tree::Const(value) => {
            checks.push(format!("$(c0=v[0-9]+) = iconst.i32 {value}"));
            checks.push("return $c0".to_owned());
        }
        Tree::Op(op, kids) => {
            let mut operands: Vec<String> = Vec::new();
            let mut captured: Vec<(i64, String)> = Vec::new();
            for kid in kids {
                match kid {
                    Tree::Param(index) => operands.push(format!("v{index}")),
                    Tree::Const(value) => {
                        let known = captured.iter().find(|(seen, _)| seen == value).map(|(_, name)| name.clone());
                        let name = match known {
                            Some(name) => name,
                            None => {
                                let name = format!("$c{}", captured.len());
                                checks.push(format!("$(c{}=v[0-9]+) = iconst.i32 {value}", captured.len()));
                                captured.push((*value, name.clone()));
                                name
                            }
                        };
                        operands.push(name);
                    }
                    Tree::Op(..) => return None,
                }
            }
            if captured.len() > 1 {
                return None;
            }
            if lints::COMMUTATIVE.contains(&op.as_str()) && operands.len() == 2 {
                if captured.is_empty() {
                    operands = vec![format!("$(a={0}, {1}|{1}, {0})", operands[0], operands[1])];
                } else if operands[0].starts_with("$c") {
                    operands.swap(0, 1);
                }
            }
            checks.push(format!("$(r=v[0-9]+) = {op} {}", operands.join(", ")));
            checks.push("return $r".to_owned());
        }
    }
    Some(checks)
}

fn identifier(text: &str) -> String {
    text.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect()
}

pub fn generate(loaded: &Loaded, file: usize, offset: usize) -> Option<Filetest> {
    let all = rules::rules(loaded);
    let rule = all
        .iter()
        .find(|rule| rule.file == file && rule.form.start <= offset && offset <= rule.form.end)
        .filter(|rule| rule.head == "simplify" && rule.args.len() == 1 && rule.clauses.is_empty())?;
    let mut context = Instantiate::default();
    let lhs = context.tree(&rules::shape(rule.text, &rule.args[0]), true)?;
    if !matches!(lhs, Tree::Op(..)) {
        return None;
    }
    let rhs = context.tree(&rules::shape(rule.text, rule.rhs), false)?;
    let checks = expectations(&rhs)?;

    let mut lines = Vec::new();
    let mut next = context.count;
    let result = emit(&lhs, &mut lines, &mut next);
    let params: Vec<String> = (0..context.count).map(|index| format!("v{index}: i32")).collect();
    let signature = vec!["i32"; context.count].join(", ");
    let source = &loaded.sources[file];
    let ordinal = all.iter().filter(|other| other.file == file && other.form.start <= rule.form.start).count();
    let stem = source.path.file_stem().and_then(|stem| stem.to_str()).unwrap_or("rules");
    let title = match rule.name {
        Some(name) => identifier(name.text(rule.text)),
        None => format!("{}_rule{ordinal}", identifier(stem)),
    };
    let file_name = format!("{title}.clif");

    let mut text = String::from("test optimize\nset opt_level=speed\ntarget x86_64\n\n");
    text.push_str(&format!("function %{title}({signature}) -> i32 {{\nblock0({}):\n", params.join(", ")));
    for line in lines {
        text.push_str(&line);
        text.push('\n');
    }
    text.push_str(&format!("    return {result}\n"));
    for check in checks {
        text.push_str(&format!("    ; check: {check}\n"));
    }
    text.push_str("}\n");
    Some(Filetest { file_name, text })
}
