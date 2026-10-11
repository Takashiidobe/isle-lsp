use std::panic::{self, AssertUnwindSafe};

use cranelift_isle::codegen::{self, CodegenOptions};

use crate::compiled::Compiled;
use crate::html::{self, escape};
use crate::view::{SCHEMA, Section, View};

const CSS: &str = r#"
.code { overflow: auto; border: 1px solid var(--border); padding: 8px 0; }
.code div { white-space: pre; padding: 0 12px; min-height: var(--row); }
.code .rule { background: var(--leaf); font-weight: 600; }
.code .doc { color: var(--muted); }
.code .sig { background: var(--match); }
"#;

fn generate(compiled: &Compiled) -> Result<String, String> {
    let options = CodegenOptions {
        exclude_global_allow_pragmas: true,
        prefixes: Vec::new(),
        emit_logging: false,
        split_match_arms: false,
        match_arm_split_threshold: None,
    };
    panic::catch_unwind(AssertUnwindSafe(|| {
        codegen::codegen(compiled.files.clone(), &compiled.types, &compiled.terms, &compiled.rule_sets, &options)
    }))
    .map_err(|_| "internal error in cranelift-isle while generating Rust".to_owned())
}

fn function(code: &str, term: &str) -> Option<String> {
    let marker = format!("// Generated as internal constructor for term {term}.");
    let lines: Vec<&str> = code.lines().collect();
    let start = lines.iter().position(|line| *line == marker)?;
    let end = lines[start..].iter().position(|line| *line == "}")? + start;
    Some(lines[start..=end].join("\n"))
}

pub fn view(compiled: &Compiled, term: &str) -> Result<View, String> {
    let code = generate(compiled)?;
    let source = function(&code, term).ok_or_else(|| format!("`{term}` has no generated constructor"))?;

    let title = format!("Generated Rust for {term}");
    let mut body = String::from("<div class=\"code mono\">\n");
    let mut rules = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim_start();
        let class = if trimmed.starts_with("// Rule at") {
            rules.push(vec![
                (rules.len() + 1).to_string(),
                trimmed.trim_start_matches("// Rule at ").trim_end_matches('.').to_owned(),
            ]);
            "rule"
        } else if trimmed.starts_with("//") {
            "doc"
        } else if trimmed.starts_with("pub fn") {
            "sig"
        } else {
            ""
        };
        body.push_str(&format!("<div class=\"{class}\">{}</div>", escape(line)));
    }
    body.push_str("</div>\n");
    let page = html::page(
        &title,
        &[("leaf", "rule result, from the rule at that location"), ("match", "function signature")],
        CSS,
        &body,
        "",
    );

    Ok(View {
        schema: SCHEMA,
        view: "rust".to_owned(),
        term: term.to_owned(),
        title,
        sections: vec![
            Section::Text { title: "Rust".to_owned(), text: source },
            Section::Html { title: "Rust page".to_owned(), html: page },
            Section::Table {
                title: "Rules in the function".to_owned(),
                columns: ["#", "location"].map(str::to_owned).to_vec(),
                rows: rules,
            },
        ],
    })
}
