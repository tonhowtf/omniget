use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

const RECOVERY_FILE: &str = "recovery.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryItem {
    pub id: u64,
    pub url: String,
    pub title: String,
    pub platform: String,
    pub output_dir: String,
    #[serde(default)]
    pub download_mode: Option<String>,
    #[serde(default)]
    pub quality: Option<String>,
    #[serde(default)]
    pub format_id: Option<String>,
    #[serde(default)]
    pub referer: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct RecoveryFile {
    #[serde(default)]
    items: Vec<RecoveryItem>,
}

static STORE: OnceLock<Mutex<HashMap<u64, RecoveryItem>>> = OnceLock::new();

fn store() -> &'static Mutex<HashMap<u64, RecoveryItem>> {
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn file_path() -> Option<PathBuf> {
    crate::core::paths::app_data_dir().map(|d| d.join(RECOVERY_FILE))
}

const WAL_FILE: &str = "queue.wal";

fn wal_path() -> Option<PathBuf> {
    crate::core::paths::app_data_dir().map(|d| d.join(WAL_FILE))
}

/// Anexa um registro ao log, com `fsync` por registro.
///
/// Isto e o ponto do B32: o formato anterior reescrevia o arquivo inteiro a
/// cada mudanca, entao um crash no meio da escrita podia levar a fila toda.
/// Anexar so pode corromper o ultimo registro, e o `decode_log` descarta a
/// linha truncada e mantem o resto.
fn append(record: &crate::core::queue_wal::WalRecord) {
    let Some(path) = wal_path() else { return };
    let Some(parent) = path.parent() else { return };
    if let Err(e) = std::fs::create_dir_all(parent) {
        tracing::warn!("[recovery] create_dir_all failed: {}", e);
        return;
    }
    let Some(line) = crate::core::queue_wal::encode(record) else {
        tracing::warn!("[recovery] encode failed");
        return;
    };
    let result = (|| -> std::io::Result<()> {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        f.write_all(line.as_bytes())?;
        f.write_all(b"\n")?;
        // Sem o fsync, o WAL nao vale mais que o JSON: o kernel pode nao ter
        // escrito nada quando o processo morre.
        f.sync_all()?;
        Ok(())
    })();
    if let Err(e) = result {
        tracing::warn!("[recovery] append failed: {}", e);
    }
}

/// The recovery log is on disk and listed in the UI: only redacted URLs.
///
/// A public link survives unchanged (redaction removes nothing) and is
/// restored as before. A link that carried a secret (signed query, token) is
/// kept in its display form; restoring it fails honestly with "link expired,
/// paste again" instead of sending `[REDACTED]` to the server. The executable
/// URL is not kept anywhere: by the time the app is back after a crash a
/// signed link has usually expired anyway.
fn scrub(mut item: RecoveryItem) -> RecoveryItem {
    use crate::core::flight_recorder::{redact_url, redact_urls};
    item.url = redact_url(&item.url);
    // The placeholder title is the URL (N-3): same redaction, same log.
    item.title = redact_urls(&item.title);
    item.referer = item.referer.as_deref().map(redact_url);
    item
}

/// True when the item can no longer be restored: its URL lost its secret.
pub fn is_expired(item: &RecoveryItem) -> bool {
    crate::core::flight_recorder::is_redacted_url(&item.url)
}

/// Rewrites the log with the given items (atomic rename). Used once at boot
/// when older records still held a raw URL.
fn rewrite(items: &[RecoveryItem]) {
    let Some(path) = wal_path() else { return };
    let tmp = path.with_extension("wal.tmp");
    let mut body = String::new();
    for (pos, item) in items.iter().enumerate() {
        if let Some(line) = crate::core::queue_wal::encode(&enqueued_record(item, pos as u32)) {
            body.push_str(&line);
            body.push('\n');
        }
    }
    let result = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(body.as_bytes())?;
        f.sync_all()?;
        std::fs::rename(&tmp, &path)
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        tracing::warn!("[recovery] rewrite failed: {}", e);
    }
}

fn enqueued_record(item: &RecoveryItem, position: u32) -> crate::core::queue_wal::WalRecord {
    crate::core::queue_wal::WalRecord::Enqueued {
        id: item.id,
        url: item.url.clone(),
        title: item.title.clone(),
        platform: item.platform.clone(),
        output_dir: item.output_dir.clone(),
        quality: item.quality.clone(),
        download_mode: item.download_mode.clone(),
        format_id: item.format_id.clone(),
        referer: item.referer.clone(),
        position,
    }
}

fn recovered_to_item(r: &crate::core::queue_wal::RecoveredItem) -> RecoveryItem {
    scrub(RecoveryItem {
        id: r.id,
        url: r.url.clone(),
        title: r.title.clone(),
        platform: r.platform.clone(),
        output_dir: r.output_dir.clone(),
        download_mode: r.download_mode.clone(),
        quality: r.quality.clone(),
        format_id: r.format_id.clone(),
        referer: r.referer.clone(),
    })
}

/// Converte um `recovery.json` da 0.7.x para o WAL, uma vez so.
///
/// Retorna os itens migrados. O JSON e renomeado em vez de apagado: se algo
/// der errado na primeira execucao com o WAL, o arquivo original ainda esta la.
/// Ordem em que os itens do `recovery.json` entram no WAL.
///
/// Estavel por id: o JSON nao guardava posicao, e inventar uma ordem arbitraria
/// mudaria a fila do usuario sem motivo. Separado do I/O para poder ser testado
/// sem tocar em disco nem no estado global.
fn ordenar_para_migracao(mut itens: Vec<RecoveryItem>) -> Vec<RecoveryItem> {
    itens.sort_by_key(|i| i.id);
    itens
}

/// Le um `recovery.json` da 0.7.x e devolve os registros equivalentes.
///
/// Puro de proposito: e a funcao que o teste de migracao precisa exercitar, e
/// e onde um campo esquecido some em silencio.
fn json_para_registros(content: &str) -> Vec<crate::core::queue_wal::WalRecord> {
    let parsed: RecoveryFile = match serde_json::from_str(content) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    ordenar_para_migracao(parsed.items.into_iter().map(scrub).collect())
        .iter()
        .enumerate()
        .map(|(pos, item)| enqueued_record(item, pos as u32))
        .collect()
}

fn migrate_from_json() -> Vec<RecoveryItem> {
    let Some(json_path) = file_path() else {
        return Vec::new();
    };
    let Ok(content) = std::fs::read_to_string(&json_path) else {
        return Vec::new();
    };

    // Mesma funcao que o teste de migracao exercita. Se a producao parseasse
    // por fora, o teste estaria provando algo que ninguem executa.
    let registros = json_para_registros(&content);
    if registros.is_empty() && !content.trim().is_empty() {
        tracing::warn!("[recovery] recovery.json ilegivel ou vazio, nada migrado");
    }
    for r in &registros {
        append(r);
    }
    let itens: Vec<RecoveryItem> = crate::core::queue_wal::replay(&registros)
        .iter()
        .map(recovered_to_item)
        .collect();

    let destino = json_path.with_extension("json.migrated");
    if let Err(e) = std::fs::rename(&json_path, &destino) {
        tracing::warn!("[recovery] nao foi possivel renomear o json migrado: {}", e);
    }
    tracing::info!("[recovery] {} itens migrados do recovery.json", itens.len());
    itens
}

pub fn init_from_disk() {
    let mut guard = store().lock().unwrap();
    guard.clear();

    let wal = wal_path().and_then(|p| std::fs::read_to_string(p).ok());
    match wal {
        Some(contents) => {
            let (records, descartados) = crate::core::queue_wal::decode_log(&contents);
            if descartados > 0 {
                // Esperado apos um crash: a ultima linha estava pela metade.
                tracing::warn!(
                    "[recovery] {} registro(s) truncado(s) descartado(s)",
                    descartados
                );
            }
            let replayed = crate::core::queue_wal::replay(&records);
            // Records written before redaction held the raw URL: rewrite the
            // log so the secret leaves the disk, not only the UI.
            let stale = replayed.iter().any(|r| {
                let scrubbed = recovered_to_item(r);
                scrubbed.url != r.url || scrubbed.referer != r.referer || scrubbed.title != r.title
            });
            let items: Vec<RecoveryItem> = replayed.iter().map(recovered_to_item).collect();
            if stale {
                rewrite(&items);
            }
            for item in items {
                guard.insert(item.id, item);
            }
        }
        None => {
            drop(guard);
            let migrados = migrate_from_json();
            let mut guard = store().lock().unwrap();
            for item in migrados {
                guard.insert(item.id, item);
            }
        }
    }
}

pub fn persist(item: RecoveryItem) {
    let item = scrub(item);
    let mut guard = store().lock().unwrap();
    let position = guard.len() as u32;
    append(&enqueued_record(&item, position));
    guard.insert(item.id, item);
}

pub fn remove(id: u64) {
    let mut guard = store().lock().unwrap();
    if guard.remove(&id).is_some() {
        append(&crate::core::queue_wal::WalRecord::Removed { id });
    }
}

pub fn list() -> Vec<RecoveryItem> {
    let guard = store().lock().unwrap();
    guard.values().cloned().collect()
}

pub fn clear_all() {
    let mut guard = store().lock().unwrap();
    guard.clear();
    if let Some(path) = wal_path() {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod migracao_tests {
    use super::*;
    use crate::core::queue_wal::{replay, WalRecord};

    /// Recorte no formato exato que a 0.7.x gravava, com `referer` preenchido.
    const JSON_0_7_X: &str = r#"{
      "items": [
        {
          "id": 7,
          "url": "https://exemplo.com/b",
          "title": "Segundo",
          "platform": "youtube",
          "output_dir": "/downloads",
          "download_mode": "audio",
          "quality": "720p",
          "format_id": "140",
          "referer": "https://exemplo.com/pagina"
        },
        {
          "id": 2,
          "url": "https://exemplo.com/a",
          "title": "Primeiro",
          "platform": "instagram",
          "output_dir": "/downloads"
        }
      ]
    }"#;

    #[test]
    fn migracao_preserva_o_referer() {
        // O `Enqueued` do WAL nao tinha `referer` quando a #217 foi escrita.
        // Sem este teste, migrar teria descartado o campo em silencio, e o
        // download so falharia depois, no site que exige o header.
        let registros = json_para_registros(JSON_0_7_X);
        let itens = replay(&registros);
        let com_referer = itens.iter().find(|i| i.id == 7).expect("item 7");
        assert_eq!(
            com_referer.referer.as_deref(),
            Some("https://exemplo.com/pagina")
        );
    }

    #[test]
    fn migracao_preserva_todos_os_campos_do_item() {
        let itens = replay(&json_para_registros(JSON_0_7_X));
        let i = itens.iter().find(|i| i.id == 7).expect("item 7");
        assert_eq!(i.url, "https://exemplo.com/b");
        assert_eq!(i.title, "Segundo");
        assert_eq!(i.platform, "youtube");
        assert_eq!(i.output_dir, "/downloads");
        assert_eq!(i.download_mode.as_deref(), Some("audio"));
        assert_eq!(i.quality.as_deref(), Some("720p"));
        assert_eq!(i.format_id.as_deref(), Some("140"));
    }

    #[test]
    fn campos_opcionais_ausentes_nao_quebram_a_migracao() {
        // O item 2 do recorte nao tem download_mode, quality, format_id nem
        // referer — arquivo de quem so usou os padroes.
        let itens = replay(&json_para_registros(JSON_0_7_X));
        let i = itens.iter().find(|i| i.id == 2).expect("item 2");
        assert_eq!(i.platform, "instagram");
        assert!(i.quality.is_none());
        assert!(i.referer.is_none());
    }

    #[test]
    fn ordem_da_fila_sobrevive_a_migracao() {
        // O JSON guardava um mapa sem ordem. Ordenar por id e a unica escolha
        // que nao embaralha a fila de quem atualizar.
        let itens = replay(&json_para_registros(JSON_0_7_X));
        assert_eq!(itens.len(), 2);
        assert_eq!(itens[0].id, 2, "o menor id vem primeiro");
        assert_eq!(itens[1].id, 7);
        assert_eq!(itens[0].position, 0);
        assert_eq!(itens[1].position, 1);
    }

    #[test]
    fn json_corrompido_nao_derruba_o_boot() {
        // Um arquivo ilegivel nao pode impedir o app de subir; a fila perdida e
        // ruim, o app que nao abre e pior.
        assert!(json_para_registros("{ isto nao e json").is_empty());
        assert!(json_para_registros("").is_empty());
    }

    #[test]
    fn url_assinada_nunca_vai_para_o_log() {
        // G06/D-11: recovery.json/queue.wal held signed queries in clear.
        let item = RecoveryItem {
            id: 9,
            url: "https://cdn.example.com/v.mp4?X-Amz-Signature=SYNTHETIC_SECRET&token=SYNTHETIC_SECRET".into(),
            // N-3: before metadata arrives the title is the URL itself.
            title: "https://cdn.example.com/v.mp4?X-Amz-Signature=SYNTHETIC_SECRET&token=SYNTHETIC_SECRET".into(),
            platform: "generic".into(),
            output_dir: "/d".into(),
            download_mode: None,
            quality: None,
            format_id: None,
            referer: Some("https://example.com/p?access_token=SYNTHETIC_SECRET".into()),
        };
        let line = crate::core::queue_wal::encode(&enqueued_record(&scrub(item), 0)).unwrap();
        assert!(!line.contains("SYNTHETIC_SECRET"), "{line}");
        let back = replay(&crate::core::queue_wal::decode_log(&line).0);
        let restored = recovered_to_item(&back[0]);
        assert!(is_expired(&restored), "a redacted link cannot be restored");
    }

    #[test]
    fn link_publico_continua_restauravel() {
        let itens = replay(&json_para_registros(JSON_0_7_X));
        let i = recovered_to_item(itens.iter().find(|i| i.id == 7).unwrap());
        assert_eq!(i.url, "https://exemplo.com/b");
        assert!(!is_expired(&i));
    }

    #[test]
    fn registro_antigo_com_segredo_sai_redigido_no_replay() {
        let json = r#"{"items":[{"id":1,"url":"https://x.example/v?sig=SYNTHETIC_SECRET","title":"https://x.example/v?sig=SYNTHETIC_SECRET","platform":"generic","output_dir":"/d"}]}"#;
        let i = recovered_to_item(&replay(&json_para_registros(json))[0]);
        assert!(!i.url.contains("SYNTHETIC_SECRET"));
        assert!(!i.title.contains("SYNTHETIC_SECRET"), "N-3: {}", i.title);
        assert!(is_expired(&i));
    }

    #[test]
    fn item_removido_some_do_replay() {
        let mut registros = json_para_registros(JSON_0_7_X);
        registros.push(WalRecord::Removed { id: 7 });
        let itens = replay(&registros);
        assert_eq!(itens.len(), 1);
        assert_eq!(itens[0].id, 2);
    }
}
