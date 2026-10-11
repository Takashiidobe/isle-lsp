use serde::Serialize;

use crate::compiled::{self, Compiled};
use crate::graph;
use crate::project::Loaded;
use crate::rustview;
use crate::trie;

pub const SCHEMA: u32 = 1;

#[derive(Serialize)]
pub struct View {
    pub schema: u32,
    pub view: String,
    pub term: String,
    pub title: String,
    pub sections: Vec<Section>,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Section {
    Text { title: String, text: String },
    Html { title: String, html: String },
    Table { title: String, columns: Vec<String>, rows: Vec<Vec<String>> },
    Tree { title: String, nodes: Vec<TreeNode> },
}

#[derive(Serialize)]
pub struct TreeNode {
    pub label: String,
    pub children: Vec<TreeNode>,
}

const VIEWS: [&str; 3] = ["trie", "rust", "graph"];

pub fn render(
    loaded: &Loaded,
    name: impl Fn(&std::path::Path) -> String,
    view: &str,
    term: Option<&str>,
) -> Result<View, String> {
    if !VIEWS.contains(&view) {
        return Err(format!("unknown view `{view}`, expected one of: {}", VIEWS.join(", ")));
    }
    let compiled = compiled::compile(loaded, name)?;
    match view {
        "trie" => trie::view(&compiled, required(&compiled, term)?),
        "rust" => rustview::view(&compiled, required(&compiled, term)?),
        _ => graph::view(&compiled, term),
    }
}

fn required<'a>(compiled: &Compiled, term: Option<&'a str>) -> Result<&'a str, String> {
    let name = term.ok_or_else(|| "this view needs a term: put the cursor on one or pass `term`".to_owned())?;
    compiled.find(name).map(|_| name).ok_or_else(|| format!("`{name}` has no rules in this project"))
}
