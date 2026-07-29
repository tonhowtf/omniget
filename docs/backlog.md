# Backlog — OmniGet

Itens abertos por ordem de retorno. Numeração contínua; concluídos não são renumerados.

**Legenda.** Custo P/M/G · Impacto 1–5 · Status: `aberto` · `bloqueado` · `em análise`

**Concluído em v0.7.7:** B21–B31 (11 itens).
**Concluído e aguardando release:** B47, B48, B41 (PR #204) · B50, B51, B36 (PR #205).

---

## Onda 1 (2026-07-29) — concluída

B50, B51 e B36 entregues na PR #205, CI 6/6 verde, mergeada. Testes 460 → 467.

### B50 — Baseline de clippy commitado ✅
Clippy virou portão de verdade: `src-tauri/clippy-baseline.json` congela 177 warnings em 51 lints e o CI reprova só o que é novo. Chave `crate|lint`, sem arquivo nem linha — mover código não reprova, só introduzir warning reprova. **Baseline é por plataforma**: `#[cfg(target_os)]` faz o clippy ver código diferente em cada SO, então roda só no ubuntu e o script recusa comparação cruzada. Baseline por SO fica como dívida.

### B51 — Campo Website ✅
Aponta para `releases/latest`. Não há GitHub Pages (`/repos/.../pages` → 404), então não havia ganho de SEO disponível; mandar o visitante para o download é o mais útil que sobrou. O convite do Discord existia **só** nesse campo — foi realocado para a seção Contribute dos três READMEs antes da troca.

### B36 — Smart Speed e Voice Boost ✅
Voice Boost normaliza para EBU R128 (−16 LUFS, −1.5 dBTP) e copia o vídeo. **Smart Speed sai como áudio, e isso é restrição real**: `silenceremove` descarta amostras sem mexer no PTS do vídeo, então mp4 dessincronizaria; cortar as duas streams exigiria `select`/`aselect`. Preview via `silencedetect` mede o ganho sem escrever arquivo. A sonda não passa pela allowlist de ffmpeg de propósito — precisa de `-f null`, e liberar `-f` daria ao caminho de IA escolher muxer arbitrário.

---

## Onda v0.7.8 — concluída, aguardando release

B47, B48 e B41 entregues na branch `feat/ci-and-hardening`. Portões: clippy no baseline (34/42/8/1), 460 testes (era 456), `pnpm check` 0 erros / 107 warnings.

### Escopo original

Duas correções baratas com impacto real, mais a rede de proteção que impede a próxima PR de entrar quebrada. Ordem de entrada: B47 → B48 → B41.

---

### B47 — Ter CI

**Status:** concluído · **Custo:** P · **Impacto:** 5 · **Pré-requisito de tudo**

**Problema.** Não existe CI neste repositório. Nenhum `pnpm check`, nenhum `cargo test`, nenhum `cargo clippy` roda em PR. Só `release.yml` está versionado.

**Causa raiz.** `.gitignore:31` ignora `/.github` inteiro. O `ci.yml` existe apenas em disco local e nunca foi versionado. Isso é o que permitiu a PR #197 ficar aberta com 2 erros de `svelte-check` sem nenhum sinal — os erros só apareceram ao rodar os portões à mão num worktree.

**Escopo.**
- Diagnosticar com `git check-ignore -v .github/workflows/ci.yml`.
- Investigar se `release.yml` só está versionado por `git add -f`. Se sim, o problema é o diretório inteiro fora do git, e qualquer workflow futuro cai no mesmo buraco.
- Versionar `ci.yml` rodando os cinco portões: `cargo build`, `cargo clippy` (0 warnings novos), `cargo test`, `pnpm check`, validação de locales.
- Rodar em `pull_request` e em push para a branch principal.

**Fora de escopo.** Matriz de plataformas, cache agressivo, release automation. Primeiro ter portão, depois otimizar.

**Critério de aceite.** Uma PR com erro deliberado de `svelte-check` é reprovada pelo CI antes de qualquer revisão humana.

> Reescrito. A versão original — "colocar `check-i18n-usage.mjs` na CI" — teve a premissa invalidada: não há CI onde colocar. O item subiu de mais barato da lista para pré-requisito.

---

### B48 — Sanitizar `plugin_id` antes do `remove_dir_all`

**Status:** concluído · **Custo:** P · **Impacto:** 5 · **Segurança — prevenção**

**Problema.** `plugin_loader.rs:193` faz `join(plugin_id)` cru e apaga o resultado com `remove_dir_all`. Um id contendo `..` escapa do diretório de plugins e deleta caminho arbitrário.

**Severidade — resolvido.** A pergunta era se o id vem de fora ou é derivado internamente. **Vem de fora.** A cadeia completa: registro `plugins.json` (buscado de `tonhowtf/omniget-plugins`) → `installed.json` → lista no frontend → `marketplace/+page.svelte:133` chama `invoke("uninstall_plugin", { pluginId: id })` → `commands/plugins.rs:179` → `unregister` → `remove_dir_all`. Nenhum ponto da cadeia sanitiza.

Ainda assim **não é release de segurança isolada**: explorar exige comprometer o registro do próprio projeto ou o usuário instalar à mão um plugin com `plugin.json` forjado. Não é acionável por site qualquer nem por conteúdo baixado. Fica na onda, com prioridade alta.

**Correção.** O guard correto já existe em `plugin_host.rs:116`. Extrair e aplicar nos dois pontos, em vez de duplicar.

**Escopo.** Sanitização no ponto de entrada, não no ponto de uso. Auditar os demais `join` com componente de origem externa no mesmo módulo — a instalação também monta `{app_data}/plugins/{id}/` a partir do mesmo id.

**Critério de aceite.** Teste com `plugin_id` = `../../algo` falha explicitamente e não toca em nada fora do diretório de plugins.

---

### B41 — Cascata de client do YouTube (SABR)

**Status:** concluído · **Custo:** P · **Impacto:** 5 · **Melhor razão custo/impacto da lista**

**Problema.** O extractor `web` passou a devolver formatos SABR-only que quebram o caminho normal de download. O usuário vê falha sem causa legível.

**Metade já existe.** `ytdlp.rs:1452` e `ytdlp.rs:1662` já têm mecanismo de fallback de `player_client`. Falta: `ytdlp.rs:1635` parar de fixar o default, e a cascata ser disparada por detecção de SABR no stderr.

**Escopo.** Cascata `android` → `ios` → `tv` → `web_safari` via `--extractor-args`, acionada por detecção. Sem tela nova, sem setting novo.

**Critério de aceite.** URL que hoje falha com formato SABR-only baixa sem intervenção do usuário. Teste com stderr capturado como fixture.

---

## Bloqueados por terceiro

### B45 — Caminho customizado de binário

**Status:** bloqueado · **Origem:** issue #196, PR #197 · **Bola com o autor**

O autor voltou em 2026-07-29 e corrigiu **5 dos 6 pontos**, verificados linha a linha: `telegramGetChats` exportado (`study-telegram-bridge.ts:333`), `source: "clipboard"` presente (`+layout.svelte:240`), **43 chaves em todos os 9 locales, zero faltando**, guarda de monotonicidade do progresso restaurada (`ytdlp.rs:2529`), changelog de PR removido.

Bloqueio atual mudou de natureza: a PR **não mergeia mais** — conflita em `src-tauri/src/core/queue.rs` depois da v0.7.7 e do remake visual. Precisa de rebase do autor. Ponto 4 (spawn duplicado em `commands/dependencies.rs:29-39`) segue aberto, severidade baixa.

Com o CI de pé, ele agora vê os portões sozinho. Re-check comentado na PR.

### B49 — Modo portátil

**Status:** aberto · **Origem:** issue #195 · **Custo:** M (revisado de P)

Diagnóstico completo e inalterado: o `WEBVIEW2_USER_DATA_FOLDER` definido em `main.rs` é código morto porque o Tauri resolve `data_directory` para `LocalData/{identifier}` antes de a variável ser lida, e o `tauri.conf.json` só aceita caminho relativo sob `data_local_dir()`.

**O que mudou:** existe CI com job `rust (windows-latest)`, então a compilação Windows passou a ser verificável. **O que não mudou:** esse job roda `cargo test`, não sobe o app — ele prova que compila, não que o modo portátil funciona. O conserto (`"create": false` + construir a janela no `setup()` com `.data_directory(...)`) reestrutura o bootstrap da janela principal, e a validação final continua sendo manual em Windows.

Custo revisado de P para M por causa disso. Ao implementar, listar a validação manual em `ENTREGA.md` em vez de fingir cobertura de CI.

---

## Bloco de sobrevivência de plataforma

Os três só valem juntos e depois do B41. Isolados, B42 e B43 entregam capacidade que o usuário não consegue acionar; o B44 é o que traduz os três em algo utilizável.

### B42 — PO Token

**Status:** aberto · **Custo:** G · **Impacto:** 5

Todo request ao YouTube exige Proof-of-Origin Token, ligado ao vídeo, ligado à sessão, com expiração rápida. Extração manual não funciona. Rodar `bgutil-ytdlp-pot-provider` como serviço lateral gerenciado pelo app: healthcheck, restart, estado visível na UI. Sem ele, formatos degradados ou bloqueio direto.

**Critério de aceite.** Provedor sobe e cai com o app; falha do provedor aparece como causa nomeada, não como erro de download.

### B43 — Impersonation

**Status:** aberto · **Custo:** G · **Impacto:** 4

`curl_cffi` fornece targets Chrome / Edge / Safari / Firefox / Tor. Sem ele, sites com TLS fingerprinting falham com mensagem que o usuário final não entende. Não vem em todos os builds — detectar ausência e oferecer instalação guiada.

**Critério de aceite.** Ausência de `curl_cffi` é detectada antes da tentativa de download, não depois da falha.

### B44 — Painel de causa raiz

**Status:** aberto · **Custo:** M · **Impacto:** 5

Traduz stderr do yt-dlp em causa humana mais correção em um clique: instalar `curl_cffi`, renovar cookies, subir o provedor de PO token, trocar de client. É o item que converte B41, B42 e B43 em algo que o usuário entende. Origem: painel único de causa raiz em controle de tráfego aéreo.

**Critério de aceite.** As cinco falhas mais frequentes do log têm causa nomeada e ação correspondente na própria tela.

---

## Restantes, por retorno

### Custo M × Impacto 4

**B33 — Flight recorder.** Buffer circular dos últimos N minutos de eventos, despejado em crash ou no botão de reportar bug, com redação automática de cookies, tokens e paths pessoais. Transforma bug report inútil em trace reproduzível — e alimenta a triagem de issues das próximas rodadas. Origem: caixa-preta de aviação.

**B34 — Pre-flight de lote.** Resolver todas as URLs, checar auth, espaço em disco e tamanho estimado antes de iniciar qualquer download. Zero lote pela metade às 3h. Origem: checklist pré-voo.

**B35 — Concorrência adaptativa.** Auto-tune de `-N` por host medindo throughput real, em vez de número fixo no settings. Ficou melhor posicionado agora que o B23 fez o freio de 429 funcionar de verdade — há sinal confiável para realimentar. Origem: ops de CDN.

**B36 — Smart Speed e Voice Boost.** `silenceremove` mais `loudnorm` (EBU R128) sobre aula gravada: corta 15–25% da duração e nivela o áudio entre módulos gravados com setups diferentes. `ffmpeg.rs` já está no lugar. **Custo P × impacto 4** — o melhor retorno fora da onda proposta. Origem: Overcast.

**B37 — Pin com rollback atômico.** Versão de yt-dlp e ffmpeg fixada com rollback: nova versão quebrou um site, volta para a anterior em um clique. Mais lockfile de curso/playlist. Origem: Nix, pnpm.

**B40 — Motor de regras declarativo.** "URL do canal X → pasta Y, qualidade Z, tag W, transcrever." Escrito uma vez pelo usuário. Origem: Sieve, de clientes de e-mail.

### Estruturais e de nicho

**B32 — Fila como write-ahead log.** Custo G × impacto 5. O item mais estrutural do backlog: `kill -9` no meio de 200 downloads e o estado volta exato. Substitui qualquer persistência best-effort. Origem: recuperação de crash em bancos de dados.

**B39 — Diff entre versões.** Re-baixar um vídeo e detectar que foi reeditado, censurado ou re-uploadado. Guarda os dois, mostra o diff de duração, chapters e hash. Feature de arquivista que não existe em lugar nenhum.

**B46 — Streaming de torrent.** Download sequencial, bloqueio inteligente até as peças chegarem, seek em vídeo parcialmente baixado, servidor UPnP/DLNA. Origem: rqbit, mais primeira-e-última-peça do qBittorrent.

**B38 — Store endereçado por conteúdo.** O mesmo vídeo em 3 qualidades ou o mesmo PDF em 4 cursos ocupa espaço uma vez, via hardlink no store. Menor retorno da lista. Origem: restic, borg.

---

## Notas de manutenção

- Este arquivo é a fonte de verdade do backlog. Item entregue sai daqui e entra no `ENTREGA.md` da release correspondente.
- Item cujo diagnóstico invalida a premissa é reescrito no lugar, com o registro do que mudou — ver B47.
- Estimativa de custo e impacto é revisada quando um item vizinho aterrissa — ver B35 depois do B23.
