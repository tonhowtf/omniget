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

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(name: &str, when: Condition, then: Actions) -> Rule {
        Rule {
            enabled: true,
            name: name.to_string(),
            when,
            then,
        }
    }

    #[test]
    fn host_normaliza_www_porta_e_caixa() {
        let casos = [
            ("https://www.YouTube.com/watch?v=1", "youtube.com"),
            ("http://youtube.com:8080/x", "youtube.com"),
            ("https://user@bilibili.com/video", "bilibili.com"),
            ("bilibili.com/video/BV1", "bilibili.com"),
        ];
        for (url, esperado) in casos {
            assert_eq!(host_of(url).as_deref(), Some(esperado), "url {url}");
        }
        assert_eq!(host_of("").as_deref(), None);
        assert_eq!(host_of("https://").as_deref(), None);
    }

    #[test]
    fn a_primeira_regra_que_casa_vence() {
        // A ordem e a prioridade. Sem isso, duas regras que casam viram
        // comportamento imprevisivel e o usuario nao consegue depurar.
        let rules = vec![
            rule(
                "especifica",
                Condition::UrlContains {
                    value: "/@canal-x".into(),
                },
                Actions {
                    output_dir: Some("/cursos/canal-x".into()),
                    ..Default::default()
                },
            ),
            rule(
                "geral",
                Condition::HostIs {
                    value: "youtube.com".into(),
                },
                Actions {
                    output_dir: Some("/videos".into()),
                    ..Default::default()
                },
            ),
        ];
        let m = first_match(&rules, "https://youtube.com/@canal-x/videos", None).unwrap();
        assert_eq!(m.name, "especifica");

        let m2 = first_match(&rules, "https://youtube.com/watch?v=9", None).unwrap();
        assert_eq!(m2.name, "geral");
    }

    #[test]
    fn regra_desligada_e_pulada_sem_bloquear_a_seguinte() {
        let mut rules = vec![
            rule(
                "desligada",
                Condition::HostIs {
                    value: "youtube.com".into(),
                },
                Actions {
                    quality: Some("360".into()),
                    ..Default::default()
                },
            ),
            rule(
                "ativa",
                Condition::HostIs {
                    value: "youtube.com".into(),
                },
                Actions {
                    quality: Some("1080".into()),
                    ..Default::default()
                },
            ),
        ];
        rules[0].enabled = false;
        let m = first_match(&rules, "https://youtube.com/watch?v=1", None).unwrap();
        assert_eq!(m.name, "ativa");
    }

    #[test]
    fn nenhuma_regra_casando_devolve_none() {
        let rules = vec![rule(
            "so bilibili",
            Condition::PlatformIs {
                value: "bilibili".into(),
            },
            Actions::default(),
        )];
        assert!(first_match(&rules, "https://youtube.com/x", Some("youtube")).is_none());
        assert!(first_match(&rules, "https://youtube.com/x", None).is_none());
        assert!(first_match(&[], "https://qualquer.com", None).is_none());
    }

    #[test]
    fn plataforma_casa_sem_depender_da_url() {
        let rules = vec![rule(
            "cursos",
            Condition::PlatformIs {
                value: "Hotmart".into(),
            },
            Actions {
                subtitles: Some(true),
                tags: vec!["curso".into()],
                ..Default::default()
            },
        )];
        let m = first_match(&rules, "https://algum-cdn.example/x", Some("hotmart")).unwrap();
        assert_eq!(m.then.tags, vec!["curso"]);
        assert_eq!(m.then.subtitles, Some(true));
    }

    #[test]
    fn acoes_ausentes_significam_nao_mexer() {
        // Distincao que importa: `None` nao pode virar "usa o default", senao
        // uma regra que so troca a pasta zeraria a qualidade escolhida.
        let a = Actions {
            output_dir: Some("/x".into()),
            ..Default::default()
        };
        assert_eq!(a.quality, None);
        assert_eq!(a.audio_only, None);
        assert!(a.tags.is_empty());
    }

    #[test]
    fn regra_sobrevive_ao_round_trip_json() {
        let r = rule(
            "canal x",
            Condition::UrlContains { value: "@x".into() },
            Actions {
                output_dir: Some("/a".into()),
                quality: Some("1080".into()),
                tags: vec!["aula".into()],
                ..Default::default()
            },
        );
        let json = serde_json::to_string(&r).unwrap();
        let back: Rule = serde_json::from_str(&json).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn regra_antiga_sem_enabled_carrega_habilitada() {
        // Campo novo nao pode desligar em silencio as regras que o usuario ja
        // tinha — e a mesma classe do bug que zerou o settings.json.
        let json = r#"{"name":"velha","when":{"kind":"host_is","value":"x.com"},"then":{}}"#;
        let r: Rule = serde_json::from_str(json).unwrap();
        assert!(r.enabled);
    }
}
