//! The rule catalogue: every stable code the guard can emit, with its
//! validator, default severity and a one-line description in English and
//! Portuguese (the UI shows the description; the finding carries the detail).
//!
//! Codes keep the original claude-code-templates numbering where a rule was
//! ported (`STRUCT_E001`, `SEM_E001`…`SEM_E019`, `REF_E002`…) and add new
//! numbers for what the original did not check. A code never changes meaning.

use serde::Serialize;

use super::model::{Severity, Validator};

#[derive(Debug, Clone, Serialize)]
pub struct Rule {
    pub code: &'static str,
    pub validator: Validator,
    pub severity: Severity,
    pub en: &'static str,
    pub pt: &'static str,
}

use Severity::*;
use Validator::*;

macro_rules! r {
    ($c:expr, $v:expr, $s:expr, $en:expr, $pt:expr) => {
        Rule {
            code: $c,
            validator: $v,
            severity: $s,
            en: $en,
            pt: $pt,
        }
    };
}

pub const RULES: &[Rule] = &[
    // ---------------------------------------------------------- structural
    r!("STRUCT_E001", Structural, Medium, "File is empty", "Arquivo vazio"),
    r!("STRUCT_E002", Structural, Medium, "Frontmatter is malformed", "Frontmatter malformado"),
    r!("STRUCT_E003", Structural, Medium, "File is larger than 100 KB", "Arquivo maior que 100 KB"),
    r!("STRUCT_E004", Structural, High, "File is not valid UTF-8", "Arquivo não é UTF-8 válido"),
    r!("STRUCT_E005", Structural, High, "File contains NUL bytes (binary content)", "Arquivo tem bytes nulos (conteúdo binário)"),
    r!("STRUCT_E006", Structural, Medium, "Required field is missing for this kind", "Campo obrigatório ausente para este tipo"),
    r!("STRUCT_E007", Structural, Low, "Description is not a string", "Descrição não é texto"),
    r!("STRUCT_E008", Structural, High, "JSON does not parse", "JSON inválido"),
    r!("STRUCT_E009", Structural, Medium, "JSON does not have the shape this kind needs", "JSON sem a forma esperada para este tipo"),
    r!("STRUCT_W001", Structural, Medium, "No frontmatter: the tool will not load it", "Sem frontmatter: a ferramenta não vai carregar"),
    r!("STRUCT_W002", Structural, Low, "File is close to the 100 KB limit", "Arquivo perto do limite de 100 KB"),
    r!("STRUCT_W003", Structural, Low, "Description is too short", "Descrição curta demais"),
    r!("STRUCT_W004", Structural, Low, "Description is too long for a picker", "Descrição longa demais para o seletor"),
    r!("STRUCT_W005", Structural, Low, "Tools field is empty", "Campo de tools vazio"),
    r!("STRUCT_W006", Structural, Low, "Unknown tool names for this tool", "Nomes de tool desconhecidos para esta ferramenta"),
    r!("STRUCT_W008", Structural, Low, "Unknown model id", "Modelo desconhecido"),
    r!("STRUCT_W009", Structural, Low, "Body is very short", "Corpo muito curto"),
    r!("STRUCT_W011", Structural, Low, "Too many sections (context bloat)", "Seções demais (incha o contexto)"),
    r!("STRUCT_W012", Structural, Low, "Written in another tool's format", "Formato de outra ferramenta"),
    r!("STRUCT_W013", Structural, Low, "Name does not match the folder or is not kebab-case", "Nome não bate com a pasta ou não é kebab-case"),
    r!("STRUCT_I005", Structural, Info, "Native format of another tool (will be converted)", "Formato nativo de outra ferramenta (será convertido)"),
    r!("STRUCT_I006", Structural, Info, "No frontmatter (optional for this kind)", "Sem frontmatter (opcional para este tipo)"),
    // ----------------------------------------------------------- integrity
    r!("INT_E001", Integrity, Critical, "SHA-256 does not match the expected value", "SHA-256 não bate com o esperado"),
    r!("INT_W001", Integrity, High, "Expected file is missing", "Arquivo esperado ausente"),
    r!("INT_W002", Integrity, Medium, "File not listed in the expected set", "Arquivo fora da lista esperada"),
    r!("INT_I001", Integrity, Info, "No expected hashes: integrity not verified", "Sem hashes esperados: integridade não verificada"),
    r!("INT_I002", Integrity, Info, "All hashes match", "Todos os hashes batem"),
    // ------------------------------------------------------------ semantic
    r!("SEM_E001", Semantic, Critical, "Prompt injection: ignore previous instructions", "Injeção de prompt: ignorar instruções anteriores"),
    r!("SEM_E002", Semantic, High, "Asks to reveal the system prompt or hidden instructions", "Pede para revelar o prompt de sistema ou instruções ocultas"),
    r!("SEM_E003", Semantic, Low, "Role redefinition (\"you are now a…\")", "Redefinição de papel (\"agora você é…\")"),
    r!("SEM_E004", Semantic, Medium, "Tells the model to execute given code", "Manda o modelo executar código dado"),
    r!("SEM_E005", Semantic, High, "Credential harvesting wording", "Texto de coleta de credenciais"),
    r!("SEM_E006", Semantic, Low, "Asks to open a shell", "Pede para abrir um shell"),
    r!("SEM_E007", Semantic, High, "Asks to disable or bypass security", "Pede para desligar ou contornar a segurança"),
    r!("SEM_E008", Semantic, Medium, "Unconditional obedience", "Obediência incondicional"),
    r!("SEM_E009", Semantic, High, "Context wipe (\"forget everything\")", "Apagar contexto (\"esqueça tudo\")"),
    r!("SEM_E010", Semantic, Medium, "Self-modification request", "Pedido de automodificação"),
    r!("SEM_E011", Semantic, High, "Hard-coded password", "Senha no texto"),
    r!("SEM_E012", Semantic, High, "Hard-coded API key", "Chave de API no texto"),
    r!("SEM_E013", Semantic, High, "Hard-coded secret or token", "Segredo ou token no texto"),
    r!("SEM_E014", Semantic, High, "<script> tag (XSS)", "Tag <script> (XSS)"),
    r!("SEM_E015", Semantic, Medium, "<iframe> tag", "Tag <iframe>"),
    r!("SEM_E016", Semantic, High, "javascript: URL (XSS)", "URL javascript: (XSS)"),
    r!("SEM_E017", Semantic, Medium, "Inline event handler (XSS)", "Handler de evento inline (XSS)"),
    r!("SEM_E018", Semantic, Medium, "onerror handler (XSS)", "Handler onerror (XSS)"),
    r!("SEM_E019", Semantic, Critical, "Destructive command in the text", "Comando destrutivo no texto"),
    r!("SEM_E020", Semantic, Critical, "Credential exfiltration: sensitive file sent out", "Exfiltração de credencial: arquivo sensível enviado para fora"),
    r!("SEM_E021", Semantic, Critical, "Known-format secret (cloud key, token, private key)", "Segredo de formato conhecido (chave de nuvem, token, chave privada)"),
    r!("SEM_E022", Semantic, Medium, "Mentions a credential location (~/.ssh, .aws, keychain)", "Menciona local de credencial (~/.ssh, .aws, keychain)"),
    r!("SEM_E023", Semantic, Medium, "Tells the model to send data to a webhook or paste site", "Manda enviar dados para webhook ou site de paste"),
    r!("SEM_E024", Semantic, High, "Hidden text: invisible or bidi Unicode, or instructions in HTML comments", "Texto oculto: Unicode invisível/bidi ou instruções em comentário HTML"),
    r!("SEM_E025", Semantic, Low, "Long opaque base64 payload", "Carga base64 longa e opaca"),
    r!("SEM_W001", Semantic, Low, "Role pretending", "Fingir ser outro"),
    r!("SEM_W002", Semantic, Medium, "Known jailbreak terminology", "Termo conhecido de jailbreak"),
    r!("SEM_W003", Semantic, Low, "Raw output request", "Pedido de saída crua"),
    r!("SEM_W004", Semantic, Low, "\"Repeat after me\" (prompt leakage)", "\"Repita comigo\" (vazamento de prompt)"),
    r!("SEM_W005", Semantic, Medium, "Over-permissive wording", "Texto permissivo demais"),
    r!("SEM_W006", Semantic, Low, "Unrestricted tool grant (Bash without a pattern)", "Tool liberada sem restrição (Bash sem padrão)"),
    // ----------------------------------------------------------- reference
    r!("REF_E002", Reference, Medium, "Blocked URL scheme (file, ftp, data, javascript, vbscript)", "Esquema de URL bloqueado (file, ftp, data, javascript, vbscript)"),
    r!("REF_E004", Reference, Medium, "Private network address (SSRF risk)", "Endereço de rede privada (risco de SSRF)"),
    r!("REF_E005", Reference, High, "Dangerous scheme in a Markdown link", "Esquema perigoso em link Markdown"),
    r!("REF_E006", Reference, High, "Cloud metadata endpoint", "Endpoint de metadados da nuvem"),
    r!("REF_W002", Reference, Low, "Plain HTTP (HTTPS recommended)", "HTTP sem TLS (use HTTPS)"),
    r!("REF_W003", Reference, Low, "Loopback address", "Endereço de loopback"),
    r!("REF_W004", Reference, Low, "Suspicious top-level domain", "Domínio de topo suspeito"),
    r!("REF_W006", Reference, Low, "Large data: URI image", "Imagem data: grande"),
    r!("REF_W007", Reference, Medium, "Paste site or request catcher", "Site de paste ou coletor de requisições"),
    r!("REF_W008", Reference, Low, "URL shortener hides the destination", "Encurtador de URL esconde o destino"),
    r!("REF_W009", Reference, Low, "Raw public IP instead of a host name", "IP público cru em vez de nome"),
    // ---------------------------------------------------------- provenance
    r!("PROV_W001", Provenance, Low, "No source information", "Sem informação de origem"),
    r!("PROV_W004", Provenance, Low, "Repository is not on a recognized host", "Repositório fora de host conhecido"),
    r!("PROV_W006", Provenance, Medium, "Repository URL uses HTTP", "URL do repositório usa HTTP"),
    r!("PROV_W007", Provenance, Low, "Version is not semver", "Versão não segue semver"),
    r!("PROV_W008", Provenance, Low, "Source is not pinned to a commit", "Origem não fixada num commit"),
    r!("PROV_W009", Provenance, Low, "No license", "Sem licença"),
    r!("PROV_I002", Provenance, Info, "Source pinned", "Origem fixada"),
    // ------------------------------------------------------------- command
    r!("CMD_E001", Command, Critical, "rm -rf on /, home or a wildcard", "rm -rf em /, home ou curinga"),
    r!("CMD_E002", Command, Critical, "Download piped into an interpreter (curl | sh)", "Download direto para interpretador (curl | sh)"),
    r!("CMD_E003", Command, High, "Downloads a file and executes it", "Baixa um arquivo e executa"),
    r!("CMD_E004", Command, Critical, "Disk or system destruction", "Destruição de disco ou sistema"),
    r!("CMD_E005", Command, High, "Reads credentials (~/.ssh, .aws, keychain…)", "Lê credenciais (~/.ssh, .aws, keychain…)"),
    r!("CMD_E006", Command, Critical, "Sends sensitive data over the network (exfiltration)", "Envia dado sensível pela rede (exfiltração)"),
    r!("CMD_W001", Command, Low, "Recursive forced delete", "Remoção recursiva forçada"),
    r!("CMD_W002", Command, High, "Runs with elevated privileges (sudo)", "Roda com privilégio elevado (sudo)"),
    r!("CMD_W003", Command, Medium, "Sends data to a webhook or paste service", "Envia dados para webhook ou serviço de paste"),
    r!("CMD_W004", Command, Info, "Network access", "Acesso à rede"),
    r!("CMD_W005", Command, High, "Obfuscated or dynamic execution (eval, base64 | sh)", "Execução ofuscada ou dinâmica (eval, base64 | sh)"),
    r!("CMD_W006", Command, Medium, "Destructive git operation", "Operação destrutiva de git"),
    r!("CMD_W007", Command, High, "Persistence (cron, launchd, shell rc, authorized_keys)", "Persistência (cron, launchd, rc do shell, authorized_keys)"),
    r!("CMD_W008", Command, Medium, "World-writable permissions or recursive chown", "Permissão aberta para todos ou chown recursivo"),
    r!("CMD_W009", Command, Low, "Runs a package fetched at run time (npx -y, uvx, @latest)", "Roda pacote baixado na hora (npx -y, uvx, @latest)"),
    r!("CMD_W010", Command, Low, "Kills processes by name", "Mata processos por nome"),
    r!("CMD_W011", Command, High, "Disables OS security (Gatekeeper, SELinux, firewall, quarantine)", "Desliga segurança do SO (Gatekeeper, SELinux, firewall, quarentena)"),
    r!("CMD_W012", Command, Medium, "rm -rf on a variable that may be empty", "rm -rf numa variável que pode estar vazia"),
    r!("CMD_W013", Command, Low, "Dumps the environment", "Despeja as variáveis de ambiente"),
    r!("CMD_W014", Command, Medium, "Reboots or shuts down the machine", "Reinicia ou desliga a máquina"),
    r!("CMD_W015", Command, Medium, "eval of dynamic text", "eval de texto dinâmico"),
    r!("CMD_I001", Command, Info, "Runs automatically when invoked", "Roda sozinho quando invocado"),
    // -------------------------------------------------------------- config
    r!("HOOK_E001", Config, Medium, "Hook without a command", "Hook sem comando"),
    r!("HOOK_W001", Config, Medium, "Uses an environment variable that does not exist: this hook does not work (input arrives as JSON on stdin)", "Usa variável de ambiente que não existe: este hook não funciona (a entrada chega como JSON no stdin)"),
    r!("HOOK_W002", Config, Low, "Agent/prompt handler: calls a model on every event", "Handler agent/prompt: chama um modelo a cada evento"),
    r!("HOOK_W003", Config, Info, "Event name unknown for this tool", "Evento desconhecido para esta ferramenta"),
    r!("HOOK_W004", Config, Info, "Conditional hook (`if`)", "Hook condicional (`if`)"),
    r!("HOOK_W005", Config, High, "Supporting file written outside the tool folder", "Arquivo de apoio escrito fora da pasta da ferramenta"),
    r!("HOOK_W006", Config, Low, "Supporting file declared but not shipped", "Arquivo de apoio declarado mas ausente"),
    r!("HOOK_W007", Config, Low, "Obsolete hook shape", "Formato de hook obsoleto"),
    r!("HOOK_W008", Config, Medium, "Runs on every tool call (no matcher) with network access", "Roda em toda chamada de tool (sem matcher) com rede"),
    r!("HOOK_I001", Config, Info, "Ships an executable supporting file", "Traz arquivo de apoio executável"),
    r!("MCP_W001", Config, High, "Secret written in the config", "Segredo escrito na config"),
    r!("MCP_W002", Config, Medium, "Remote server over plain HTTP", "Servidor remoto sem TLS"),
    r!("MCP_W004", Config, Low, "Unpinned package version", "Versão de pacote não fixada"),
    r!("MCP_W005", Config, High, "Docker with privileged mode or host root mounted", "Docker privilegiado ou com a raiz do host montada"),
    r!("MCP_W006", Config, Medium, "Server launched through a shell", "Servidor iniciado via shell"),
    r!("MCP_W007", Config, Low, "Placeholder path or value must be edited", "Caminho ou valor de exemplo precisa ser editado"),
    r!("MCP_W008", Config, Medium, "Tools auto-approved", "Tools aprovadas automaticamente"),
    r!("MCP_I001", Config, Info, "Needs a secret filled in", "Precisa preencher um segredo"),
    r!("SET_W001", Config, Medium, "Permission rule too broad", "Regra de permissão ampla demais"),
    r!("SET_W002", Config, High, "Bypasses permission prompts", "Pula os pedidos de permissão"),
    r!("SET_W003", Config, Medium, "Enables every project MCP server", "Habilita todos os MCPs do projeto"),
    r!("SET_W004", Config, Medium, "Redirects API traffic (and your key) to another host", "Redireciona o tráfego da API (e sua chave) para outro host"),
    r!("SET_W005", Config, High, "Secret written in env", "Segredo escrito no env"),
    r!("SET_W006", Config, Info, "Runs a helper command", "Roda um comando auxiliar"),
    r!("SET_W007", Config, Low, "Exports telemetry to a third party", "Exporta telemetria para terceiro"),
    r!("SET_W008", Config, Info, "Status line command", "Comando de status line"),
    // --------------------------------------------------------- skillspector
    r!("SPEC_E001", Skillspector, Critical, "SkillSpector recommends not installing", "SkillSpector recomenda não instalar"),
    r!("SPEC_W001", Skillspector, Medium, "SkillSpector finding", "Achado do SkillSpector"),
    r!("SPEC_I001", Skillspector, Info, "SkillSpector could not run", "SkillSpector não rodou"),
];

pub fn rule(code: &str) -> Option<&'static Rule> {
    RULES.iter().find(|r| r.code == code)
}

/// Default severity of a code (Medium for an unknown code, never panics).
pub fn sev(code: &str) -> Severity {
    rule(code).map(|r| r.severity).unwrap_or(Severity::Medium)
}

#[cfg(test)]
mod tests {
    #[test]
    fn codes_are_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for r in super::RULES {
            assert!(seen.insert(r.code), "duplicate {}", r.code);
        }
    }
}
