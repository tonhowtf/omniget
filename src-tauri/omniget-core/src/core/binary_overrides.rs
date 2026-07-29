//! Binarios que o usuario apontou, em vez dos que o OmniGet gerencia.
//!
//! Pedido na issue #222: quem ja tem yt-dlp, FFmpeg ou PDFium instalado nao
//! quer uma segunda copia dentro do diretorio do app. Ate aqui so o PDFium
//! aceitava arquivo customizado, e mesmo assim **copiava** — o que resolve
//! metade do problema e cria a outra metade, porque a copia envelhece sozinha.
//!
//! Aqui o caminho e guardado e usado no lugar. Se o usuario atualizar o yt-dlp
//! dele pelo gerenciador de pacotes, o OmniGet passa a usar a versao nova sem
//! fazer nada.
//!
//! O arquivo proprio, e nao um campo no `settings.json`, por dois motivos: nao
//! obriga migracao para todo mundo por causa de poucos, e um caminho quebrado
//! aqui nao pode impedir as configuracoes de carregar.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const OVERRIDES_FILE: &str = "binary-overrides.json";

/// Nomes aceitos. Fechado de proposito: aceitar qualquer string deixaria o
/// arquivo virar depósito de chave escrita errada que nunca tem efeito.
pub const SUPPORTED: [&str; 3] = ["yt-dlp", "FFmpeg", "PDFium"];

pub fn is_supported(name: &str) -> bool {
    SUPPORTED.contains(&name)
}

fn file_path() -> Option<PathBuf> {
    super::paths::app_data_dir().map(|d| d.join(OVERRIDES_FILE))
}

/// Le o mapa do disco. Um arquivo ilegivel vale como vazio: perder o override
/// e um incomodo, nao abrir o app e um problema.
pub fn load() -> BTreeMap<String, String> {
    let Some(path) = file_path() else {
        return BTreeMap::new();
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return BTreeMap::new();
    };
    match serde_json::from_str(&raw) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("[overrides] {} ilegivel, ignorando: {}", path.display(), e);
            BTreeMap::new()
        }
    }
}

fn save(map: &BTreeMap<String, String>) -> std::io::Result<()> {
    let Some(path) = file_path() else {
        return Err(std::io::Error::other("sem diretorio de dados"));
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(map).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, &path)
}

/// Caminho que o usuario apontou para esta ferramenta, se ainda existir.
///
/// A checagem de existencia acontece aqui, e nao so na hora de gravar: o
/// binario pode ter sido removido depois, e nesse caso o certo e voltar para o
/// gerenciado em silencio, nao falhar o download.
pub fn get(name: &str) -> Option<PathBuf> {
    let raw = load().get(name)?.clone();
    let path = PathBuf::from(raw);
    if path.exists() {
        Some(path)
    } else {
        tracing::warn!(
            "[overrides] {} aponta para {} que nao existe mais — usando o gerenciado",
            name,
            path.display()
        );
        None
    }
}

/// Motivo pelo qual um caminho nao serve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rejeicao {
    NaoSuportado,
    NaoExiste,
    NaoEArquivo,
}

impl Rejeicao {
    pub fn mensagem(&self, name: &str) -> String {
        match self {
            Rejeicao::NaoSuportado => {
                format!("{name} nao aceita um caminho customizado")
            }
            Rejeicao::NaoExiste => "O arquivo escolhido nao existe".to_string(),
            Rejeicao::NaoEArquivo => "O caminho escolhido e uma pasta, nao um arquivo".to_string(),
        }
    }
}

/// Valida sem tocar em disco de configuracao. Separado de `set` para poder ser
/// testado sem estado global.
pub fn validar(name: &str, path: &Path) -> Result<(), Rejeicao> {
    if !is_supported(name) {
        return Err(Rejeicao::NaoSuportado);
    }
    if !path.exists() {
        return Err(Rejeicao::NaoExiste);
    }
    if !path.is_file() {
        return Err(Rejeicao::NaoEArquivo);
    }
    Ok(())
}

pub fn set(name: &str, path: &Path) -> Result<(), String> {
    validar(name, path).map_err(|r| r.mensagem(name))?;
    let mut map = load();
    map.insert(name.to_string(), path.to_string_lossy().to_string());
    save(&map).map_err(|e| e.to_string())
}

pub fn clear(name: &str) -> Result<(), String> {
    let mut map = load();
    if map.remove(name).is_none() {
        return Ok(());
    }
    save(&map).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempfile(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("omniget-ovr-{tag}-{}", std::process::id()));
        std::fs::write(&p, b"stub").expect("cria arquivo");
        p
    }

    #[test]
    fn so_as_tres_ferramentas_conhecidas_sao_aceitas() {
        assert!(is_supported("yt-dlp"));
        assert!(is_supported("FFmpeg"));
        assert!(is_supported("PDFium"));
        // Nome escrito diferente nao pode virar entrada morta no arquivo.
        assert!(!is_supported("ffmpeg"));
        assert!(!is_supported("aria2c"));
    }

    #[test]
    fn caminho_inexistente_e_recusado_na_hora_de_escolher() {
        // O ponto: falhar aqui, com o seletor de arquivo aberto, e muito melhor
        // do que falhar no meio de um download.
        let r = validar("yt-dlp", Path::new("/nao/existe/yt-dlp"));
        assert_eq!(r, Err(Rejeicao::NaoExiste));
    }

    #[test]
    fn pasta_nao_serve_como_binario() {
        let dir = std::env::temp_dir();
        assert_eq!(validar("FFmpeg", &dir), Err(Rejeicao::NaoEArquivo));
    }

    #[test]
    fn ferramenta_desconhecida_e_recusada_antes_de_olhar_o_disco() {
        let f = tempfile("desconhecida");
        assert_eq!(validar("aria2c", &f), Err(Rejeicao::NaoSuportado));
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn arquivo_de_verdade_passa() {
        let f = tempfile("valido");
        assert_eq!(validar("yt-dlp", &f), Ok(()));
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn a_mensagem_diz_o_que_houve_e_nao_so_que_falhou() {
        // Regra de copy do projeto: erro fala o que aconteceu.
        assert!(Rejeicao::NaoExiste
            .mensagem("yt-dlp")
            .contains("nao existe"));
        assert!(Rejeicao::NaoEArquivo.mensagem("yt-dlp").contains("pasta"));
    }
}
