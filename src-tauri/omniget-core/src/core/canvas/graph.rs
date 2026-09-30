//! Wait-for graph of the cords (estudo 98/03 §B.11).
//!
//! An edge `A -> B` exists while A's CLI is blocked in `omniget canvas ask B`.
//! That is what deadlocks: A is busy inside the command and cannot take a
//! message. Two ways out:
//!
//! - **ask back:** B messages A while A waits on B. B's message *is* the
//!   answer to A's ask ([`WaitGraph::ask_back`]); nothing is pasted into A.
//! - **ring break:** admitting X -> Y where Y already (transitively) waits on
//!   X closes a cycle. The ask Y has open on the ring is released
//!   ([`WaitGraph::cycle_release`]) so Y becomes free to read X's message.

use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    pub ask_id: String,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Default, Clone)]
pub struct WaitGraph {
    /// ask id -> edge. Several asks of one node (a batch) are several edges.
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

    /// Asks `node` is blocked on, oldest id first (ids sort by creation).
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

    /// `from` messages `to` while `to` waits on `from`: the ask of `to` that
    /// this message answers.
    pub fn ask_back(&self, from: &str, to: &str) -> Option<String> {
        let mut v: Vec<&Edge> = self
            .edges
            .values()
            .filter(|e| e.from == to && e.to == from)
            .collect();
        v.sort_by(|a, b| a.ask_id.cmp(&b.ask_id));
        v.first().map(|e| e.ask_id.clone())
    }

    /// Path of nodes from `start` to `goal` following wait edges, if any
    /// (`[start, ..., goal]`).
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

    /// Admitting a message `from -> to` (where `to` waits, directly or not,
    /// on `from`) closes a ring. Returns the ask to release: the one `to`
    /// has open on the ring (its first step towards `from`). `None` when
    /// there is no ring, or when it is a direct ask back (see [`ask_back`]).
    pub fn cycle_release(&self, from: &str, to: &str) -> Option<Edge> {
        let path = self.path(to, from)?;
        if path.len() < 2 {
            return None;
        }
        if path.len() == 2 {
            // `to` waits on `from` directly: that is an ask back.
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

    pub fn edges(&self) -> impl Iterator<Item = &Edge> {
        self.edges.values()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_direct_ask_back_resolves_the_waiting_ask() {
        let mut g = WaitGraph::default();
        g.add("ask_1", "A", "B");
        assert_eq!(g.ask_back("B", "A"), Some("ask_1".into()));
        assert_eq!(g.ask_back("C", "A"), None);
        // A direct ask back is not a ring to break.
        assert_eq!(g.cycle_release("B", "A"), None);
    }

    #[test]
    fn a_ring_releases_the_targets_ask_on_the_ring() {
        let mut g = WaitGraph::default();
        g.add("ask_1", "A", "B");
        g.add("ask_2", "B", "C");
        // C asks A: A waits on B, B waits on C -> C would wait on A = ring.
        let rel = g.cycle_release("C", "A").expect("ring");
        assert_eq!(rel.ask_id, "ask_1");
        assert_eq!(rel.from, "A");
        // No ring when nobody waits back.
        assert_eq!(g.cycle_release("A", "D"), None);
    }

    #[test]
    fn edges_leave_with_their_ask() {
        let mut g = WaitGraph::default();
        g.add("ask_1", "A", "B");
        assert!(g.is_waiting("A"));
        g.remove("ask_1");
        assert!(!g.is_waiting("A"));
        assert_eq!(g.path("A", "B"), None);
    }
}
