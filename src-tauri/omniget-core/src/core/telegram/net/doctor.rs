//! Step-by-step connection diagnosis. Each step carries a stable `hint`
//! key the screen turns into a sentence in the person's language, so the
//! answer to "why won't it connect" is never a spinner.

use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;

use super::manager::{Manager, Status};
use super::route::{RouteKind, RouteSource};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    Ok,
    Warn,
    Fail,
    Skip,
}

#[derive(Debug, Clone, Serialize)]
pub struct Step {
    pub id: &'static str,
    pub state: StepState,
    /// i18n key under `telegram.doctor.hints`.
    pub hint: &'static str,
    /// Raw detail (addresses, error text) shown under the sentence.
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub steps: Vec<Step>,
    pub status: Status,
    /// Short overall verdict key under `telegram.doctor.verdicts`.
    pub verdict: &'static str,
}

pub async fn run(mgr: &Arc<Manager>) -> Report {
    let (cn_ok, world_ok) = tokio::join!(
        reach("https://www.baidu.com"),
        reach("https://www.gstatic.com/generate_204")
    );
    let status = mgr.scan().await;
    let mut steps = Vec::new();

    steps.push(Step {
        id: "internet",
        state: if cn_ok || world_ok {
            StepState::Ok
        } else {
            StepState::Fail
        },
        hint: if cn_ok || world_ok {
            "internet_ok"
        } else {
            "internet_down"
        },
        detail: format!(
            "baidu.com: {}, gstatic.com: {}",
            yesno(cn_ok),
            yesno(world_ok)
        ),
    });
    let censored = cn_ok && !world_ok;

    let direct = status
        .routes
        .iter()
        .find(|v| v.route.kind == RouteKind::Direct);
    let direct_ok = direct.and_then(|d| d.health.ok) == Some(true);
    steps.push(Step {
        id: "direct",
        state: if direct_ok {
            StepState::Ok
        } else if censored {
            StepState::Warn
        } else {
            StepState::Fail
        },
        hint: if direct_ok {
            "direct_ok"
        } else if censored {
            "direct_blocked_expected"
        } else {
            "direct_failed"
        },
        detail: direct
            .and_then(|d| d.health.error.clone())
            .unwrap_or_default(),
    });

    let locals: Vec<_> = status
        .routes
        .iter()
        .filter(|v| {
            matches!(
                v.route.source,
                RouteSource::LocalApp { .. }
                    | RouteSource::System
                    | RouteSource::Env { .. }
                    | RouteSource::Clash { .. }
            )
        })
        .collect();
    let local_ok = locals.iter().filter(|v| v.health.ok == Some(true)).count();
    steps.push(Step {
        id: "local_proxy",
        state: if local_ok > 0 {
            StepState::Ok
        } else if locals.is_empty() {
            if direct_ok {
                StepState::Skip
            } else {
                StepState::Fail
            }
        } else {
            StepState::Fail
        },
        hint: if local_ok > 0 {
            "local_proxy_ok"
        } else if locals.is_empty() {
            "local_proxy_none"
        } else {
            "local_proxy_broken"
        },
        detail: locals
            .iter()
            .map(|v| {
                format!(
                    "{} → {}",
                    v.route.name,
                    match v.health.ok {
                        Some(true) => format!("{} ms", v.health.rtt_ms.unwrap_or(0)),
                        _ => v.health.error.clone().unwrap_or_else(|| "?".into()),
                    }
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
    });

    let saved: Vec<_> = status.routes.iter().filter(|v| v.saved).collect();
    let saved_ok = saved.iter().filter(|v| v.health.ok == Some(true)).count();
    steps.push(Step {
        id: "saved",
        state: if saved.is_empty() {
            StepState::Skip
        } else if saved_ok > 0 {
            StepState::Ok
        } else {
            StepState::Fail
        },
        hint: if saved.is_empty() {
            "saved_none"
        } else if saved_ok > 0 {
            "saved_ok"
        } else {
            "saved_all_failed"
        },
        detail: format!("{saved_ok}/{}", saved.len()),
    });

    let rescue: Vec<_> = status
        .routes
        .iter()
        .filter(|v| matches!(&v.route.source, RouteSource::PublicList { list } if list == "telegram-rescue"))
        .collect();
    let rescue_ok = rescue.iter().any(|v| v.health.ok == Some(true));
    steps.push(Step {
        id: "rescue",
        state: if !status.use_rescue {
            StepState::Skip
        } else if rescue_ok {
            StepState::Ok
        } else if rescue.is_empty() {
            StepState::Warn
        } else {
            StepState::Fail
        },
        hint: if rescue_ok {
            "rescue_ok"
        } else if rescue.is_empty() {
            "rescue_unavailable"
        } else {
            "rescue_blocked"
        },
        detail: rescue
            .iter()
            .map(|v| v.route.name.clone())
            .collect::<Vec<_>>()
            .join(", "),
    });

    let off = status.clock_offset_secs.unwrap_or(0);
    steps.push(Step {
        id: "clock",
        state: if status.clock_offset_secs.is_none() {
            StepState::Skip
        } else if off.abs() > 30 {
            StepState::Warn
        } else {
            StepState::Ok
        },
        hint: if off.abs() > 30 {
            "clock_off"
        } else {
            "clock_ok"
        },
        detail: status
            .clock_offset_secs
            .map(|o| format!("{o:+} s"))
            .unwrap_or_default(),
    });

    let any_ok = status.routes.iter().any(|v| v.health.ok == Some(true));
    let verdict = if any_ok {
        "connected"
    } else if !(cn_ok || world_ok) {
        "no_internet"
    } else if censored && locals.is_empty() {
        "need_proxy"
    } else if !locals.is_empty() {
        "proxy_app_not_passing"
    } else {
        "all_routes_failed"
    };
    Report {
        steps,
        status,
        verdict,
    }
}

fn yesno(b: bool) -> &'static str {
    if b {
        "ok"
    } else {
        "—"
    }
}

async fn reach(url: &str) -> bool {
    let Ok(c) = reqwest::Client::builder()
        .use_rustls_tls()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
    else {
        return false;
    };
    c.head(url).send().await.is_ok()
}
