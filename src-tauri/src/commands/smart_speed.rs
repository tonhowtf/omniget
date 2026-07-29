//! Mapa de silencio de uma aula, para o player pular durante a reproducao.
//!
//! B52. O `core::silence_map` sabe onde estao os silencios e para onde saltar;
//! o que faltava era computar o mapa e guardar.
//!
//! Pular na reproducao, e nao cortar o arquivo, e a diferenca que importa: o
//! B36 cortava com `silenceremove` e pagava dois precos — residuo de padding e
//! saida obrigatoriamente em audio, porque filtro de audio dessincroniza video.
//! Aqui o video continua video e o padding vira decisao reversivel.
//!
//! A sonda e cara (uma passada de ffmpeg no arquivo inteiro), entao roda uma vez
//! e o mapa fica em disco. Nada e reprocessado ao assistir de novo.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;

use crate::core::silence_map::{
    needs_recompute, savings_secs, skip_target, SilenceMap, CURRENT_VERSION,
};

const MAPS_FILE: &str = "silence-maps.json";

fn file_path() -> Option<PathBuf> {
    crate::core::paths::app_data_dir().map(|d| d.join(MAPS_FILE))
}

fn load() -> BTreeMap<String, SilenceMap> {
    let Some(path) = file_path() else {
        return BTreeMap::new();
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return BTreeMap::new();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

fn save(map: &BTreeMap<String, SilenceMap>) {
    let Some(path) = file_path() else { return };
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    let Ok(json) = serde_json::to_string(map) else {
        return;
    };
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, json).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// Duracao total do arquivo, lida da mesma saida do ffmpeg.
///
/// Precisa vir junto porque um `silence_start` sem `silence_end` so pode ser
/// interpretado sabendo onde o arquivo acaba.
fn parse_duration_secs(stderr: &str) -> Option<f64> {
    let idx = stderr.find("Duration:")?;
    let resto = &stderr[idx + "Duration:".len()..];
    let bruto = resto.split(',').next()?.trim();
    let mut partes = bruto.split(':');
    let h: f64 = partes.next()?.trim().parse().ok()?;
    let m: f64 = partes.next()?.trim().parse().ok()?;
    let s: f64 = partes.next()?.trim().parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + s)
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SilenceMapInfo {
    pub map: SilenceMap,
    /// Quanto tempo o pulo economiza, em segundos.
    pub savings_secs: f64,
    /// `true` quando o mapa veio do cache, sem sondar de novo.
    pub from_cache: bool,
}

/// Computa (ou reaproveita) o mapa de silencio de um arquivo.
#[tauri::command]
pub async fn compute_silence_map(
    path: String,
    force: Option<bool>,
) -> Result<SilenceMapInfo, String> {
    let force = force.unwrap_or(false);
    let arquivo = PathBuf::from(&path);
    if !arquivo.is_file() {
        return Err("O arquivo nao existe".to_string());
    }

    if !force {
        if let Some(cached) = load().get(&path) {
            // `needs_recompute` compara a versao do algoritmo: um mapa gravado
            // com outros parametros de sonda descreve outra coisa.
            if !needs_recompute(cached) {
                return Ok(SilenceMapInfo {
                    savings_secs: savings_secs(cached),
                    map: cached.clone(),
                    from_cache: true,
                });
            }
        }
    }

    let ffmpeg = crate::core::dependencies::find_tool("ffmpeg")
        .await
        .ok_or_else(|| "FFmpeg nao encontrado. Instale em Config → Plugins.".to_string())?;

    let alvo = arquivo.clone();
    let stderr = tokio::task::spawn_blocking(move || {
        let mut cmd = crate::core::process::std_command(&ffmpeg);
        cmd.arg("-i").arg(&alvo);
        for a in omniget_core::core::ffmpeg_ops::silence_probe_args() {
            cmd.arg(a);
        }
        cmd.arg("-")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;

    let texto = String::from_utf8_lossy(&stderr.stderr);
    let duracao = parse_duration_secs(&texto)
        .ok_or_else(|| "Nao foi possivel ler a duracao do arquivo".to_string())?;

    let mapa = SilenceMap {
        version: CURRENT_VERSION,
        media_duration_secs: duracao,
        spans: crate::core::silence_map::parse_spans(&texto),
    };

    let mut todos = load();
    todos.insert(path, mapa.clone());
    save(&todos);

    Ok(SilenceMapInfo {
        savings_secs: savings_secs(&mapa),
        map: mapa,
        from_cache: false,
    })
}

/// Para onde saltar, se a posicao atual cai dentro de um silencio.
///
/// Chamada pelo player a cada atualizacao de tempo. Fica no backend em vez de
/// duplicar a regra em TypeScript: uma segunda implementacao do mesmo calculo
/// divergiria no primeiro ajuste de padding.
#[tauri::command]
pub fn silence_skip_target(path: String, position_secs: f64) -> Option<f64> {
    let mapa = load();
    skip_target(mapa.get(&path)?, position_secs)
}

/// Esquece o mapa de um arquivo.
#[tauri::command]
pub fn forget_silence_map(path: String) {
    let mut todos = load();
    if todos.remove(&path).is_some() {
        save(&todos);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// stderr real do ffmpeg, recortado.
    const STDERR: &str = "\
Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'aula.mp4':
  Duration: 00:41:03.52, start: 0.000000, bitrate: 1200 kb/s
[silencedetect @ 0x7f8] silence_start: 12.4213
[silencedetect @ 0x7f8] silence_end: 15.0102 | silence_duration: 2.5889
";

    #[test]
    fn a_duracao_sai_do_mesmo_stderr_da_sonda() {
        // Sem ela, um silencio que vai ate o fim do arquivo nao tem como ser
        // medido — e uma passada extra so para isso dobraria o custo.
        let d = parse_duration_secs(STDERR).expect("le duracao");
        assert!((d - 2463.52).abs() < 0.01, "41:03.52 = 2463.52s, veio {d}");
    }

    #[test]
    fn stderr_sem_duracao_nao_inventa_numero() {
        assert_eq!(parse_duration_secs("nada aqui"), None);
    }

    #[test]
    fn duracao_com_horas_e_lida_certo() {
        let s = "  Duration: 02:05:30.00, start: 0.0";
        let d = parse_duration_secs(s).expect("le");
        assert!((d - 7530.0).abs() < 0.01, "2h05m30s = 7530s, veio {d}");
    }

    #[test]
    fn mapa_de_versao_antiga_e_recomputado() {
        // O ponto do campo `version`: um mapa gravado com outros parametros de
        // sonda descreve outra coisa, e reaproveitar seria pular no lugar errado.
        let antigo = SilenceMap {
            version: 0,
            media_duration_secs: 100.0,
            spans: Vec::new(),
        };
        assert!(needs_recompute(&antigo));

        let atual = SilenceMap {
            version: CURRENT_VERSION,
            media_duration_secs: 100.0,
            spans: Vec::new(),
        };
        assert!(!needs_recompute(&atual));
    }
}
