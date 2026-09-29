//! Ajusta `-N` por host medindo o throughput real, em vez de confiar num
//! numero fixo em Config.
//!
//! O numero em Config e um chute que o usuario tem que dar sem informacao: alto
//! demais toma 429, baixo demais desperdiça banda, e o valor certo depende do
//! host e do momento. O limitador por host e o contador de 429 ja existem — o
//! que faltava era fechar o laco.
//!
//! Ficou possivel agora porque o B23 fez o freio de 429 funcionar de verdade:
//! antes, `-N` e `--concurrent-fragments` disputavam e o valor efetivo nao era
//! o que o codigo pensava, entao nao havia sinal confiavel para realimentar.
//!
//! A decisao e pura: dadas amostras de throughput e o historico de 429, qual
//! deve ser o proximo `-N`. Medir e I/O.
//!
//! Origem: controle de congestionamento de CDN — subir devagar, cair rapido.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

/// Limites do ajuste. Um host nunca sai desta faixa, por mais que as medicoes
/// sugiram: o teto protege o servidor e o piso garante que o download anda.
pub const MIN_N: u32 = 1;
pub const MAX_N: u32 = 16;

/// Orçamento de conexões da máquina inteira, somando todos os downloads
/// ativos. O stress de 2/4/8 downloads de 1 GiB no loopback (estudo 66)
/// saturou acima de 32 conexões: 4 downloads × 8 segmentos rendeu 3,9 GB/s e
/// 8 × 8 caiu para 1,9 GB/s. Vinte e quatro deixa 3 downloads com o `-N 8`
/// cheio e reparte o resto quando a fila enche.
pub const GLOBAL_CONNECTION_BUDGET: u32 = 24;

static ACTIVE_DOWNLOADS: AtomicU32 = AtomicU32::new(0);

/// Quantos downloads estão puxando bytes agora (yt-dlp ou fetcher direto).
pub fn active_downloads() -> u32 {
    ACTIVE_DOWNLOADS.load(Ordering::Relaxed)
}

/// Marca um download como ativo enquanto o guard vive.
pub struct ActiveDownloadGuard(());

impl ActiveDownloadGuard {
    pub fn new() -> Self {
        ACTIVE_DOWNLOADS.fetch_add(1, Ordering::Relaxed);
        Self(())
    }
}

impl Default for ActiveDownloadGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ActiveDownloadGuard {
    fn drop(&mut self) {
        ACTIVE_DOWNLOADS.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Teto por download dado o orçamento global e quantos downloads o dividem.
///
/// `active` inclui o download que está perguntando. Nunca abaixo de `MIN_N`:
/// um download sempre anda, mesmo que a fila esteja lotada.
pub fn per_download_ceiling(budget: u32, active: u32) -> u32 {
    (budget / active.max(1)).clamp(MIN_N, MAX_N)
}

/// Ganho minimo para justificar subir mais.
///
/// Dez por cento: abaixo disso a diferenca se confunde com variacao normal de
/// rede, e subir `-N` atras de ruido so aumenta a chance de 429.
pub const MEANINGFUL_GAIN: f64 = 1.10;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Sample {
    pub concurrency: u32,
    /// Bytes por segundo observados com essa concorrencia.
    pub throughput_bps: f64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct HostStats {
    samples: Vec<Sample>,
    rate_limit_hits: u32,
}

impl HostStats {
    pub fn record(&mut self, concurrency: u32, throughput_bps: f64) {
        if throughput_bps.is_finite() && throughput_bps > 0.0 {
            self.samples.push(Sample {
                concurrency,
                throughput_bps,
            });
        }
    }

    pub fn record_rate_limit(&mut self) {
        self.rate_limit_hits = self.rate_limit_hits.saturating_add(1);
    }

    /// Melhor throughput observado para uma dada concorrencia.
    ///
    /// Melhor e nao media: uma amostra ruim costuma ser interferencia local
    /// (outro download, wifi), nao a capacidade real daquele nivel.
    fn best_at(&self, concurrency: u32) -> Option<f64> {
        self.samples
            .iter()
            .filter(|s| s.concurrency == concurrency)
            .map(|s| s.throughput_bps)
            .fold(None, |acc: Option<f64>, v| {
                Some(acc.map_or(v, |a| a.max(v)))
            })
    }
}

/// Proximo `-N` para o host.
///
/// Sobe devagar e cai rapido, que e a assimetria certa: subir errado custa um
/// 429 e um retry; descer errado custa um pouco de velocidade.
pub fn next_concurrency(stats: &HostStats, current: u32) -> u32 {
    let current = current.clamp(MIN_N, MAX_N);

    // 429 domina qualquer medicao de velocidade. Nao adianta o throughput
    // parecer bom se o host esta recusando.
    if stats.rate_limit_hits > 0 {
        let penalizado = current / 2u32.pow(stats.rate_limit_hits.min(3));
        return penalizado.clamp(MIN_N, MAX_N);
    }

    let Some(atual) = stats.best_at(current) else {
        return current; // sem medicao ainda: nao mexe
    };

    match stats.best_at(current.saturating_sub(1)) {
        // Se um nivel abaixo era melhor, subir foi erro — desce.
        Some(anterior) if anterior > atual * MEANINGFUL_GAIN => (current - 1).clamp(MIN_N, MAX_N),
        _ => {
            // Só sobe se o degrau anterior mostrou ganho real.
            let vale_subir = match stats.best_at(current.saturating_sub(1)) {
                Some(anterior) => atual > anterior * MEANINGFUL_GAIN,
                None => true, // primeiro degrau medido: tenta subir uma vez
            };
            if vale_subir {
                (current + 1).clamp(MIN_N, MAX_N)
            } else {
                current
            }
        }
    }
}

/// Por quanto tempo um 429 continua valendo para os próximos processos.
///
/// Um dia: o download.blender.org recusou a segunda execução seguida do CLI
/// (benchmarks, sessões 01 a 04), e cada `omniget download` começava de novo
/// com todas as conexões. Depois de um dia o host ganha outra chance inteira.
pub const RATE_LIMIT_MEMORY_SECS: u64 = 24 * 60 * 60;

/// O que fica gravado de um host entre processos: só os 429, que são a
/// resposta do servidor. Amostras de velocidade dependem da rede do momento e
/// morrem com o processo.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct RememberedLimit {
    pub rate_limit_hits: u32,
    /// Segundos desde a época Unix do último 429.
    pub last_at: u64,
}

/// Lê a memória de 429, descartando o que passou de `RATE_LIMIT_MEMORY_SECS`.
/// Arquivo ausente ou ilegível é memória vazia: nunca impede um download.
pub fn load_remembered(path: &Path, now: u64) -> HashMap<String, RememberedLimit> {
    let Ok(bytes) = std::fs::read(path) else {
        return HashMap::new();
    };
    let map: HashMap<String, RememberedLimit> = serde_json::from_slice(&bytes).unwrap_or_default();
    map.into_iter()
        .filter(|(_, r)| now.saturating_sub(r.last_at) < RATE_LIMIT_MEMORY_SECS)
        .collect()
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Estado por host, para o ajuste sobreviver entre downloads.
///
/// Com `store`, os 429 também sobrevivem entre processos: o app e o CLI leem
/// e gravam o mesmo arquivo (`tools/host-limits.json` na pasta de dados).
#[derive(Debug, Default)]
pub struct ConcurrencyTuner {
    hosts: HashMap<String, HostStats>,
    store: Option<PathBuf>,
}

impl ConcurrencyTuner {
    /// Tuner que começa com os 429 ainda válidos gravados em `path`.
    pub fn with_store(path: PathBuf) -> Self {
        let mut hosts: HashMap<String, HostStats> = HashMap::new();
        for (host, r) in load_remembered(&path, unix_now()) {
            hosts.entry(host).or_default().rate_limit_hits = r.rate_limit_hits;
        }
        Self {
            hosts,
            store: Some(path),
        }
    }

    /// Grava os hosts com 429, somando aos que outro processo gravou.
    fn persist(&self, host: &str) {
        let Some(path) = &self.store else { return };
        let now = unix_now();
        let mut map = load_remembered(path, now);
        let hits = self.hosts.get(host).map_or(0, |s| s.rate_limit_hits);
        let entry = map.entry(host.to_string()).or_default();
        entry.rate_limit_hits = entry.rate_limit_hits.max(hits);
        entry.last_at = now;
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        // Escreve ao lado e renomeia: um processo lendo no meio nunca vê
        // meio arquivo.
        let tmp = path.with_extension(format!("json.{}", std::process::id()));
        if let Ok(bytes) = serde_json::to_vec_pretty(&map) {
            if std::fs::write(&tmp, bytes).is_ok() && std::fs::rename(&tmp, path).is_err() {
                let _ = std::fs::remove_file(&tmp);
            }
        }
    }

    pub fn record(&mut self, host: &str, concurrency: u32, throughput_bps: f64) {
        self.hosts
            .entry(host.to_string())
            .or_default()
            .record(concurrency, throughput_bps);
    }

    pub fn record_rate_limit(&mut self, host: &str) {
        self.hosts
            .entry(host.to_string())
            .or_default()
            .record_rate_limit();
        self.persist(host);
    }

    /// `ceiling` e o numero de Config: vira **teto**, nao valor fixo. O teto
    /// ainda e cortado pelo orcamento global de conexoes repartido entre os
    /// downloads ativos (`GLOBAL_CONNECTION_BUDGET`).
    pub fn suggest(&self, host: &str, current: u32, ceiling: u32) -> u32 {
        let shared = per_download_ceiling(GLOBAL_CONNECTION_BUDGET, active_downloads());
        self.suggest_with_ceiling(host, current, ceiling.min(shared))
    }

    /// `suggest` sem o orcamento global: a regra por host, pura.
    pub fn suggest_with_ceiling(&self, host: &str, current: u32, ceiling: u32) -> u32 {
        let teto = ceiling.clamp(MIN_N, MAX_N);
        match self.hosts.get(host) {
            Some(stats) => next_concurrency(stats, current).min(teto),
            None => current.min(teto),
        }
    }
}

#[cfg(test)]
impl ConcurrencyTuner {
    /// Esquece o que foi medido em `host`. Nos testes a porta de um servidor
    /// local pode ser reusada por outro teste do mesmo processo, e um 429
    /// antigo nessa porta cortaria os segmentos de quem veio depois.
    pub fn forget(&mut self, host: &str) {
        self.hosts.remove(host);
    }
}

/// Um tuner por processo, compartilhado entre o downloader direto e o yt-dlp.
///
/// O host é o mesmo nos dois caminhos (um 429 do `googlevideo.com` vale para
/// qualquer um deles), então dividir as estatísticas é o comportamento certo.
/// `tokio::sync::Mutex` porque é travado dentro de código async.
pub fn global() -> &'static tokio::sync::Mutex<ConcurrencyTuner> {
    static TUNER: std::sync::OnceLock<tokio::sync::Mutex<ConcurrencyTuner>> =
        std::sync::OnceLock::new();
    TUNER.get_or_init(|| {
        // Testes nunca leem nem gravam a pasta de dados de quem roda.
        let store = if cfg!(test) {
            None
        } else {
            crate::core::paths::app_data_dir().map(|d| d.join("tools").join("host-limits.json"))
        };
        tokio::sync::Mutex::new(
            store.map_or_else(ConcurrencyTuner::default, ConcurrencyTuner::with_store),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: f64 = 1_000_000.0;

    #[test]
    fn orcamento_global_reparte_entre_downloads_ativos() {
        assert_eq!(per_download_ceiling(24, 0), 16); // MAX_N
        assert_eq!(per_download_ceiling(24, 1), 16);
        assert_eq!(per_download_ceiling(24, 3), 8);
        assert_eq!(per_download_ceiling(24, 6), 4);
        assert_eq!(per_download_ceiling(24, 100), 1); // nunca abaixo de MIN_N
    }

    #[test]
    fn guard_conta_e_desconta() {
        let antes = active_downloads();
        {
            let _a = ActiveDownloadGuard::new();
            let _b = ActiveDownloadGuard::new();
            assert_eq!(active_downloads(), antes + 2);
        }
        assert_eq!(active_downloads(), antes);
    }

    #[test]
    fn sem_medicao_nenhuma_nao_mexe() {
        // Ajustar sem dado seria adivinhar, que e o que a feature veio evitar.
        let s = HostStats::default();
        assert_eq!(next_concurrency(&s, 4), 4);
    }

    #[test]
    fn sobe_quando_o_degrau_anterior_mostrou_ganho_real() {
        let mut s = HostStats::default();
        s.record(3, 3.0 * MB);
        s.record(4, 5.0 * MB); // +66%, bem acima do ruido
        assert_eq!(next_concurrency(&s, 4), 5);
    }

    #[test]
    fn nao_sobe_atras_de_ruido() {
        // 5% de diferenca se confunde com variacao normal de rede; subir por
        // isso so aumenta a chance de 429 sem ganhar velocidade.
        let mut s = HostStats::default();
        s.record(3, 5.0 * MB);
        s.record(4, 5.25 * MB);
        assert_eq!(next_concurrency(&s, 4), 4);
    }

    #[test]
    fn desce_quando_subir_piorou() {
        let mut s = HostStats::default();
        s.record(3, 8.0 * MB);
        s.record(4, 5.0 * MB); // subir custou velocidade
        assert_eq!(next_concurrency(&s, 4), 3);
    }

    #[test]
    fn rate_limit_derruba_rapido_e_ignora_o_throughput() {
        // Assimetria deliberada: subir errado custa 429 e retry, descer errado
        // custa um pouco de velocidade. E throughput bom nao significa nada se
        // o host esta recusando.
        let mut s = HostStats::default();
        s.record(8, 50.0 * MB);
        s.record_rate_limit();
        assert_eq!(next_concurrency(&s, 8), 4);

        s.record_rate_limit();
        assert_eq!(next_concurrency(&s, 8), 2);

        s.record_rate_limit();
        assert_eq!(next_concurrency(&s, 8), 1);
    }

    #[test]
    fn nunca_sai_da_faixa() {
        let mut s = HostStats::default();
        s.record(16, 100.0 * MB);
        s.record(15, 10.0 * MB);
        assert_eq!(next_concurrency(&s, 16), MAX_N, "nao passa do teto");

        let mut baixo = HostStats::default();
        for _ in 0..10 {
            baixo.record_rate_limit();
        }
        assert_eq!(next_concurrency(&baixo, 1), MIN_N, "nao vai abaixo de 1");
    }

    #[test]
    fn amostra_ruim_nao_apaga_a_boa() {
        // Interferencia local (outro download, wifi) nao e a capacidade real do
        // nivel. Por isso o melhor, e nao a media.
        let mut s = HostStats::default();
        s.record(3, 3.0 * MB);
        s.record(4, 5.0 * MB);
        s.record(4, 0.1 * MB); // pico de interferencia
        assert_eq!(next_concurrency(&s, 4), 5, "a amostra ruim nao deve mandar");
    }

    #[test]
    fn medicao_invalida_e_descartada() {
        let mut s = HostStats::default();
        s.record(4, f64::NAN);
        s.record(4, -1.0);
        s.record(4, 0.0);
        assert_eq!(next_concurrency(&s, 4), 4, "sem amostra valida, nao mexe");
    }

    #[test]
    fn o_numero_de_config_vira_teto_e_nao_valor_fixo() {
        // O ponto da feature: o usuario deixa de adivinhar um numero e passa a
        // declarar um limite.
        let mut t = ConcurrencyTuner::default();
        t.record("youtube.com", 3, 3.0 * MB);
        t.record("youtube.com", 4, 6.0 * MB);
        assert_eq!(t.suggest("youtube.com", 4, 16), 5, "abaixo do teto, sobe");
        assert_eq!(t.suggest("youtube.com", 4, 4), 4, "o teto segura");
    }

    fn store_temporario(nome: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "omniget-host-limits-{}-{}",
            nome,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("tools").join("host-limits.json")
    }

    #[test]
    fn um_429_vale_para_o_proximo_processo() {
        // O CLI é um processo por download: sem isto, a segunda execução
        // seguida voltava a abrir todas as conexões e levava outro 429.
        let path = store_temporario("proximo");
        let mut primeiro = ConcurrencyTuner::with_store(path.clone());
        assert_eq!(
            primeiro.suggest_with_ceiling("download.blender.org", 12, 16),
            12
        );
        primeiro.record_rate_limit("download.blender.org");

        let segundo = ConcurrencyTuner::with_store(path.clone());
        assert_eq!(
            segundo.suggest_with_ceiling("download.blender.org", 12, 16),
            6
        );
        assert_eq!(
            segundo.suggest_with_ceiling("bilibili.com", 12, 16),
            12,
            "outros hosts continuam inteiros"
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn memoria_de_429_vence_em_um_dia_e_arquivo_ruim_e_vazio() {
        let path = store_temporario("vence");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let agora = 1_800_000_000;
        let antigo = RememberedLimit {
            rate_limit_hits: 2,
            last_at: agora - RATE_LIMIT_MEMORY_SECS,
        };
        let recente = RememberedLimit {
            rate_limit_hits: 1,
            last_at: agora - 60,
        };
        let map: HashMap<&str, RememberedLimit> =
            [("velho.example", antigo), ("novo.example", recente)].into();
        std::fs::write(&path, serde_json::to_vec(&map).unwrap()).unwrap();
        let lido = load_remembered(&path, agora);
        assert!(!lido.contains_key("velho.example"));
        assert_eq!(lido.get("novo.example"), Some(&recente));

        std::fs::write(&path, b"{ nao e json").unwrap();
        assert!(load_remembered(&path, agora).is_empty());
        assert!(load_remembered(&path.with_file_name("nao-existe.json"), agora).is_empty());
        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn hosts_nao_contaminam_uns_aos_outros() {
        // 429 do YouTube nao pode reduzir a concorrencia do Bilibili.
        let mut t = ConcurrencyTuner::default();
        t.record_rate_limit("youtube.com");
        assert_eq!(t.suggest("youtube.com", 8, 16), 4);
        assert_eq!(t.suggest("bilibili.com", 8, 16), 8);
    }
}
