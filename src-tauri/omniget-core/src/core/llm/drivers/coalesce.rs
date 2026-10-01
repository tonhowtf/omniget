use std::time::{Duration, Instant};

use super::{RuntimeEvent, RuntimeEventKind};

pub const COALESCE_WINDOW: Duration = Duration::from_millis(50);
const MAX_PENDING_BYTES: usize = 64 * 1024;

#[derive(Debug, Default)]
pub struct DeltaCoalescer {
    pending: Vec<RuntimeEvent>,
    bytes: usize,
    since: Option<Instant>,
    window: Duration,
}

fn key(ev: &RuntimeEvent) -> Option<(String, Option<String>, Option<String>, String)> {
    match &ev.kind {
        RuntimeEventKind::ContentDelta(p) => Some((
            ev.thread_id.clone(),
            ev.turn_id.clone(),
            ev.item_id.clone(),
            format!("{:?}", p.stream_kind),
        )),
        RuntimeEventKind::ProposedDelta(_) => Some((
            ev.thread_id.clone(),
            ev.turn_id.clone(),
            ev.item_id.clone(),
            "proposed".into(),
        )),
        _ => None,
    }
}

fn append_delta(into: &mut RuntimeEvent, from: &RuntimeEvent) -> usize {
    match (&mut into.kind, &from.kind) {
        (RuntimeEventKind::ContentDelta(a), RuntimeEventKind::ContentDelta(b)) => {
            a.delta.push_str(&b.delta);
            b.delta.len()
        }
        (RuntimeEventKind::ProposedDelta(a), RuntimeEventKind::ProposedDelta(b)) => {
            a.delta.push_str(&b.delta);
            b.delta.len()
        }
        _ => 0,
    }
}

impl DeltaCoalescer {
    pub fn new() -> Self {
        Self::with_window(COALESCE_WINDOW)
    }

    pub fn with_window(window: Duration) -> Self {
        Self {
            window,
            ..Default::default()
        }
    }

    pub fn push(&mut self, ev: RuntimeEvent, now: Instant) -> Vec<RuntimeEvent> {
        match key(&ev) {
            Some(k) => {
                let mut out = Vec::new();
                let merged = match self.pending.last_mut() {
                    Some(last) if key(last).as_ref() == Some(&k) => {
                        self.bytes += append_delta(last, &ev);
                        true
                    }
                    _ => false,
                };
                if !merged {
                    out.extend(self.take());
                    self.bytes = match &ev.kind {
                        RuntimeEventKind::ContentDelta(p) => p.delta.len(),
                        RuntimeEventKind::ProposedDelta(p) => p.delta.len(),
                        _ => 0,
                    };
                    self.pending.push(ev);
                    self.since = Some(now);
                }
                if self.bytes >= MAX_PENDING_BYTES || self.due(now) {
                    out.extend(self.take());
                }
                out
            }
            None => {
                let mut out = self.take();
                out.push(ev);
                out
            }
        }
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.since.map(|s| s + self.window)
    }

    pub fn due(&self, now: Instant) -> bool {
        self.deadline().map(|d| now >= d).unwrap_or(false)
    }

    pub fn take(&mut self) -> Vec<RuntimeEvent> {
        self.since = None;
        self.bytes = 0;
        std::mem::take(&mut self.pending)
    }
}
