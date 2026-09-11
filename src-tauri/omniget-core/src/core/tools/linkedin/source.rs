//! Abre o export oficial do LinkedIn (Settings -> Data privacy -> Get a copy
//! of your data), aceitando tanto o `.zip` quanto a pasta ja extraida. Nada
//! aqui toca a rede: e leitura de arquivo local e ponto.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};

use super::csv::{norm, Table};

/// Origem dos CSVs. Guarda so o mapa de nomes; o conteudo e lido sob demanda
/// para nao carregar um `messages.csv` de dezenas de MB sem necessidade.
pub enum Source {
    Dir {
        files: HashMap<String, PathBuf>,
    },
    Zip {
        path: PathBuf,
        files: HashMap<String, String>,
    },
}

/// Nome do arquivo sem pasta e sem extensao, normalizado.
fn key(entry: &str) -> String {
    let base = entry.rsplit(['/', '\\']).next().unwrap_or(entry);
    let stem = base.rsplit_once('.').map(|(s, _)| s).unwrap_or(base);
    norm(stem)
}

fn is_csv(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".csv")
}

/// Tudo que o export tem dentro, arquivo por arquivo (inclui imagens), para
/// o inventario e para a checagem de foto de perfil.
pub struct Opened {
    pub source: Source,
    pub entries: Vec<String>,
}

impl Source {
    pub fn open(path: &str) -> Result<Opened> {
        let p = Path::new(path);
        if !p.exists() {
            return Err(anyhow!("nao achei o caminho do export: {}", path));
        }
        let mut entries = Vec::new();
        let source = if p.is_dir() {
            let mut files = HashMap::new();
            for e in walkdir::WalkDir::new(p)
                .max_depth(4)
                .into_iter()
                .filter_map(std::result::Result::ok)
            {
                if !e.file_type().is_file() {
                    continue;
                }
                let name = e.file_name().to_string_lossy().to_string();
                let rel = e
                    .path()
                    .strip_prefix(p)
                    .unwrap_or(e.path())
                    .to_string_lossy()
                    .replace('\\', "/");
                entries.push(rel);
                if is_csv(&name) {
                    files
                        .entry(key(&name))
                        .or_insert_with(|| e.path().to_path_buf());
                }
            }
            Source::Dir { files }
        } else {
            let f = std::fs::File::open(p)?;
            let mut zip = zip::ZipArchive::new(f)?;
            let mut files = HashMap::new();
            for i in 0..zip.len() {
                let e = zip.by_index(i)?;
                if !e.is_file() {
                    continue;
                }
                let name = e.name().to_string();
                entries.push(name.clone());
                if is_csv(&name) {
                    files.entry(key(&name)).or_insert(name);
                }
            }
            Source::Zip {
                path: p.to_path_buf(),
                files,
            }
        };
        let empty = match &source {
            Source::Dir { files } => files.is_empty(),
            Source::Zip { files, .. } => files.is_empty(),
        };
        if empty {
            return Err(anyhow!(
                "nenhum CSV no export; aponte para o zip do LinkedIn ou para a pasta extraida"
            ));
        }
        entries.sort();
        Ok(Opened { source, entries })
    }

    /// Nomes normalizados dos CSVs disponiveis.
    pub fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = match self {
            Source::Dir { files } => files.keys().cloned().collect(),
            Source::Zip { files, .. } => files.keys().cloned().collect(),
        };
        v.sort();
        v
    }

    /// Le um CSV pelo nome (aceita apelidos). Texto em UTF-8 tolerante.
    pub fn read(&self, candidates: &[&str]) -> Option<String> {
        let wanted: Vec<String> = candidates.iter().map(|c| key(c)).collect();
        match self {
            Source::Dir { files } => {
                let path = wanted.iter().find_map(|w| files.get(w))?;
                std::fs::read(path)
                    .ok()
                    .map(|b| String::from_utf8_lossy(&b).into_owned())
            }
            Source::Zip { path, files } => {
                let entry = wanted.iter().find_map(|w| files.get(w))?;
                let f = std::fs::File::open(path).ok()?;
                let mut zip = zip::ZipArchive::new(f).ok()?;
                let mut e = zip.by_name(entry).ok()?;
                let mut buf = Vec::new();
                e.read_to_end(&mut buf).ok()?;
                Some(String::from_utf8_lossy(&buf).into_owned())
            }
        }
    }

    /// Le e ja parseia. `None` quando o arquivo nao veio no export.
    pub fn table(&self, candidates: &[&str]) -> Option<Table> {
        self.read(candidates).map(|s| Table::parse(&s))
    }

    /// Igual ao `table`, mas devolve tabela vazia em vez de `None`.
    pub fn table_or_empty(&self, candidates: &[&str]) -> Table {
        self.table(candidates).unwrap_or_default()
    }
}

#[cfg(test)]
pub(crate) mod fixture {
    use std::io::Write;
    use std::path::PathBuf;

    /// Export sintetico bagunçado de proposito: BOM, preambulo "Notes:",
    /// campo com virgula e aspas, quebra de linha dentro da mensagem, coluna
    /// faltando e arquivo faltando (nao tem `Comments.csv`).
    pub fn files() -> Vec<(&'static str, String)> {
        vec![
            (
                "Connections.csv",
                concat!(
                    "\u{feff}Notes:\n",
                    "\"When exporting your connection data, you may notice...\"\n",
                    "\n",
                    "First Name,Last Name,URL,Email Address,Company,Position,Connected On\n",
                    "Ana,Silva,https://www.linkedin.com/in/anasilva,ana@acme.com,Acme,\"Engenheira de Software, Senior\",06 Aug 2021\n",
                    "Bruno,Souza,https://www.linkedin.com/in/brunosouza,,Acme,Designer,12 Sep 2021\n",
                    "Ana,Silva,https://www.linkedin.com/in/anasilva,,Acme,Engenheira,06 Aug 2021\n",
                    "Carla,Dias,https://www.linkedin.com/in/carladias,carla@beta.io,Beta,Designer,03 Mar 2022\n",
                    "Diego,Rocha,https://www.linkedin.com/in/diegorocha,,Beta,Product Manager,15 Mar 2022\n"
                )
                .to_string(),
            ),
            (
                "messages.csv",
                concat!(
                    "CONVERSATION ID,CONVERSATION TITLE,FROM,SENDER PROFILE URL,TO,DATE,SUBJECT,CONTENT\n",
                    "c1,,Ana Silva,https://www.linkedin.com/in/anasilva,Tonho Dev,2021-08-07 10:00:00 UTC,Oi,\"Bom dia!\nTudo certo?\"\n",
                    "c1,,Tonho Dev,https://www.linkedin.com/in/tonhodev,Ana Silva,2021-08-07 10:05:00 UTC,,\"Tudo, e ai?\"\n",
                    "c1,,Ana Silva,https://www.linkedin.com/in/anasilva,Tonho Dev,2021-08-07 10:06:00 UTC,,\n",
                    "c2,,Tonho Dev,https://www.linkedin.com/in/tonhodev,Carla Dias,2022-03-04 09:00:00 UTC,Vaga,\"Oi, Carla\"\n",
                    "c2,,Carla Dias,https://www.linkedin.com/in/carladias,Tonho Dev,2022-03-04 09:30:00 UTC,,Fechado\n"
                )
                .to_string(),
            ),
            (
                "Shares.csv",
                concat!(
                    "Date,ShareLink,ShareCommentary,MediaUrl,Visibility\n",
                    "2021-08-10 12:00:00,https://www.linkedin.com/feed/update/1,\"Post um, com virgula\",,MEMBER_NETWORK\n",
                    "2022-03-05 12:00:00,https://www.linkedin.com/feed/update/2,Post dois,https://media.licdn.com/x.jpg,PUBLIC\n"
                )
                .to_string(),
            ),
            (
                "Reactions.csv",
                "Date,Type,Link\n2021-09-01 08:00:00,LIKE,https://x/1\n2022-03-02 08:00:00,PRAISE,https://x/2\n2022-03-03 08:00:00,LIKE,https://x/3\n".to_string(),
            ),
            (
                "Invitations.csv",
                "From,To,Sent At,Message,Direction\nTonho Dev,Ana Silva,2021-08-01 10:00:00,,OUTGOING\nCarla Dias,Tonho Dev,2022-03-01 10:00:00,,INCOMING\n".to_string(),
            ),
            (
                "Profile.csv",
                concat!(
                    "First Name,Last Name,Headline,Summary,Industry,Geo Location,Websites\n",
                    "Tonho,Dev,\"Dev de Rust e Svelte que gosta de ferramenta local\",\"Trabalho com aplicativos de desktop ha dez anos.\",Software Development,\"Sao Paulo, Brasil\",[MY_WEBSITE:https://omniget.wtf]\n"
                )
                .to_string(),
            ),
            (
                "Positions.csv",
                concat!(
                    "Company Name,Title,Description,Location,Started On,Finished On\n",
                    "Acme,Engenheiro de Software,\"Cuido do app de desktop.\",Sao Paulo,Jan 2019,\n",
                    "Beta,Dev Junior,,Sao Paulo,Jan 2017,Dec 2018\n"
                )
                .to_string(),
            ),
            (
                "Education.csv",
                "School Name,Start Date,End Date,Degree Name\nUniversidade X,2012,2016,Bacharel\n".to_string(),
            ),
            // Sem cabecalho conhecido de proposito: so uma coluna.
            ("Skills.csv", "Name\nRust\nSvelte\nTypeScript\n".to_string()),
            (
                "Endorsement_Received_Info.csv",
                "Endorsement Date,Skill Name,Endorser First Name,Endorser Last Name,Endorsement Status\n2021-10-01,Rust,Ana,Silva,ACCEPTED\n".to_string(),
            ),
            (
                "Recommendations_Received.csv",
                "First Name,Last Name,Company,Job Title,Text,Creation Date,Status\nAna,Silva,Acme,Engenheira,\"Otimo colega.\",2021-11-02,VISIBLE\n".to_string(),
            ),
            (
                "Ad_Targeting.csv",
                concat!(
                    "Notes:\n",
                    "\"These are the targeting criteria advertisers used.\"\n",
                    "\n",
                    "Member Age,Company Names,Job Titles,Interests\n",
                    "\"25 to 34\",\"Acme | Beta\",\"Software Engineer | Developer\",\"Rust | Svelte | Linux\"\n"
                )
                .to_string(),
            ),
            (
                "Company Follows.csv",
                "Organization,Followed On\nAcme,2021-01-01 10:00:00\n".to_string(),
            ),
        ]
    }

    /// Escreve o export sintetico numa pasta temporaria unica.
    pub fn dir() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "omniget-li-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        let base = root.join("Basic_LinkedInDataExport_2026");
        let _ = std::fs::create_dir_all(&base);
        for (name, body) in files() {
            let _ = std::fs::write(base.join(name), body);
        }
        base
    }

    /// Mesmo export, mas dentro de um zip (com uma pasta interna).
    pub fn zip() -> PathBuf {
        let dir = dir();
        let path = dir.with_extension("zip");
        let Ok(f) = std::fs::File::create(&path) else {
            return path;
        };
        let mut z = zip::ZipWriter::new(f);
        let opts: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, body) in files() {
            if z.start_file(format!("Basic_LinkedInDataExport/{}", name), opts)
                .is_ok()
            {
                let _ = z.write_all(body.as_bytes());
            }
        }
        let _ = z.finish();
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abre_pasta_e_acha_os_csvs() {
        let dir = fixture::dir();
        let o = Source::open(&dir.to_string_lossy()).expect("abre pasta");
        assert!(o.source.names().contains(&"connections".to_string()));
        assert!(o.source.names().contains(&"companyfollows".to_string()));
        assert!(!o.source.names().contains(&"comments".to_string()));
        let t = o.source.table_or_empty(&["Connections.csv"]);
        assert_eq!(t.len(), 5);
    }

    #[test]
    fn abre_zip_com_pasta_interna() {
        let path = super::fixture::zip();
        let o = Source::open(&path.to_string_lossy()).expect("abre zip");
        let t = o.source.table_or_empty(&["Connections.csv"]);
        assert_eq!(t.len(), 5);
        assert_eq!(
            t.get(&t.rows[0], &["position"]),
            "Engenheira de Software, Senior"
        );
        assert!(o.entries.iter().any(|e| e.ends_with("messages.csv")));
    }

    #[test]
    fn caminho_inexistente_da_erro() {
        assert!(Source::open("/caminho/que/nao/existe/export.zip").is_err());
    }

    #[test]
    fn arquivo_faltando_devolve_tabela_vazia() {
        let dir = fixture::dir();
        let o = Source::open(&dir.to_string_lossy()).expect("abre pasta");
        assert!(o.source.table(&["Comments.csv"]).is_none());
        assert!(o.source.table_or_empty(&["Comments.csv"]).is_empty());
    }
}
