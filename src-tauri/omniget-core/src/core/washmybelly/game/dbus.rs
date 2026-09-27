//! The D-Bus calls of the Linux game mode, built as plain data so the names
//! and the argument order are tested on every system (no daemon needed).
//!
//! Argument order, checked in the gamemode sources (1.8.2, a74b810):
//!
//! - daemon `com.feralinteractive.GameMode`, `RegisterGameByPID(ii)`: the
//!   daemon reads **(caller, game)** (`daemon/gamemode-dbus.c`,
//!   `method_register_game_by_pid`, `sd_bus_message_read(m, "ii",
//!   &callerpid, &gamepid)`). Same for `UnregisterGameByPID` and
//!   `QueryStatusByPID`.
//! - the client library sends `pids[0]=game, pids[1]=getpid()`
//!   (`lib/client_impl.c`, `gamemode_request`), which matches the pidfd
//!   variant and the portal but is the reverse of what the daemon's `ByPID`
//!   reads.
//! - portal `org.freedesktop.portal.GameMode` spells it `RegisterGameByPid`
//!   (lower-case "id") and takes **(target, requester)**
//!   (`xdg-desktop-portal/data/org.freedesktop.portal.GameMode.xml`).
//!
//! So the daemon gets `(self, game)` and the portal gets `(game, self)`.

use serde::Serialize;

pub const GAMEMODE_NAME: &str = "com.feralinteractive.GameMode";
pub const GAMEMODE_PATH: &str = "/com/feralinteractive/GameMode";
pub const GAMEMODE_IFACE: &str = "com.feralinteractive.GameMode";

pub const PORTAL_NAME: &str = "org.freedesktop.portal.Desktop";
pub const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
pub const PORTAL_GAMEMODE_IFACE: &str = "org.freedesktop.portal.GameMode";
pub const PORTAL_INHIBIT_IFACE: &str = "org.freedesktop.portal.Inhibit";

pub const PROFILES_NAME: &str = "org.freedesktop.UPower.PowerProfiles";
pub const PROFILES_PATH: &str = "/org/freedesktop/UPower/PowerProfiles";
pub const PROFILES_IFACE: &str = "org.freedesktop.UPower.PowerProfiles";

pub const SCREENSAVER_NAME: &str = "org.freedesktop.ScreenSaver";
pub const SCREENSAVER_PATH: &str = "/org/freedesktop/ScreenSaver";
pub const SCREENSAVER_IFACE: &str = "org.freedesktop.ScreenSaver";

pub const APP_ID: &str = "wtf.tonho.omniget";
pub const REASON: &str = "A game is running (OmniGet game mode)";

/// `Inhibit` flag of the portal: idle (screen saver and suspend on idle).
pub const PORTAL_INHIBIT_IDLE: u32 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Bus {
    Session,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Arg {
    I32(i32),
    U32(u32),
    Str(String),
}

/// One method call, ready for `zbus::blocking::Connection::call_method`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Call {
    pub bus: Bus,
    pub dest: &'static str,
    pub path: &'static str,
    pub iface: &'static str,
    pub method: &'static str,
    pub args: Vec<Arg>,
}

impl Call {
    /// The D-Bus signature of the arguments (`ii`, `sss`...).
    pub fn signature(&self) -> String {
        self.args
            .iter()
            .map(|a| match a {
                Arg::I32(_) => 'i',
                Arg::U32(_) => 'u',
                Arg::Str(_) => 's',
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Register,
    Unregister,
    Query,
}

fn session(
    dest: &'static str,
    path: &'static str,
    iface: &'static str,
    method: &'static str,
    args: Vec<Arg>,
) -> Call {
    Call {
        bus: Bus::Session,
        dest,
        path,
        iface,
        method,
        args,
    }
}

/// The daemon, asked by OmniGet (`caller`) on behalf of `game`: the
/// `ByPID` form, so `ListGames` shows OmniGet as the requester.
pub fn daemon_by_pid(verb: Verb, caller: i32, game: i32) -> Call {
    let method = match verb {
        Verb::Register => "RegisterGameByPID",
        Verb::Unregister => "UnregisterGameByPID",
        Verb::Query => "QueryStatusByPID",
    };
    session(
        GAMEMODE_NAME,
        GAMEMODE_PATH,
        GAMEMODE_IFACE,
        method,
        vec![Arg::I32(caller), Arg::I32(game)],
    )
}

/// The daemon, one pid used as client and requester.
pub fn daemon_single(verb: Verb, pid: i32) -> Call {
    let method = match verb {
        Verb::Register => "RegisterGame",
        Verb::Unregister => "UnregisterGame",
        Verb::Query => "QueryStatus",
    };
    session(
        GAMEMODE_NAME,
        GAMEMODE_PATH,
        GAMEMODE_IFACE,
        method,
        vec![Arg::I32(pid)],
    )
}

/// The portal (inside Flatpak). The pid is the one this sandbox sees; the
/// portal translates it.
pub fn portal(verb: Verb, pid: i32) -> Call {
    let method = match verb {
        Verb::Register => "RegisterGame",
        Verb::Unregister => "UnregisterGame",
        Verb::Query => "QueryStatus",
    };
    session(
        PORTAL_NAME,
        PORTAL_PATH,
        PORTAL_GAMEMODE_IFACE,
        method,
        vec![Arg::I32(pid)],
    )
}

/// The portal's two-pid form: `(target, requester)`, the reverse of the
/// daemon's `ByPID`.
pub fn portal_by_pid(verb: Verb, game: i32, requester: i32) -> Call {
    let method = match verb {
        Verb::Register => "RegisterGameByPid",
        Verb::Unregister => "UnregisterGameByPid",
        Verb::Query => "QueryStatusByPid",
    };
    session(
        PORTAL_NAME,
        PORTAL_PATH,
        PORTAL_GAMEMODE_IFACE,
        method,
        vec![Arg::I32(game), Arg::I32(requester)],
    )
}

pub fn list_games() -> Call {
    session(
        GAMEMODE_NAME,
        GAMEMODE_PATH,
        GAMEMODE_IFACE,
        "ListGames",
        vec![],
    )
}

/// power-profiles-daemon: hold `performance` until released or until this
/// connection closes. Returns a cookie (`u`).
pub fn hold_profile() -> Call {
    Call {
        bus: Bus::System,
        dest: PROFILES_NAME,
        path: PROFILES_PATH,
        iface: PROFILES_IFACE,
        method: "HoldProfile",
        args: vec![
            Arg::Str("performance".into()),
            Arg::Str(REASON.into()),
            Arg::Str(APP_ID.into()),
        ],
    }
}

pub fn release_profile(cookie: u32) -> Call {
    Call {
        bus: Bus::System,
        dest: PROFILES_NAME,
        path: PROFILES_PATH,
        iface: PROFILES_IFACE,
        method: "ReleaseProfile",
        args: vec![Arg::U32(cookie)],
    }
}

/// `org.freedesktop.ScreenSaver.Inhibit(application, reason) -> cookie`,
/// the same call gamemode makes.
pub fn screensaver_inhibit() -> Call {
    session(
        SCREENSAVER_NAME,
        SCREENSAVER_PATH,
        SCREENSAVER_IFACE,
        "Inhibit",
        vec![Arg::Str("OmniGet".into()), Arg::Str(REASON.into())],
    )
}

pub fn screensaver_uninhibit(cookie: u32) -> Call {
    session(
        SCREENSAVER_NAME,
        SCREENSAVER_PATH,
        SCREENSAVER_IFACE,
        "UnInhibit",
        vec![Arg::U32(cookie)],
    )
}

/// What a gamemode reply code means (`gamemode-context.c`).
pub fn reply_meaning(verb: Verb, code: i32) -> &'static str {
    match (verb, code) {
        (Verb::Query, 0) => "inactive",
        (Verb::Query, 1) => "active_not_this_game",
        (Verb::Query, 2) => "active",
        (_, 0) => "ok",
        (_, -2) => "rejected_by_config",
        _ => "error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_daemon_reads_caller_then_game() {
        let c = daemon_by_pid(Verb::Register, 100, 4242);
        assert_eq!(c.method, "RegisterGameByPID");
        assert_eq!(c.args, vec![Arg::I32(100), Arg::I32(4242)]);
        assert_eq!(c.signature(), "ii");
        assert_eq!(
            (c.dest, c.path, c.iface),
            (GAMEMODE_NAME, GAMEMODE_PATH, GAMEMODE_IFACE)
        );
        assert_eq!(c.bus, Bus::Session);
        assert_eq!(
            daemon_by_pid(Verb::Unregister, 1, 2).method,
            "UnregisterGameByPID"
        );
        assert_eq!(daemon_by_pid(Verb::Query, 1, 2).method, "QueryStatusByPID");
    }

    #[test]
    fn the_portal_reads_target_then_requester_and_spells_pid_lowercase() {
        let c = portal_by_pid(Verb::Register, 4242, 100);
        assert_eq!(c.method, "RegisterGameByPid");
        assert_eq!(c.args, vec![Arg::I32(4242), Arg::I32(100)]);
        assert_eq!(
            (c.dest, c.path, c.iface),
            (PORTAL_NAME, PORTAL_PATH, PORTAL_GAMEMODE_IFACE)
        );
        let d = daemon_by_pid(Verb::Register, 100, 4242);
        // Same request, opposite order: the trap the study warned about.
        assert_eq!(c.args.iter().rev().cloned().collect::<Vec<_>>(), d.args);
    }

    #[test]
    fn single_pid_calls_and_signatures() {
        assert_eq!(daemon_single(Verb::Register, 7).method, "RegisterGame");
        assert_eq!(daemon_single(Verb::Query, 7).signature(), "i");
        assert_eq!(portal(Verb::Unregister, 7).method, "UnregisterGame");
        assert_eq!(portal(Verb::Register, 7).iface, PORTAL_GAMEMODE_IFACE);
        assert_eq!(list_games().signature(), "");
    }

    #[test]
    fn fallback_calls_go_to_the_right_bus() {
        let h = hold_profile();
        assert_eq!(h.bus, Bus::System);
        assert_eq!(h.signature(), "sss");
        assert_eq!(h.args[0], Arg::Str("performance".into()));
        assert_eq!(release_profile(9).args, vec![Arg::U32(9)]);
        let s = screensaver_inhibit();
        assert_eq!(
            (s.bus, s.method, s.signature().as_str()),
            (Bus::Session, "Inhibit", "ss")
        );
        assert_eq!(screensaver_uninhibit(3).signature(), "u");
    }

    #[test]
    fn reply_codes() {
        assert_eq!(reply_meaning(Verb::Register, 0), "ok");
        assert_eq!(reply_meaning(Verb::Register, -2), "rejected_by_config");
        assert_eq!(reply_meaning(Verb::Query, 2), "active");
        assert_eq!(reply_meaning(Verb::Query, 1), "active_not_this_game");
        assert_eq!(reply_meaning(Verb::Unregister, -1), "error");
    }
}
