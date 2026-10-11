use std::collections::{BTreeSet, VecDeque};

use cranelift_isle::trie_again::{Binding, Overlap};

use crate::compiled::Compiled;
use crate::html::{self, escape};
use crate::view::{SCHEMA, Section, View};

const CHAR_WIDTH: usize = 8;
const LINE_HEIGHT: usize = 20;
const COLUMN_GAP: usize = 40;
const LAYER_GAP: usize = 64;
const MARGIN: usize = 24;
const LOOP_ROOM: usize = 90;
const OVERLAP_LIMIT: usize = 500;

const CSS: &str = r#"
.graph { position: relative; }
.graph svg { position: absolute; left: 0; top: 0; }
.graph path.edge { fill: none; stroke: var(--line); stroke-width: 1.4; marker-end: url(#arrow); }
.graph path.edge.cycle { stroke: var(--hl); stroke-dasharray: 5 3; marker-end: url(#arrow-cycle); }
.graph marker path { fill: var(--line); }
.graph marker.cycle path { fill: var(--hl); }
.term { position: absolute; box-sizing: border-box; border: 1px solid var(--border); background: var(--bg); padding: 4px 8px; white-space: pre; overflow: hidden; line-height: 20px; }
.term b { display: block; }
.term.cycle { background: var(--loop); }
.term.focus { border-left: 4px solid var(--hl); }
.scroll { overflow: auto; }
"#;

struct Term {
    name: String,
    rules: usize,
    declared_rec: bool,
}

struct Graph {
    terms: Vec<Term>,
    edges: BTreeSet<(usize, usize)>,
}

fn build(compiled: &Compiled) -> Graph {
    let position = |id| compiled.rule_sets.iter().position(|(term, _)| *term == id);
    let terms = compiled
        .rule_sets
        .iter()
        .map(|(id, set)| Term {
            name: compiled.term_name(*id).to_owned(),
            rules: set.rules.len(),
            declared_rec: compiled.terms.terms[id.index()].is_recursive(),
        })
        .collect();
    let mut edges = BTreeSet::new();
    for (from, (_, set)) in compiled.rule_sets.iter().enumerate() {
        for binding in &set.bindings {
            if let Binding::Constructor { term, .. } = binding
                && let Some(to) = position(*term)
            {
                edges.insert((from, to));
            }
        }
    }
    Graph { terms, edges }
}

fn reachable(edges: &BTreeSet<(usize, usize)>, start: usize, reverse: bool) -> BTreeSet<usize> {
    let mut seen = BTreeSet::from([start]);
    let mut queue = VecDeque::from([start]);
    while let Some(at) = queue.pop_front() {
        for &(a, b) in edges {
            let (from, to) = if reverse { (b, a) } else { (a, b) };
            if from == at && seen.insert(to) {
                queue.push_back(to);
            }
        }
    }
    seen
}

fn back_edges(count: usize, edges: &[(usize, usize, bool)]) -> BTreeSet<(usize, usize)> {
    fn visit(at: usize, edges: &[(usize, usize, bool)], state: &mut [u8], back: &mut BTreeSet<(usize, usize)>) {
        state[at] = 1;
        for &(from, to, _) in edges.iter().filter(|e| e.0 == at) {
            match state[to] {
                0 => visit(to, edges, state, back),
                1 => {
                    back.insert((from, to));
                }
                _ => {}
            }
        }
        state[at] = 2;
    }
    let mut state = vec![0u8; count];
    let mut back = BTreeSet::new();
    let roots = (0..count).filter(|&n| edges.iter().all(|e| e.1 != n || e.0 == n));
    for root in roots.chain(0..count).collect::<Vec<_>>() {
        if state[root] == 0 {
            visit(root, edges, &mut state, &mut back);
        }
    }
    back
}

struct Placed {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

fn layers(count: usize, acyclic: &[(usize, usize)]) -> Vec<usize> {
    let mut depth = vec![0; count];
    let mut incoming = vec![0; count];
    for &(_, to) in acyclic {
        incoming[to] += 1;
    }
    let mut ready: VecDeque<usize> = (0..count).filter(|&n| incoming[n] == 0).collect();
    while let Some(at) = ready.pop_front() {
        for &(from, to) in acyclic {
            if from == at {
                depth[to] = depth[to].max(depth[at] + 1);
                incoming[to] -= 1;
                if incoming[to] == 0 {
                    ready.push_back(to);
                }
            }
        }
    }
    depth
}

fn markup(names: &[String], notes: &[String], cycle: &[bool], focus: Option<usize>, depth: &[usize], edges: &[(usize, usize, bool)]) -> String {
    let count = names.len();
    let size = |n: usize| {
        let chars = names[n].chars().count().max(notes[n].chars().count());
        ((chars * CHAR_WIDTH + 2 * CHAR_WIDTH + 4).max(120), 2 * LINE_HEIGHT + 8)
    };
    let last = depth.iter().copied().max().unwrap_or(0);
    let mut rows: Vec<Vec<usize>> = vec![Vec::new(); last + 1];
    for n in 0..count {
        rows[depth[n]].push(n);
    }
    for row in &mut rows {
        row.sort_by(|&a, &b| names[a].cmp(&names[b]));
    }
    let row_width = |row: &[usize]| row.iter().map(|&n| size(n).0).sum::<usize>() + COLUMN_GAP * row.len().saturating_sub(1);
    let widest = rows.iter().map(|row| row_width(row)).max().unwrap_or(0);
    let total_width = widest + 2 * MARGIN + LOOP_ROOM;

    let mut placed: Vec<Option<Placed>> = (0..count).map(|_| None).collect();
    let mut y = MARGIN;
    for row in &rows {
        let mut x = MARGIN + (widest - row_width(row)) / 2;
        for &n in row {
            let (width, height) = size(n);
            placed[n] = Some(Placed { x, y, width, height });
            x += width + COLUMN_GAP;
        }
        y += 2 * LINE_HEIGHT + 8 + LAYER_GAP;
    }
    let total_height = y.saturating_sub(LAYER_GAP) + MARGIN;

    let mut svg = format!("<svg width=\"{total_width}\" height=\"{total_height}\" viewBox=\"0 0 {total_width} {total_height}\">\n");
    svg.push_str("<defs><marker id=\"arrow\" markerWidth=\"9\" markerHeight=\"9\" refX=\"8\" refY=\"4.5\" orient=\"auto\"><path d=\"M0,0 L9,4.5 L0,9 z\"/></marker>");
    svg.push_str("<marker id=\"arrow-cycle\" class=\"cycle\" markerWidth=\"9\" markerHeight=\"9\" refX=\"8\" refY=\"4.5\" orient=\"auto\"><path d=\"M0,0 L9,4.5 L0,9 z\"/></marker></defs>\n");
    for &(from, to, in_cycle) in edges {
        let (Some(a), Some(b)) = (&placed[from], &placed[to]) else { continue };
        let d = if depth[to] > depth[from] {
            let (x1, y1, x2, y2) = (a.x + a.width / 2, a.y + a.height, b.x + b.width / 2, b.y);
            format!("M{x1},{y1} C{x1},{} {x2},{} {x2},{y2}", y1 + LAYER_GAP / 2, y2.saturating_sub(LAYER_GAP / 2))
        } else if from == to {
            let (x, mid) = (a.x + a.width, a.y + a.height / 2);
            format!("M{x},{} C{},{} {},{} {x},{}", mid - 10, x + 55, mid - 35, x + 55, mid + 35, mid + 10)
        } else {
            let (x1, y1, x2, y2) = (a.x + a.width, a.y + a.height / 2, b.x + b.width, b.y + b.height / 2);
            let reach = LOOP_ROOM - 20;
            format!("M{x1},{y1} C{},{y1} {},{y2} {x2},{y2}", x1 + reach, x2 + reach)
        };
        let class = if in_cycle { "edge cycle" } else { "edge" };
        svg.push_str(&format!("<path class=\"{class}\" d=\"{d}\"/>\n"));
    }
    svg.push_str("</svg>\n");

    let mut body = format!("<div class=\"scroll mono\"><div class=\"graph\" style=\"width:{total_width}px;height:{total_height}px\">\n{svg}");
    for n in 0..count {
        let Some(at) = &placed[n] else { continue };
        let mut class = String::from("term");
        if cycle[n] {
            class.push_str(" cycle");
        }
        if focus == Some(n) {
            class.push_str(" focus");
        }
        body.push_str(&format!(
            "<div class=\"{class}\" style=\"left:{}px;top:{}px;width:{}px;height:{}px\"><b>{}</b>{}</div>\n",
            at.x,
            at.y,
            at.width,
            at.height,
            escape(&names[n]),
            escape(&notes[n])
        ));
    }
    body.push_str("</div></div>\n");
    body
}

pub fn view(compiled: &Compiled, term: Option<&str>) -> Result<View, String> {
    let graph = build(compiled);
    let focus = match term {
        Some(name) => Some(graph.terms.iter().position(|t| t.name == name).ok_or_else(|| format!("`{name}` has no rules in this project"))?),
        None => None,
    };

    let reach: Vec<BTreeSet<usize>> = (0..graph.terms.len()).map(|n| reachable(&graph.edges, n, false)).collect();
    let in_cycle = |a: usize, b: usize| reach[b].contains(&a);
    let keep: BTreeSet<usize> = match focus {
        Some(n) => {
            let mut keep = reach[n].clone();
            keep.extend(graph.edges.iter().filter(|&&(_, to)| to == n).map(|&(from, _)| from));
            keep
        }
        None => (0..graph.terms.len()).collect(),
    };
    let order: Vec<usize> = keep.iter().copied().collect();
    let local = |n: usize| order.iter().position(|&o| o == n);

    let kept_edges: Vec<(usize, usize, bool)> = graph
        .edges
        .iter()
        .filter_map(|&(a, b)| Some((local(a)?, local(b)?, in_cycle(a, b))))
        .collect();
    let back = back_edges(order.len(), &kept_edges);
    let acyclic: Vec<(usize, usize)> =
        kept_edges.iter().map(|&(a, b, _)| (a, b)).filter(|edge| !back.contains(edge)).collect();
    let depth = layers(order.len(), &acyclic);

    let names: Vec<String> = order.iter().map(|&n| graph.terms[n].name.clone()).collect();
    let notes: Vec<String> = order
        .iter()
        .map(|&n| {
            let term = &graph.terms[n];
            let rec = if term.declared_rec { ", rec" } else { "" };
            format!("{} {}{rec}", term.rules, if term.rules == 1 { "rule" } else { "rules" })
        })
        .collect();
    let cycle: Vec<bool> = order.iter().map(|&n| graph.edges.iter().any(|&(a, b)| (a == n || b == n) && in_cycle(a, b))).collect();

    let title = match term {
        Some(name) => format!("Term graph around {name}"),
        None => "Term graph".to_owned(),
    };
    let body = markup(&names, &notes, &cycle, focus.and_then(local), &depth, &kept_edges);
    let page = html::page(&title, &[("bg", "term"), ("loop", "in a call cycle (dashed edges)")], CSS, &body, "");

    let rows: Vec<Vec<String>> = order
        .iter()
        .enumerate()
        .map(|(at, &n)| {
            let calls: Vec<&str> = graph.edges.iter().filter(|e| e.0 == n && e.1 != n).map(|e| graph.terms[e.1].name.as_str()).collect();
            vec![
                names[at].clone(),
                graph.terms[n].rules.to_string(),
                if calls.is_empty() { "-".to_owned() } else { calls.join(", ") },
                if cycle[at] { "yes" } else { "no" }.to_owned(),
            ]
        })
        .collect();

    let mut sections = vec![
        Section::Table {
            title: "Terms".to_owned(),
            columns: ["term", "rules", "calls", "in a cycle"].map(str::to_owned).to_vec(),
            rows,
        },
        Section::Html { title: "Graph page".to_owned(), html: page },
    ];
    if let Some(n) = focus {
        sections.push(overlaps(compiled, n));
    }
    Ok(View { schema: SCHEMA, view: "graph".to_owned(), term: term.unwrap_or_default().to_owned(), title, sections })
}

fn overlaps(compiled: &Compiled, term: usize) -> Section {
    let (_, set) = &compiled.rule_sets[term];
    let label = |rule: &cranelift_isle::trie_again::Rule| {
        let location = rule.pos.pretty_print_line(&compiled.files);
        match rule.name {
            Some(name) => format!("{} ({location})", compiled.types.syms[name.index()]),
            None => location,
        }
    };
    let mut rows = Vec::new();
    let mut total = 0;
    for (at, a) in set.rules.iter().enumerate() {
        for b in &set.rules[at + 1..] {
            if a.prio == b.prio {
                continue;
            }
            let Overlap::Yes { subset } = a.may_overlap(b) else { continue };
            total += 1;
            if rows.len() < OVERLAP_LIMIT {
                let (high, low) = if a.prio > b.prio { (a, b) } else { (b, a) };
                rows.push(vec![
                    label(high),
                    high.prio.to_string(),
                    label(low),
                    low.prio.to_string(),
                    if subset { "one matches a subset of the other" } else { "partial overlap" }.to_owned(),
                ]);
            }
        }
    }
    let shown = if total > rows.len() { format!(" (first {} of {total})", rows.len()) } else { String::new() };
    Section::Table {
        title: format!("Overlapping rules at different priorities{shown}"),
        columns: ["wins", "priority", "loses", "priority", "overlap"].map(str::to_owned).to_vec(),
        rows,
    }
}
