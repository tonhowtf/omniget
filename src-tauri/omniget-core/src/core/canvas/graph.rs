use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    pub ask_id: String,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Default, Clone)]
pub struct WaitGraph {
    edges: HashMap<String, Edge>,
}

impl WaitGraph {
    pub fn add(&mut self, ask_id: &str, from: &str, to: &str) {
        self.edges.insert(
            ask_id.to_string(),
            Edge {
                ask_id: ask_id.to_string(),
                from: from.to_string(),
                to: to.to_string(),
            },
        );
    }

    pub fn remove(&mut self, ask_id: &str) -> Option<Edge> {
        self.edges.remove(ask_id)
    }

    pub fn contains(&self, ask_id: &str) -> bool {
        self.edges.contains_key(ask_id)
    }

    pub fn is_waiting(&self, node: &str) -> bool {
        self.edges.values().any(|e| e.from == node)
    }

    pub fn waiting_on(&self, node: &str) -> Vec<Edge> {
        let mut v: Vec<Edge> = self
            .edges
            .values()
            .filter(|e| e.from == node)
            .cloned()
            .collect();
        v.sort_by(|a, b| a.ask_id.cmp(&b.ask_id));
        v
    }

    pub fn ask_back(&self, from: &str, to: &str) -> Option<String> {
        let mut v: Vec<&Edge> = self
            .edges
            .values()
            .filter(|e| e.from == to && e.to == from)
            .collect();
        v.sort_by(|a, b| a.ask_id.cmp(&b.ask_id));
        v.first().map(|e| e.ask_id.clone())
    }

    pub fn path(&self, start: &str, goal: &str) -> Option<Vec<String>> {
        let mut stack: Vec<(String, Vec<String>)> =
            vec![(start.to_string(), vec![start.to_string()])];
        let mut seen: HashSet<String> = HashSet::new();
        while let Some((node, path)) = stack.pop() {
            if node == goal {
                return Some(path);
            }
            if !seen.insert(node.clone()) {
                continue;
            }
            let mut next: Vec<&Edge> = self.edges.values().filter(|e| e.from == node).collect();
            next.sort_by(|a, b| b.ask_id.cmp(&a.ask_id));
            for e in next {
                let mut p = path.clone();
                p.push(e.to.clone());
                stack.push((e.to.clone(), p));
            }
        }
        None
    }

    pub fn cycle_release(&self, from: &str, to: &str) -> Option<Edge> {
        let path = self.path(to, from)?;
        if path.len() < 2 {
            return None;
        }
        if path.len() == 2 {
            return None;
        }
        let next = &path[1];
        let mut v: Vec<&Edge> = self
            .edges
            .values()
            .filter(|e| e.from == to && &e.to == next)
            .collect();
        v.sort_by(|a, b| a.ask_id.cmp(&b.ask_id));
        v.first().map(|e| (*e).clone())
    }
}
