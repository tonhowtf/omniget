# ENTREGA — sessão autônoma 2026-07-29

Continuação do backlog. `docs/backlog.md` é a fonte de verdade pública; este arquivo registra a sessão.

**Placar: 4 `CONCLUIDO`, 1 `BLOQUEADO`, 12 `PENDENTE`.** Próximo item: **B49**.

---

## 1. Itens entregues

Todos com CI 6/6 verde antes de qualquer merge. Nenhum marcado por auto-relato.

### B47 + B48 + B41 — PR #204 · mergeada · CI 6/6

Herdados da sessão anterior, mergeados no início desta. CI verde em `frontend`, `metainfo`, `rust` nas três plataformas e `rust-debian`.

### B50 — Baseline de clippy · PR #205 · CI 6/6 · mergeada

Clippy era informativo porque o workspace carrega **177 warnings herdados** e `-D warnings` reprovaria toda PR no dia um. O baseline commitado (`src-tauri/clippy-baseline.json`, 51 lints) congela o que existe e reprova só o que é novo. Chave `crate|lint`, sem arquivo nem linha — mover código não pode reprovar CI.

**Escrito contra:** clippy do Rust 1.90 (imagem `rust:1.90-bookworm`), Node 20+ (pré-instalado no runner ubuntu).

**Critério de aceite provado, não presumido:** injetei um argumento `&Vec<String>`, o check reprovou nomeando `omniget_lib: clippy::ptr_arg 2 -> 4`; revertido, voltou a `177 warnings, none new`. As duas execuções em container Debian, igual à CI.

**Erro meu no caminho, corrigido:** o primeiro baseline foi gerado no macOS e rotulado `linux` à mão. O script agora grava `process.platform` sozinho, e recusa comparar entre plataformas em vez de reprovar alguém por warning que só existe no SO dele.

### B51 — Campo Website · PR #205 · CI 6/6 · mergeada

Apontava para um convite do Discord, que não passa autoridade nenhuma. Agora aponta para `releases/latest`.

**O achado que mudou a ordem de execução:** o convite existia **só** nesse campo. Os três READMEs citavam Discord apenas como feature (Discord presence, baixar link do Discord), nunca como porta da comunidade. Trocar o campo primeiro teria apagado a entrada da comunidade — então o convite foi para a seção Contribute dos três READMEs **antes** da troca.

Não existe GitHub Pages (`/repos/.../pages` → 404), então não havia ganho de SEO disponível de qualquer forma.

### B36 — Smart Speed e Voice Boost · PR #205 · CI 6/6 · mergeada · +7 testes

**Escrito contra:** ffmpeg com filtros `loudnorm`, `silenceremove` e `silencedetect` (presentes desde o ffmpeg 4.x; a allowlist do projeto já permitia `loudnorm`).

Voice Boost normaliza para EBU R128 (−16 LUFS, −1.5 dBTP) e copia o vídeo intacto.

**Smart Speed sai como áudio, e isso é restrição real, não simplificação.** `silenceremove` descarta amostras de áudio sem tocar no PTS do vídeo, então aplicar sobre mp4 dessincroniza imagem e som. Cortar as duas streams nos mesmos pontos exigiria `select`/`aselect` com expressões — outra ordem de grandeza, e o caso de uso de origem (Overcast) consome aula como áudio. Há teste travando essa decisão para ninguém "melhorar" para mp4 depois.

Preview antes de converter: `silencedetect` mede o silêncio sem escrever arquivo.

**Decisão de segurança registrada:** a sonda **não** passa pela allowlist de ffmpeg. Ela precisa de `-f null`, e liberar `-f` deixaria o caminho de proposta por IA escolher muxer arbitrário — ampliar superfície de segurança para ganhar nada. A sonda é montada só pelo app com argumentos constantes, e um teste afirma as duas metades desse raciocínio.

**Um teste reprovou e estava certo:** `os_presets_novos_passam_pela_allowlist_de_seguranca` pegou que a sonda quebraria em runtime. A correção foi tirar a sonda do caminho da allowlist, não alargar a allowlist.

---

## 2. Bloqueados, com evidência

### B45 — Caminho customizado de binário · PR #197 · bola com o autor

O autor (**@gtxPrime**) voltou durante a sessão e pediu re-check. Verifiquei linha a linha em vez de aceitar o diff:

| Ponto | Estado | Onde verifiquei |
|---|---|---|
| `telegramGetChats` ausente | corrigido | `study-telegram-bridge.ts:333` |
| `source` faltando | corrigido | `+layout.svelte:240` |
| 37 chaves i18n faltando | corrigido | 43 chaves em todos os 9 locales, **zero faltando** |
| Progresso podia regredir | corrigido | `ytdlp.rs:2529` |
| `CHANGELOG_MERGE_REQUEST.md` | removido | — |
| Spawn duplicado | **aberto** | `commands/dependencies.rs:29-39` |

**O bloqueio mudou de natureza:** a PR não mergeia mais — conflita em `src-tauri/src/core/queue.rs` depois da v0.7.7 e do remake visual. Precisa de rebase do autor.

Não empurrei commit na branch dele e não abri PR concorrente. Autoria intacta.

---

## 3. O que NÃO foi verificado

- **O baseline de clippy só cobre Linux.** Warning novo que só aparece no Windows ou no macOS passa despercebido. O script recusa comparar entre plataformas de propósito — o contrário reprovaria gente por warning de outro SO. Baseline por SO é possível e fica como dívida.
- **Smart Speed e Voice Boost não foram exercitados contra mídia real.** Os 7 testes cobrem o parser de `silencedetect` com log real capturado, a construção dos presets e a allowlist. Nenhum roda ffmpeg. Os alvos EBU R128 vêm da especificação, não de medição minha.
- **A estimativa de silêncio não foi comparada com o resultado real da conversão.** O preview diz quanto `silencedetect` encontrou com os mesmos parâmetros do `silenceremove`, o que deve bater — mas não medi as duas pontas.
- **Não rodei o app.** Tudo é build, teste unitário e CI. Os três botões novos no `VideoOpsOverlay` nunca foram clicados.
- **Traduções de es/fr/it/ja/ru/zh/zh-TW/el** são de boa-fé, não revisadas por falante nativo. pt-BR e inglês eu escrevi diretamente.
- **B49 não foi iniciado**, por decisão de contexto e não por bloqueio técnico — ver seção 7.

---

## 4. Cenários de teste manual

1. **Voice Boost.** Baixe um vídeo com áudio baixo, abra as ferramentas de vídeo no item da lista, clique em "Nivelar voz". Sai um `.mp4` novo com o vídeo idêntico e o áudio em volume de podcast. O original não é tocado.
2. **Medir silêncio.** Numa aula longa, clique em "Medir silêncio". Deve aparecer algo como "Cerca de 18 min de silêncio — cortar removeria uns 21%". Num vídeo sem silêncio, a mensagem muda para "Quase não há silêncio para cortar aqui".
3. **Cortar silêncio.** Clique em "Cortar silêncio" na mesma aula. Sai um `.m4a` — **áudio, não vídeo, e isso é esperado**. Confira que a duração caiu perto do que o preview previu e que a fala não ficou picotada entre palavras.
4. **Campo Website.** Abra a página do repositório: o link no topo deve levar a `releases/latest`, e o Discord deve estar na seção Contribute dos três READMEs.
5. **Baseline de clippy, se quiser conferir o portão.** Adicione um `fn x(v: &Vec<String>) -> usize { v.len() }` em qualquer crate, abra PR: o job `rust (ubuntu-latest)` reprova nomeando o lint. Remova e ele volta a passar.

---

## 5. Atribuição

| Feature | Inspirada em | Licença | Como foi usado |
|---|---|---|---|
| B36 Smart Speed / Voice Boost | Overcast | proprietário | Só a descrição pública do que as features fazem. Nenhum código lido ou copiado; os filtros são do ffmpeg e os alvos são da especificação EBU R128. |
| B50 baseline de lint | prática de `golangci-lint` e `ruff` (baseline/`--diff`) | MIT / MIT | Só o padrão de comportamento: congelar o herdado, reprovar o novo. Implementação própria. |
| B51 | — | — | Não é feature. |

Nenhum trecho copiado de projeto algum. Nenhuma dependência nova adicionada — `Cargo.toml`, `Cargo.lock` e `pnpm-lock.yaml` intocados.

---

## 6. Dívida introduzida e riscos abertos

1. **O baseline de clippy é só de Linux.** Já explicado. É a dívida mais concreta desta sessão.
2. **O baseline pode ficar velho para baixo.** Quando alguém corrigir warnings, o script avisa que melhorou e sugere regravar, mas não força. Um baseline inflado protege menos do que parece.
3. **`silence_probe_args()` fica fora da allowlist por design.** Está documentado no código e travado por teste, mas é uma exceção — se alguém acrescentar entrada de usuário ali, o raciocínio quebra em silêncio.
4. **Smart Speed descarta o vídeo sem avisar antes.** O botão diz "Cortar silêncio" e o resultado é `.m4a`. A copy não deixa isso óbvio de antemão. Candidato a ajuste de UX.
5. **O `check_dependencies` do `main` e o da PR #197 divergiram.** O `main` ganhou `outdated: bool` na v0.7.7; o rebase do autor vai encontrar isso. Avisei na PR.

---

## 7. PRs abertas e próximo passo

**Nenhuma PR minha aguardando merge.** #204 e #205 foram mergeadas com CI verde.

**PR #197** (@gtxPrime) está com o autor, aguardando rebase.

**Próximo item: B49** (modo portátil, issue #195). Não foi iniciado — decisão de contexto, não bloqueio técnico. O escopo foi revisado durante a sessão e está em `docs/backlog.md`: custo subiu de P para M porque o job `rust (windows-latest)` prova que **compila**, não que o modo portátil **funciona** — ele roda `cargo test`, não sobe o app. O conserto reestrutura o bootstrap da janela principal, e começar isso com contexto no fim é como se deixa o repo pela metade.

Depois dele, a ordem segue: B42 → B43 → B44 (bloco de sobrevivência de plataforma), depois a Onda 4.
