use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Mode {
    #[default]
    #[serde(rename = "off")]
    Off,
    #[serde(rename = "30m")]
    Minutes30,
    #[serde(rename = "1h")]
    Hour1,
    #[serde(rename = "2h")]
    Hours2,
    #[serde(rename = "on")]
    UntilOff,
}

pub const MODES: [&str; 5] = ["off", "30m", "1h", "2h", "on"];

impl Mode {
    pub fn parse(raw: &str) -> Option<Mode> {
        match raw {
            "off" => Some(Mode::Off),
            "30m" => Some(Mode::Minutes30),
            "1h" => Some(Mode::Hour1),
            "2h" => Some(Mode::Hours2),
            "on" => Some(Mode::UntilOff),
            _ => None,
        }
    }

    fn duration_ms(self) -> Option<u64> {
        match self {
            Mode::Minutes30 => Some(30 * 60_000),
            Mode::Hour1 => Some(60 * 60_000),
            Mode::Hours2 => Some(120 * 60_000),
            Mode::Off | Mode::UntilOff => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct AwakeView {
    pub mode: Mode,
    pub active: bool,
    pub until_ms: Option<u64>,
    pub remaining_secs: Option<u64>,
}

#[derive(Debug, Default)]
pub struct Awake {
    mode: Mode,
    until_ms: Option<u64>,
}

impl Awake {
    pub fn set(&mut self, mode: Mode, now_ms: u64) {
        self.mode = mode;
        self.until_ms = mode.duration_ms().map(|d| now_ms + d);
    }

    pub fn expire(&mut self, now_ms: u64) -> bool {
        match self.until_ms {
            Some(until) if now_ms >= until => {
                self.set(Mode::Off, now_ms);
                true
            }
            _ => false,
        }
    }

    pub fn active(&self) -> bool {
        self.mode != Mode::Off
    }

    pub fn until_ms(&self) -> Option<u64> {
        self.until_ms
    }

    pub fn view(&self, now_ms: u64) -> AwakeView {
        AwakeView {
            mode: self.mode,
            active: self.active(),
            until_ms: self.until_ms,
            remaining_secs: self
                .until_ms
                .map(|until| until.saturating_sub(now_ms).div_ceil(1000)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_starts_off() {
        let a = Awake::default();
        assert!(!a.active());
        assert_eq!(a.view(0).mode, Mode::Off);
        assert_eq!(a.view(0).remaining_secs, None);
    }

    #[test]
    fn a_timed_mode_counts_down_and_turns_itself_off() {
        let mut a = Awake::default();
        a.set(Mode::Minutes30, 1_000);
        assert!(a.active());
        assert_eq!(a.until_ms(), Some(1_000 + 30 * 60_000));
        assert_eq!(a.view(1_000).remaining_secs, Some(1800));
        assert_eq!(a.view(61_500).remaining_secs, Some(1740));
        assert!(!a.expire(1_000 + 30 * 60_000 - 1));
        assert!(a.active());
        assert!(a.expire(1_000 + 30 * 60_000));
        assert!(!a.active());
        assert_eq!(a.view(0).until_ms, None);
    }

    #[test]
    fn until_turned_off_never_expires() {
        let mut a = Awake::default();
        a.set(Mode::UntilOff, 5);
        assert!(!a.expire(u64::MAX));
        assert!(a.active());
        assert_eq!(a.view(10).remaining_secs, None);
        a.set(Mode::Off, 20);
        assert!(!a.active());
    }

    #[test]
    fn choosing_again_restarts_the_timer() {
        let mut a = Awake::default();
        a.set(Mode::Hours2, 0);
        a.set(Mode::Hour1, 10_000);
        assert_eq!(a.until_ms(), Some(10_000 + 3_600_000));
    }

    #[test]
    fn mode_names_round_trip_and_unknown_names_are_refused() {
        for name in MODES {
            let mode = Mode::parse(name).unwrap();
            assert_eq!(serde_json::to_value(mode).unwrap(), name);
        }
        assert_eq!(Mode::parse("forever"), None);
        assert_eq!(Mode::parse(""), None);
    }
}
