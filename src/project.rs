use std::{env, fs};
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use directories::ProjectDirs;
use serde::Deserialize;

use crate::parser;
use crate::syntax;

pub const CONFIG_NAME: &str = "isle-lsp.toml";
const CRANELIFT_VERSION: &str = "0.136.2";

#[derive(Debug, Deserialize)]
struct Config {
    #[serde(default)]
    set: Vec<SetConfig>,
    #[serde(default)]
    lints: LintConfig,
}

#[derive(Debug, Default, Deserialize)]
struct LintConfig {
    #[serde(default)]
    disable: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct SetConfig {
    name: Option<String>,
    files: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    Config(PathBuf),
    CraneliftOpt,
    CraneliftLower,
    Siblings,
    Single,
}

#[derive(Clone, Debug)]
pub struct Project {
    pub origin: Origin,
    pub name: Option<String>,
    pub files: Vec<PathBuf>,
    pub notes: Vec<String>,
    pub incomplete: bool,
    pub disabled_lints: Vec<String>,
}

pub struct Source {
    pub path: PathBuf,
    pub text: String,
    pub file: syntax::File,
}

pub struct Loaded {
    pub sources: Vec<Source>,
    pub missing: Vec<PathBuf>,
}

impl Project {
    pub fn load(&self, open: impl Fn(&Path) -> Option<String>) -> Loaded {
        let mut loaded = Loaded { sources: Vec::new(), missing: Vec::new() };
        for path in &self.files {
            match open(path).or_else(|| fs::read_to_string(path).ok()) {
                Some(text) => {
                    let file = parser::parse(&text);
                    loaded.sources.push(Source { path: path.clone(), text, file });
                }
                None => loaded.missing.push(path.clone()),
            }
        }
        loaded
    }
}

pub fn discover(file: &Path, boundary: Option<&Path>) -> Project {
    let mut project = find(file, boundary);
    project.disabled_lints = disabled_lints(&normalize(file), boundary);
    project
}

fn disabled_lints(file: &Path, boundary: Option<&Path>) -> Vec<String> {
    let dir = file.parent().unwrap_or(Path::new(""));
    for ancestor in dir.ancestors() {
        if boundary.is_some_and(|b| !ancestor.starts_with(b)) {
            break;
        }
        let config = ancestor.join(CONFIG_NAME);
        if config.is_file() {
            let parsed = fs::read_to_string(config).ok().and_then(|text| toml::from_str::<Config>(&text).ok());
            return parsed.map(|config| config.lints.disable).unwrap_or_default();
        }
    }
    Vec::new()
}

fn find(file: &Path, boundary: Option<&Path>) -> Project {
    let file = normalize(file);
    let mut notes = Vec::new();
    let dir = file.parent().unwrap_or(Path::new("")).to_owned();
    for ancestor in dir.ancestors() {
        if boundary.is_some_and(|b| !ancestor.starts_with(b)) {
            break;
        }
        let config = ancestor.join(CONFIG_NAME);
        if !config.is_file() {
            continue;
        }
        match from_config(&config, &file) {
            Ok(Some(project)) => return project,
            Ok(None) => {
                notes.push(format!("{CONFIG_NAME} does not list this file"));
                return Project { origin: Origin::Single, name: None, files: vec![file], notes, incomplete: false, disabled_lints: Vec::new() };
            }
            Err(note) => notes.push(note),
        }
        break;
    }
    let mut project = cranelift_opt(&file, boundary).or_else(|| cranelift_lower(&file, boundary)).unwrap_or_else(|| siblings(&file));
    notes.append(&mut project.notes);
    project.notes = notes;
    project
}

fn from_config(config: &Path, file: &Path) -> Result<Option<Project>, String> {
    let text = fs::read_to_string(config).map_err(|e| format!("{CONFIG_NAME}: {e}"))?;
    let parsed: Config = toml::from_str(&text).map_err(|e| format!("{CONFIG_NAME}: {}", e.message()))?;
    let base = config.parent().unwrap_or(Path::new(""));
    for set in parsed.set {
        let files: Vec<PathBuf> = set.files.iter().flat_map(|pattern| expand(base, pattern)).collect();
        if files.contains(&file.to_owned()) {
            return Ok(Some(Project {
                origin: Origin::Config(config.to_owned()),
                name: set.name,
                files,
                notes: Vec::new(),
                incomplete: false,
                disabled_lints: Vec::new(),
            }));
        }
    }
    Ok(None)
}

fn expand(base: &Path, pattern: &str) -> Vec<PathBuf> {
    let path = normalize(&base.join(pattern));
    let Some(name) = path.file_name().and_then(|n| n.to_str()).filter(|n| n.contains('*')) else {
        return vec![path];
    };
    let mut found = list_dir(path.parent().unwrap_or(Path::new("")), |n| wildcard(name, n));
    found.sort();
    found
}

fn wildcard(pattern: &str, text: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == text,
        Some((head, rest)) => {
            let Some(after) = text.strip_prefix(head) else { return false };
            (0..=after.len()).filter(|&i| after.is_char_boundary(i)).any(|i| wildcard(rest, &after[i..]))
        }
    }
}

fn list_dir(dir: &Path, keep: impl Fn(&str) -> bool) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_str().is_some_and(&keep))
        .map(|e| e.path())
        .collect()
}

fn is_isle(name: &str) -> bool {
    name.ends_with(".isle")
}

fn siblings(file: &Path) -> Project {
    let dir = file.parent().unwrap_or(Path::new(""));
    let mut files = list_dir(dir, is_isle);
    if !files.iter().any(|f| f == file) {
        files.push(file.to_owned());
    }
    files.sort();
    let origin = if files.len() == 1 { Origin::Single } else { Origin::Siblings };
    Project { origin, name: None, files, notes: Vec::new(), incomplete: false, disabled_lints: Vec::new() }
}

fn cranelift_opt(file: &Path, boundary: Option<&Path>) -> Option<Project> {
    let opts = file.parent().filter(|d| d.file_name().is_some_and(|n| n == "opts"))?;
    let src = opts.parent()?;
    let preludes = [src.join("prelude.isle"), src.join("prelude_opt.isle")];
    if !preludes.iter().all(|p| p.is_file()) {
        return None;
    }
    let mut rules = list_dir(opts, is_isle);
    rules.sort();
    let mut files = preludes.to_vec();
    files.extend(rules);
    Some(with_generated(Origin::CraneliftOpt, "opt".to_owned(), files, file, src, "clif_opt.isle", boundary))
}

fn cranelift_lower(file: &Path, boundary: Option<&Path>) -> Option<Project> {
    let arch_dir = file.parent()?;
    let isa_dir = arch_dir.parent().filter(|d| d.file_name().is_some_and(|n| n == "isa"))?;
    let src = isa_dir.parent()?;
    let preludes = [src.join("prelude.isle"), src.join("prelude_lower.isle")];
    if !preludes.iter().all(|p| p.is_file()) {
        return None;
    }
    let mut isa = list_dir(arch_dir, is_isle);
    isa.sort_by_key(|path| {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        (!name.starts_with("inst"), name.to_owned())
    });
    let mut files = preludes.to_vec();
    files.extend(isa);
    let arch = arch_dir.file_name()?.to_string_lossy();
    Some(with_generated(Origin::CraneliftLower, format!("lower {arch}"), files, file, src, "clif_lower.isle", boundary))
}

fn with_generated(
    origin: Origin,
    name: String,
    mut files: Vec<PathBuf>,
    file: &Path,
    src: &Path,
    clif: &str,
    boundary: Option<&Path>,
) -> Project {
    let mut notes = Vec::new();
    let generated = src
        .parent()
        .and_then(|crate_dir| generated_isle(crate_dir, clif, boundary))
        .or_else(|| bundled_isle(clif));
    let incomplete = generated.is_none();
    match generated {
        Some(generated) => files.extend(generated),
        None => notes.push(format!("generated numerics.isle and {clif} not found, build cranelift-codegen or list them in isle-lsp.toml")),
    }
    if !files.iter().any(|f| f == file) {
        files.push(file.to_owned());
    }
    Project { origin, name: Some(name), files, notes, incomplete, disabled_lints: Vec::new() }
}

pub fn bundled_dir() -> PathBuf {
    let base = ProjectDirs::from("", "", "isle-lsp").map_or_else(|| env::temp_dir().join("isle-lsp"), |dirs| dirs.cache_dir().to_owned());
    base.join(format!("cranelift-{CRANELIFT_VERSION}"))
}

fn bundled_isle(clif: &str) -> Option<Vec<PathBuf>> {
    let dir = bundled_dir();
    let wanted = vec![dir.join("numerics.isle"), dir.join(clif)];
    if !wanted.iter().all(|p| p.is_file()) {
        let staging = dir.with_extension(format!("tmp{}", std::process::id()));
        fs::create_dir_all(&staging).ok()?;
        let generated = cranelift_codegen_meta::generate_isle(&staging).is_ok();
        let published = generated && fs::rename(&staging, &dir).is_ok();
        if !published {
            let _ = fs::remove_dir_all(&staging);
        }
    }
    wanted.iter().all(|p| p.is_file()).then_some(wanted)
}

fn generated_isle(crate_dir: &Path, clif: &str, boundary: Option<&Path>) -> Option<Vec<PathBuf>> {
    let mut newest: Option<(SystemTime, PathBuf)> = None;
    for ancestor in crate_dir.ancestors().take_while(|a| boundary.is_none_or(|b| a.starts_with(b))) {
        for profile in list_dir(&ancestor.join("target"), |_| true) {
            let mut stack = vec![(profile.join("build"), 0)];
            while let Some((dir, depth)) = stack.pop() {
                let out = dir.join("out");
                let clif_path = out.join(clif);
                if out.join("numerics.isle").is_file() && clif_path.is_file() {
                    let modified = fs::metadata(&clif_path).and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
                    if newest.as_ref().is_none_or(|(t, _)| modified > *t) {
                        newest = Some((modified, out));
                    }
                }
                if depth < 2 {
                    let keep = |n: &str| depth > 0 || n.starts_with("cranelift-codegen");
                    stack.extend(list_dir(&dir, keep).into_iter().filter(|p| p.is_dir()).map(|p| (p, depth + 1)));
                }
            }
        }
    }
    let (_, out) = newest?;
    Some(vec![out.join("numerics.isle"), out.join(clif)])
}

pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other),
        }
    }
    out
}
