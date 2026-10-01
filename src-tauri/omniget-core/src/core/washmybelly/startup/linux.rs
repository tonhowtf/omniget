use std::collections::HashMap;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Desktop {
    pub name: Option<String>,
    pub exec: Option<String>,
    pub hidden: bool,
    pub gnome_enabled: Option<bool>,
    pub only_show_in: Vec<String>,
    pub not_show_in: Vec<String>,
    pub session_part: bool,
}

impl Desktop {
    pub fn enabled(&self) -> bool {
        !self.hidden && self.gnome_enabled != Some(false)
    }

    pub fn applies_to(&self, current: &str) -> bool {
        let cur: Vec<String> = current
            .split(':')
            .filter(|s| !s.is_empty())
            .map(str::to_ascii_lowercase)
            .collect();
        if cur.is_empty() {
            return true;
        }
        let has = |list: &[String]| list.iter().any(|d| cur.contains(&d.to_ascii_lowercase()));
        (self.only_show_in.is_empty() || has(&self.only_show_in)) && !has(&self.not_show_in)
    }
}

fn truthy(v: &str) -> bool {
    matches!(v.trim(), "true" | "1")
}

pub fn parse_desktop(text: &str) -> Desktop {
    let mut d = Desktop::default();
    let mut in_entry = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_entry = t == "[Desktop Entry]";
            continue;
        }
        if !in_entry || t.starts_with('#') {
            continue;
        }
        let Some((k, v)) = t.split_once('=') else {
            continue;
        };
        let v = v.trim();
        let list = || {
            v.split(';')
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        };
        match k.trim() {
            "Name" => d.name = Some(v.to_string()),
            "Exec" => d.exec = Some(v.to_string()),
            "Hidden" => d.hidden = truthy(v),
            "X-GNOME-Autostart-enabled" => d.gnome_enabled = Some(truthy(v)),
            "OnlyShowIn" => d.only_show_in = list(),
            "NotShowIn" => d.not_show_in = list(),
            "X-GNOME-Autostart-Phase" | "X-GNOME-AutoRestart" | "X-KDE-autostart-phase" => {
                d.session_part = true
            }
            _ => {}
        }
    }
    d
}

const DESKTOP_PARTS: &[&str] = &[
    "org.gnome.",
    "gnome-",
    "gsd-",
    "org.kde.",
    "kde",
    "plasma",
    "xdg-user-dirs",
    "at-spi",
    "geoclue",
    "pulseaudio",
    "print-applet",
    "snap-userd",
    "ubuntu-",
    "im-launch",
    "mate-",
    "xfce",
    "xfsettingsd",
    "lxqt",
    "lxpolkit",
    "polkit",
    "spice-vdagent",
    "tracker-",
    "localsearch",
    "evolution-",
    "orca",
    "caribou",
    "zeitgeist",
    "blueman",
    "nm-applet",
    "pipewire",
    "wireplumber",
    "baloo",
    "kaccess",
    "user-dirs",
    "gnome-keyring",
    "cinnamon",
    "budgie",
    "deepin",
    "elementary",
    "io.elementary.",
];

pub fn is_desktop_part(file: &str, d: &Desktop) -> bool {
    let f = file.to_ascii_lowercase();
    d.session_part || DESKTOP_PARTS.iter().any(|p| f.starts_with(p))
}

pub fn parse_unit_files(text: &str) -> Vec<(String, String, Option<String>)> {
    text.lines()
        .filter_map(|l| {
            let mut c = l.split_whitespace();
            let unit = c.next()?;
            let state = c.next()?;
            if !unit.contains('.') || unit.ends_with('.') {
                return None;
            }
            Some((
                unit.to_string(),
                state.to_string(),
                c.next().map(str::to_string),
            ))
        })
        .collect()
}

pub fn duration_ms(text: &str) -> Option<u64> {
    let mut total = 0f64;
    let mut any = false;
    for part in text.split_whitespace() {
        let split = part.find(|c: char| !(c.is_ascii_digit() || c == '.'))?;
        let (num, unit) = part.split_at(split);
        let n: f64 = num.parse().ok()?;
        total += n * match unit {
            "us" | "µs" => 0.001,
            "ms" => 1.0,
            "s" => 1000.0,
            "min" => 60_000.0,
            "h" => 3_600_000.0,
            _ => return None,
        };
        any = true;
    }
    any.then(|| total.round() as u64)
}

pub fn parse_blame(text: &str) -> HashMap<String, u64> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            let i = l.rfind(' ')?;
            let (time, unit) = (&l[..i], l[i + 1..].trim());
            Some((unit.to_string(), duration_ms(time)?))
        })
        .collect()
}

#[cfg(target_os = "linux")]
pub use live::collect;

#[cfg(target_os = "linux")]
mod live {
    use std::path::{Path, PathBuf};

    use super::super::actions::Action;
    use super::super::{file_time, looks_like_updater, Collected, Env, Impact, StartupItem, Sys};
    use super::*;

    fn desktop_files(dir: &Path) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|e| e == "desktop"))
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }

    pub fn collect(sys: &dyn Sys, env: &Env, include_system: bool) -> Collected {
        let mut notes = Vec::new();
        let mut items: Vec<StartupItem> = Vec::new();
        let current = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();

        let mut files: Vec<(String, Option<PathBuf>, Option<PathBuf>)> = Vec::new();
        for p in desktop_files(&env.autostart_user) {
            let f = p
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            files.push((f, Some(p), None));
        }
        for dir in &env.autostart_system {
            for p in desktop_files(dir) {
                let f = p
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                match files.iter_mut().find(|e| e.0 == f) {
                    Some(e) if e.2.is_none() => e.2 = Some(p),
                    Some(_) => {}
                    None => files.push((f, None, Some(p))),
                }
            }
        }
        for (file, user, system) in files {
            let Some(effective) = user.clone().or(system.clone()) else {
                continue;
            };
            let d = parse_desktop(&std::fs::read_to_string(&effective).unwrap_or_default());
            let part = system.is_some() && is_desktop_part(&file, &d);
            if part && !include_system {
                continue;
            }
            let ok_name = super::super::actions::check_desktop_file(&file).is_ok();
            let exec = d.exec.clone().unwrap_or_default();
            items.push(StartupItem {
                id: format!("startup:autostart:{file}"),
                name: d
                    .name
                    .clone()
                    .unwrap_or_else(|| file.trim_end_matches(".desktop").to_string()),
                kind: "autostart".into(),
                scope: if system.is_some() {
                    "system".into()
                } else {
                    "user".into()
                },
                location: effective.to_string_lossy().into_owned(),
                command: exec.clone(),
                enabled: d.enabled(),
                installed: file_time(&effective),
                third_party: !part,
                recommended: !part && looks_like_updater(&format!("{file} {exec}")),
                can_disable: ok_name,
                can_enable: ok_name,
                reason: if !d.applies_to(&current) {
                    Some("other_desktop".into())
                } else if part {
                    Some("desktop_part".into())
                } else {
                    None
                },
                off: ok_name.then(|| Action::XdgAutostart {
                    file: file.clone(),
                    enable: false,
                }),
                on: ok_name.then(|| Action::XdgAutostart {
                    file: file.clone(),
                    enable: true,
                }),
                ..Default::default()
            });
        }

        let blame = sys
            .run(&argv_array![
                "systemd-analyze",
                "--user",
                "blame",
                "--no-pager"
            ])
            .ok()
            .filter(|o| o.ok)
            .map(|o| parse_blame(&o.stdout))
            .unwrap_or_default();
        match sys.run(&argv_array![
            "systemctl",
            "--user",
            "list-unit-files",
            "--state=enabled",
            "--no-legend",
            "--no-pager"
        ]) {
            Ok(o) if o.ok => {
                for (unit, _state, preset) in parse_unit_files(&o.stdout) {
                    if unit.contains("@.")
                        || !(unit.ends_with(".service") || unit.ends_with(".timer"))
                    {
                        continue;
                    }
                    let part = preset.as_deref() == Some("enabled");
                    if part && !include_system {
                        continue;
                    }
                    let active = sys
                        .run(&argv_array![
                            "systemctl",
                            "--user",
                            "is-active",
                            "--",
                            unit.as_str()
                        ])
                        .map(|o| o.stdout.trim() == "active")
                        .ok();
                    items.push(StartupItem {
                        id: format!("startup:user_service:{unit}"),
                        name: unit.trim_end_matches(".service").to_string(),
                        kind: "user_service".into(),
                        scope: "user".into(),
                        location: unit.clone(),
                        command: unit.clone(),
                        enabled: true,
                        running: active,
                        impact: blame.get(&unit).map(|ms| Impact::from_ms(*ms)),
                        third_party: !part,
                        recommended: !part && looks_like_updater(&unit),
                        can_disable: true,
                        can_enable: true,
                        reason: part.then(|| "desktop_part".into()),
                        off: Some(Action::SystemdUser {
                            unit: unit.clone(),
                            enable: false,
                            now: true,
                        }),
                        on: Some(Action::SystemdUser {
                            unit: unit.clone(),
                            enable: true,
                            now: true,
                        }),
                        ..Default::default()
                    });
                }
            }
            Ok(o) => notes.push(format!("user_services_unavailable: {}", o.stderr.trim())),
            Err(e) => notes.push(format!("user_services_unavailable: {e}")),
        }
        Collected { items, notes }
    }
}
