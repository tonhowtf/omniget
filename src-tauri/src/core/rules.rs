//! Motor de regras declarativo: "URL do canal X -> pasta Y, qualidade Z".
//!
//! Escrito uma vez pelo usuario e avaliado no enqueue, para parar de repetir as
//! mesmas cinco escolhas em todo download do mesmo canal.
//!
//! Deliberadamente **nao** e linguagem de script. E uma lista de regras com
//! combos, porque um editor de expressao vira superficie de suporte propria e o
//! ganho sobre "contem / e igual a" e pequeno para este caso.
//!
//! Origem: Sieve, de clientes de e-mail.

use serde::{Deserialize, Serialize};

/// O que a regra olha. Primeira que casar vence, entao a ordem da lista e a
/// prioridade — igual a filtro de e-mail, e pela mesma razao: previsibilidade
/// vale mais que expressividade.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Condition {
    /// Host exatamente igual, ignorando `www.` e caixa.
    HostIs { value: String },
    /// Substring em qualquer lugar da URL, sem caixa.
    UrlContains { value: String },
    /// Plataforma resolvida pelo registry (`youtube`, `bilibili`, ...).
    PlatformIs { value: String },
}

impl Condition {
    pub fn matches(&self, url: &str, platform: Option<&str>) -> bool {
        match self {
            Condition::HostIs { value } => host_of(url)
                .is_some_and(|h| h.eq_ignore_ascii_case(value.trim_start_matches("www."))),
            Condition::UrlContains { value } => url.to_lowercase().contains(&value.to_lowercase()),
            Condition::PlatformIs { value } => {
                platform.is_some_and(|p| p.eq_ignore_ascii_case(value))
            }
        }
    }
}

/// O que a regra faz. Campo ausente significa "nao mexe", nao "usa o default" —
/// a distincao importa para poder ter regra que so muda a pasta.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Actions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_only: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitles: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Rule {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub name: String,
    pub when: Condition,
    pub then: Actions,
}

fn default_enabled() -> bool {
    true
}

/// Host de uma URL, minusculo e sem `www.`.
///
/// Parsing manual em vez da crate `url`: aceita entrada malformada sem erro, o
/// que e o que se quer aqui — regra que nao casa e melhor que enqueue abortado.
pub fn host_of(url: &str) -> Option<String> {
    let after_scheme = url.split("://").nth(1).unwrap_or(url);
    let host = after_scheme
        .split(['/', '?', '#'])
        .next()?
        .split('@')
        .next_back()?
        .split(':')
        .next()?;
    if host.is_empty() {
        return None;
    }
    Some(host.trim_start_matches("www.").to_lowercase())
}

/// Primeira regra habilitada que casa.
pub fn first_match<'a>(rules: &'a [Rule], url: &str, platform: Option<&str>) -> Option<&'a Rule> {
    rules
        .iter()
        .filter(|r| r.enabled)
        .find(|r| r.when.matches(url, platform))
}
