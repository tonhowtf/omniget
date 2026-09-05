//! Autoclicker (estudo 14, Blur AutoClicker — GPL-3): laço de cliques com
//! timer de alta resolução (sleep grosso + espera ativa no último
//! milissegundo), CPS fixo ou em faixa aleatória, botão, clique duplo, limite
//! por cliques ou tempo e posição fixa. Multiplataforma via `enigo`; no macOS
//! exige a permissão de Acessibilidade para o OmniGet, no Wayland o clique
//! sintético é limitado. O atalho global fica no app (hotkey.rs).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::anyhow;
use enigo::{Button, Coordinate, Direction, Enigo, Mouse, Settings};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClickOptions {
    /// cliques por segundo (0,1 a 1000)
    pub cps: f64,
    /// variação aleatória do intervalo, em % (0 = fixo)
    #[serde(default)]
    pub jitter_pct: u8,
    /// "left" | "right" | "middle"
    #[serde(default = "left")]
    pub button: String,
    #[serde(default)]
    pub double: bool,
    /// 0 = sem limite
    #[serde(default)]
    pub max_clicks: u64,
    /// 0 = sem limite
    #[serde(default)]
    pub max_seconds: u64,
    /// posição fixa (senão clica onde o mouse estiver)
    #[serde(default)]
    pub position: Option<(i32, i32)>,
    /// tempo com o botão pressionado, em ms (0 = clique instantâneo)
    #[serde(default)]
    pub hold_ms: u32,
    /// atraso antes de começar, em segundos
    #[serde(default)]
    pub start_delay: u32,
}

fn left() -> String {
    "left".into()
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ClickState {
    pub running: bool,
    pub clicks: u64,
    pub elapsed_ms: u64,
    pub error: Option<String>,
}

static RUNNING: AtomicBool = AtomicBool::new(false);
static CLICKS: AtomicU64 = AtomicU64::new(0);
static STARTED: Mutex<Option<Instant>> = Mutex::new(None);
static ERROR: Mutex<Option<String>> = Mutex::new(None);
static LAST_OPTS: Mutex<Option<ClickOptions>> = Mutex::new(None);

pub fn state() -> ClickState {
    let started = STARTED.lock().unwrap_or_else(|e| e.into_inner());
    ClickState {
        running: RUNNING.load(Ordering::SeqCst),
        clicks: CLICKS.load(Ordering::SeqCst),
        elapsed_ms: started.map(|s| s.elapsed().as_millis() as u64).unwrap_or(0),
        error: ERROR.lock().unwrap_or_else(|e| e.into_inner()).clone(),
    }
}

pub fn last_options() -> Option<ClickOptions> {
    LAST_OPTS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Espera precisa: dorme até faltar 1,5 ms e gira o resto.
fn sleep_until(deadline: Instant) {
    loop {
        let now = Instant::now();
        if now >= deadline {
            return;
        }
        let left = deadline - now;
        if left > Duration::from_micros(1500) {
            std::thread::sleep(left - Duration::from_micros(1200));
        } else {
            std::hint::spin_loop();
        }
    }
}

fn button_of(name: &str) -> Button {
    match name {
        "right" => Button::Right,
        "middle" => Button::Middle,
        _ => Button::Left,
    }
}

pub fn start(opts: ClickOptions) -> anyhow::Result<()> {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Err(anyhow!("ja esta rodando"));
    }
    if !(0.1..=1000.0).contains(&opts.cps) {
        RUNNING.store(false, Ordering::SeqCst);
        return Err(anyhow!("CPS fora de 0,1 a 1000"));
    }
    *LAST_OPTS.lock().unwrap_or_else(|e| e.into_inner()) = Some(opts.clone());
    *ERROR.lock().unwrap_or_else(|e| e.into_inner()) = None;
    CLICKS.store(0, Ordering::SeqCst);
    *STARTED.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());

    std::thread::Builder::new()
        .name("omniget-autoclick".into())
        .spawn(move || {
            let mut enigo = match Enigo::new(&Settings::default()) {
                Ok(e) => e,
                Err(e) => {
                    *ERROR.lock().unwrap_or_else(|e| e.into_inner()) = Some(format!("nao consegui controlar o mouse: {} (macOS: Ajustes → Privacidade → Acessibilidade)", e));
                    RUNNING.store(false, Ordering::SeqCst);
                    return;
                }
            };
            if opts.start_delay > 0 {
                let until = Instant::now() + Duration::from_secs(opts.start_delay as u64);
                while RUNNING.load(Ordering::SeqCst) && Instant::now() < until {
                    std::thread::sleep(Duration::from_millis(20));
                }
                *STARTED.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
            }
            let button = button_of(&opts.button);
            let base = Duration::from_secs_f64(1.0 / opts.cps);
            let hold = Duration::from_millis(opts.hold_ms as u64);
            let started = Instant::now();
            let mut next = Instant::now();
            let mut rng_state: u64 = started.elapsed().as_nanos() as u64 ^ 0x9E37_79B9_7F4A_7C15;
            while RUNNING.load(Ordering::SeqCst) {
                if opts.max_seconds > 0 && started.elapsed().as_secs() >= opts.max_seconds {
                    break;
                }
                if let Some((x, y)) = opts.position {
                    let _ = enigo.move_mouse(x, y, Coordinate::Abs);
                }
                let clicks = if opts.double { 2 } else { 1 };
                for _ in 0..clicks {
                    let r = if hold.is_zero() {
                        enigo.button(button, Direction::Click)
                    } else {
                        enigo.button(button, Direction::Press).and_then(|_| {
                            std::thread::sleep(hold);
                            enigo.button(button, Direction::Release)
                        })
                    };
                    if let Err(e) = r {
                        *ERROR.lock().unwrap_or_else(|e| e.into_inner()) = Some(e.to_string());
                        RUNNING.store(false, Ordering::SeqCst);
                        return;
                    }
                    if opts.double {
                        std::thread::sleep(Duration::from_millis(30));
                    }
                }
                let n = CLICKS.fetch_add(1, Ordering::SeqCst) + 1;
                if opts.max_clicks > 0 && n >= opts.max_clicks {
                    break;
                }
                // xorshift para o jitter, sem crate
                rng_state ^= rng_state << 13;
                rng_state ^= rng_state >> 7;
                rng_state ^= rng_state << 17;
                let jitter = if opts.jitter_pct > 0 {
                    let pct = (rng_state % (2 * opts.jitter_pct as u64 + 1)) as f64 - opts.jitter_pct as f64;
                    1.0 + pct / 100.0
                } else {
                    1.0
                };
                next += base.mul_f64(jitter.max(0.05));
                if next < Instant::now() {
                    next = Instant::now();
                }
                sleep_until(next);
            }
            RUNNING.store(false, Ordering::SeqCst);
        })
        .map_err(|e| {
            RUNNING.store(false, Ordering::SeqCst);
            anyhow!("thread: {}", e)
        })?;
    Ok(())
}

pub fn stop() {
    RUNNING.store(false, Ordering::SeqCst);
}

/// Atalho global: para se estiver rodando, senão começa com as últimas opções.
pub fn toggle() -> anyhow::Result<bool> {
    if RUNNING.load(Ordering::SeqCst) {
        stop();
        return Ok(false);
    }
    let opts = last_options().ok_or_else(|| anyhow!("configure e inicie uma vez pela tela antes de usar o atalho"))?;
    start(opts)?;
    Ok(true)
}

pub fn mouse_position() -> Option<(i32, i32)> {
    let enigo = Enigo::new(&Settings::default()).ok()?;
    enigo.location().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precise_sleep() {
        let t = Instant::now();
        sleep_until(t + Duration::from_millis(5));
        let e = t.elapsed();
        assert!(e >= Duration::from_millis(5) && e < Duration::from_millis(25), "{:?}", e);
    }

    #[test]
    fn options_defaults() {
        let o: ClickOptions = serde_json::from_str(r#"{"cps": 10}"#).unwrap();
        assert_eq!(o.button, "left");
        assert_eq!(o.max_clicks, 0);
        assert!(!state().running);
    }
}
