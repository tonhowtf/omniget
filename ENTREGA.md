# ENTREGA — sessão autônoma 2026-07-29 (Ondas 1 e 2)

`docs/backlog.md` é a fonte de verdade pública; este arquivo é o registro da sessão.

**Placar: 8 `CONCLUIDO`, 0 `BLOQUEADO`, 12 `PENDENTE`.** Próximo item: **B52**.

> ⚠️ **A v0.7.8 não deve ser tagueada antes da issue #209.** O B49 reestrutura a criação da janela principal e a CI não sobe o app.

---

## 1. Itens entregues

Todos com CI 6/6 verde antes do merge. Nenhum marcado por auto-relato.

| Item | PR | CI | Testes |
|---|---|---|---|
| B47 + B48 + B41 | #204 | 6/6 | — |
| B50 · baseline de clippy | #205 | 6/6 | — |
| B51 · campo Website | #205 | 6/6 | — |
| B36 · Smart Speed / Voice Boost | #205 | 6/6 | +7 |
| B45 · caminho de binário | #197 | 6/6 | — |
| B50-b · baseline reconciliado | #207 | 6/6 | — |
| B49 · modo portátil | #208 | 6/6 | +4 |
| B53 · validação do B36 | #210 | 6/6 | +2 |

Testes **460 → 473**. Clippy: 92 warnings reais, portão ativo nas três plataformas. `pnpm check`: 0 erros.

### O que cada um virou, quando divergiu do plano

**B45 — o bloqueio nunca foi o autor.** Vim rebasar a branch dele e não havia o que rebasar: @gtxPrime tinha sincronizado com `main` sozinho, corrigido o ponto 4 e rodado `cargo fmt`. O bloqueio real era **`action_required`** — o GitHub segura workflow de contribuidor externo até o mantenedor aprovar, e o CI tinha nascido no dia anterior, então ninguém tinha notado a fila. Aprovei, verde, mergeada, issue #196 fechada.

**B50-b — os 177 eram contagem dupla.** `1+1+34+8+8+37+42+46 = 177` exatamente: `--all-targets` compila `lib` e `lib test` e o mesmo warning sai uma vez por target. O portão funcionava, mas o número mentia e um warning novo aparecia como +2. Dedup por origem → **92 reais**. Três baselines por plataforma; **os três dão 92/51 e zero lints diferem**, o que é honesto reportar: o split não rende nada hoje.

**B49 — o CI destravou menos do que parecia.** Ter job de Windows torna a compilação verificável, não o comportamento. O job roda `cargo test`, não sobe o app. Implementei e mergeei, mas abri a **issue #209** com os três casos de teste manual, porque um erro aqui não quebra o modo portátil — quebra a abertura do app para todo mundo.

**B53 — a validação achou um bug.** Duas das três promessas do B36 se sustentaram. A terceira não: a sonda previa 20,0% e o corte real foi 18,5%, porque `silenceremove` preserva `stop_duration` de cada trecho (12 × 0,35 s = 1,4%, que fecha a conta). Estimativa corrigida.

---

## 2. Bloqueados

**Nenhum.** O único item que estava bloqueado (B45) foi destravado e mergeado.

---

## 3. O que NÃO foi verificado

- **O B49 nunca abriu uma janela.** É a lacuna mais séria desta sessão. A CI prova compilação nas três plataformas e a lógica de diretório (4 testes). Não prova que o app sobe, nem que `%LOCALAPPDATA%\wtf.tonho.omniget` para de aparecer. Issue #209, com os passos exatos.
- **O B53 usou mídia sintética, não uma aula real.** Construí 300 s com 60 s de silêncio conhecido — mais rigoroso para checar a aritmética, mas não é uma aula. O download real do YouTube estourou 10 minutos.
- **Os três botões do B36 continuam sem nunca terem sido clicados.** Validei os filtros por linha de comando, não pela UI.
- **O baseline de clippy do Windows veio do log da CI**, não de uma máquina Windows minha. É o output do próprio script rodando lá, então é legítimo, mas não fui eu que rodei.
- **`pnpm check` subiu de 107 para 109 warnings.** Não fui eu: são dois a11y warnings em `TelegramUploadModal.svelte`, da #197. Verifiquei a origem em vez de adotar o número novo em silêncio.
- **Traduções** de es/fr/it/ja/ru/zh/zh-TW/el continuam sem revisão nativa.

---

## 4. Cenários de teste manual

**Bloqueantes para a v0.7.8** (issue #209, Windows):

1. **Portátil.** `portable.txt` ao lado do `.exe`, apague `%LOCALAPPDATA%\wtf.tonho.omniget`, abra. A janela precisa abrir, a pasta não pode voltar, e `<app>\data\webview` precisa existir.
2. **Instalação normal — o risco de regressão.** Abra **sem** `portable.txt`. A janela precisa abrir e se comportar como antes. Esse caminho é o de todo mundo.
3. **Segunda abertura** nos dois modos: settings e tamanho de janela persistem.

**Não bloqueantes:**

4. **Smart Speed pela UI.** Numa aula real, "Medir silêncio" → deve estimar um pouco **menos** que antes (a correção do B53). Depois "Cortar silêncio": sai `.m4a`, e a duração deve bater com a estimativa agora.
5. **Voice Boost.** Sai `.mp4` com vídeo idêntico e áudio nivelado.

---

## 5. Atribuição

| Feature | Inspirada em | Licença | Como foi usado |
|---|---|---|---|
| B36 / B53 Smart Speed e Voice Boost | Overcast | proprietário | Só a descrição pública. Filtros são do ffmpeg, alvos são da EBU R128. |
| B50 / B50-b baseline de lint | `golangci-lint`, `ruff` | MIT / MIT | Só o padrão: congelar o herdado, reprovar o novo. |
| B49 modo portátil | — | — | Diagnóstico próprio sobre a fonte do Tauri 2.10.2. |

Nenhum trecho copiado. Nenhuma dependência nova — `Cargo.toml`, `Cargo.lock` e `pnpm-lock.yaml` intocados nas duas ondas.

---

## 6. Dívida introduzida e riscos abertos

1. **O B49 está em `main` sem nunca ter sido executado.** Risco de startup, não de feature. A issue #209 existe para isso, mas depende de alguém com Windows.
2. **Os três baselines de clippy são idênticos hoje**, então mantê-los é custo triplo sem benefício até o código condicional divergir. Se em três meses continuarem iguais, vale reconsiderar.
3. **O baseline pode inflar sem ninguém notar.** O script avisa quando melhora e sugere regravar, mas não força.
4. **`silence_probe_args()` segue fora da allowlist por design**, documentado e travado por teste. Se alguém acrescentar entrada de usuário ali, o raciocínio quebra em silêncio.
5. **Smart Speed descarta o vídeo sem avisar antes de clicar.** O botão diz "Cortar silêncio" e devolve `.m4a`. O B52 resolve isso movendo a feature para o player, que é a camada certa.

---

## 7. PRs abertas e próximo passo

**Nenhuma PR aguardando merge.** #197, #204, #205, #207, #208 e #210 foram mergeadas com CI verde.

**Issues abertas por esta sessão:** #209 (validação do B49 em Windows — bloqueia a v0.7.8), além de #202 e #203 de ontem.

**Próximo item: B52** — Smart Speed no player em vez de no arquivo. Não iniciado, decisão de contexto. É Custo M e mexe no player de curso; o mapa de silêncio já existe e já está fora do caminho da allowlist, então o trabalho é persistir como metadado da aula e pular na reprodução.

Depois: Onda 3 (B42 → B43 → B44), e o B42 ganhou evidência nova nesta sessão — o download de aula do YouTube estourou dez minutos.
