use std::collections::HashMap;
use std::time::{Duration, Instant};

pub const MIN_GAP: Duration = Duration::from_secs(3);

#[derive(Default)]
pub struct AckQueue {
    last_sent: HashMap<String, Instant>,
    pending: HashMap<String, String>,
}

fn newer(a: &str, b: &str) -> bool {
    a.parse::<u64>().unwrap_or(0) > b.parse::<u64>().unwrap_or(0)
}

impl AckQueue {
    pub fn offer(&mut self, channel: &str, message: &str, now: Instant) -> Option<String> {
        if let Some(p) = self.pending.get(channel) {
            if !newer(message, p) {
                return None;
            }
        }
        match self.last_sent.get(channel) {
            Some(at) if now.duration_since(*at) < MIN_GAP => {
                self.pending
                    .insert(channel.to_string(), message.to_string());
                None
            }
            _ => {
                self.pending.remove(channel);
                self.last_sent.insert(channel.to_string(), now);
                Some(message.to_string())
            }
        }
    }

    pub fn due(&mut self, now: Instant) -> Vec<(String, String)> {
        let ready: Vec<String> = self
            .pending
            .keys()
            .filter(|c| {
                self.last_sent
                    .get(*c)
                    .is_none_or(|at| now.duration_since(*at) >= MIN_GAP)
            })
            .cloned()
            .collect();
        let mut out = Vec::new();
        for c in ready {
            if let Some(m) = self.pending.remove(&c) {
                self.last_sent.insert(c.clone(), now);
                out.push((c, m));
            }
        }
        if self.last_sent.len() > 256 {
            self.last_sent
                .retain(|_, at| now.duration_since(*at) < MIN_GAP * 4);
        }
        out
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_most_one_ack_per_channel_every_three_seconds() {
        let t0 = Instant::now();
        let mut q = AckQueue::default();
        assert_eq!(q.offer("c", "10", t0).as_deref(), Some("10"));
        assert_eq!(q.offer("c", "11", t0 + Duration::from_secs(1)), None);
        assert_eq!(q.offer("c", "12", t0 + Duration::from_secs(2)), None);
        assert_eq!(q.offer("c", "9", t0 + Duration::from_secs(2)), None);
        assert_eq!(
            q.offer("other", "5", t0 + Duration::from_secs(2))
                .as_deref(),
            Some("5")
        );
        assert!(q.due(t0 + Duration::from_millis(2500)).is_empty());
        assert_eq!(
            q.due(t0 + Duration::from_secs(3)),
            vec![("c".to_string(), "12".to_string())]
        );
        assert!(!q.has_pending());
        assert_eq!(q.offer("c", "13", t0 + Duration::from_secs(4)), None);
        assert_eq!(
            q.offer("c", "14", t0 + Duration::from_secs(7)).as_deref(),
            Some("14")
        );
        assert!(!q.has_pending());
    }
}
