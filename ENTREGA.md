# ENTREGA — sessão autônoma 2026-07-29

`docs/backlog.md` é a fonte de verdade pública; este arquivo é o registro da sessão.

**O backlog de feature acabou.** 22 itens entregues e mergeados com CI verde, 1 PR aberta por regra, 1 bloqueado com evidência, 1 adiado por decisão.

| | |
|---|---|
| Testes | 460 → **576** em `main` (596 com a PR #217) |
| Clippy | 92 warnings reais, portão ativo nas três plataformas |
| `pnpm check` | 0 erros |
| Commits em `main` | 0 diretos — tudo por PR |
| Force-push | 0 |

---

## 1. Entregue e mergeado

| Onda | Itens | PR | CI |
|---|---|---|---|
| — | B47 CI · B48 `plugin_id` · B41 cascata SABR | #204 | 6/6 |
| 1 | B50 baseline clippy · B51 Website · B36 Smart Speed | #205 | 6/6 |
| 2 | B45 caminho de binário (contribuidor) | #197 | 6/6 |
| 2 | B50-b baseline dedup + por plataforma | #207 | 6/6 |
| 2 | B49 modo portátil | #208 | 6/6 |
| 2 | B53 validação do B36 contra mídia | #210 | 6/6 |
| 2 | B57 sem regressão · B58 boot observável | #212 | 6/6 |
| A | B33 flight recorder · B40 regras · B39 diff · B37 rollback · B38 store | #213 | 6/6 |
| B | B34 pre-flight · B52 Smart Speed no player | #214 | 6/6 |
| C | B42 PO token · B43 impersonation · B44 causa raiz | #215 | 6/6 |
| D | B35 concorrência adaptativa · B46 streaming de torrent | #216 | 6/6 |
| — | B56 política de aprovação de workflow | esta PR | — |

---

## 2. Aberta e **não** mergeada

### B32 — fila como write-ahead log · PR #217 · CI 6/6

Não mergeada por regra, e a regra está certa: muda como a fila sobrevive a um crash.

Testado contra um **SIGKILL de verdade**, não simulação. `scripts/wal-kill9.py` sobe um filho que grava com `fsync` por registro e o mata enquanto o último está pela metade: **200 registros íntegros, exatamente 1 truncado**. O teste de regressão reproduz esse arquivo e recupera a fila com ordem, progresso e opções.

**Não está ligada ao `queue.rs`.** Substituir o `recovery.json` significa mudar todos os call sites e decidir a migração de quem já tem um — decisão deliberada, não efeito colateral desta PR.

---

## 3. Bloqueado, com evidência

### B54 — caso 1 da #209 · portátil no Windows

O caso 2 (abrir **sem** `portable.txt`, o caminho de todo usuário) foi **provado no macOS**: `JANELA CRIADA: label=main` mais boot completo até carregar três plugins. Isso foi possível porque só o `.data_directory()` está atrás de `cfg(windows)` — a reestruturação em si roda em toda plataforma.

O caso 1 continua sem ter sido observado funcionando, e é o que o B49 conserta. Precisa de máquina Windows. Rastreado na **#209**, com @PaduaPlay marcado.

---

## 4. Adiado por decisão

### B55 — smoke test nas três plataformas

Adiado para a estabilização da 0.8.0 pelo próprio modo feature: durante acúmulo de feature, não se age sobre falha de smoke test. É o que fecha a #209 por CI e tapa o buraco estrutural — o CI prova que compila, nunca que abre.

---

## 5. O que NÃO foi verificado

- **Nenhuma das 12 features do modo feature tem UI ligada.** São módulos de lógica com teste; nada foi clicado. As 12 linhas estão em `docs/VALIDACAO-0.8.0.md`.
- **O B49 está em `main` sem nunca ter aberto uma janela no Windows.** É a lacuna mais séria.
- **O B53 usou mídia sintética.** O download de uma aula real do YouTube estourou dez minutos — evidência a favor do B42, e limitação do teste.
- **Nada do bloco de sobrevivência foi exercitado contra rede real.** O B42 nunca falou com um provedor bgutil rodando; o B43 leu a saída real do `--list-impersonate-targets` mas nunca tentou baixar de um site que bloqueia por fingerprint.
- **O B35 nunca mediu throughput real** e o **B46 nunca viu um torrent vivo.** Ambos têm o núcleo de decisão testado; a medição é I/O não exercitado.
- **O baseline de clippy do Windows veio do log da CI**, não de uma máquina Windows minha.
- **Traduções** de es/fr/it/ja/ru/zh/zh-TW/el continuam sem revisão nativa.

---

## 6. Erros meus, e como apareceram

Registrados porque o padrão importa mais que os casos.

1. **Diagnóstico errado do socket órfão.** Reportei na #209 que runs repetidos batiam num socket órfão. Era minha própria instância de teste, ainda viva. Descobri porque `ls /tmp/*_si.sock` não achou nada com o app supostamente rodando — a ausência contradizia minha explicação. Corrigido publicamente.
2. **Inferência errada sobre o boot.** Deduzi que o `plugin_loader` no log provava que a janela tinha subido. `PluginManager::new` está antes do `setup()`. Refiz com log direto.
3. **Aritmética errada num teste.** Afirmei que 9 GB + 10% não cabe em 10 GB. Cabe.
4. **Baseline gerado no SO errado.** Gerei no macOS e rotulei `linux` à mão. O script agora grava `process.platform` sozinho.
5. **Teste que não conseguia reprovar.** A fixture do B43 listava Chrome desktop primeiro, então "preferir desktop" e "pegar o primeiro" eram indistinguíveis. Só apareceu no revert-e-veja-vermelho.
6. **Quebra que não aplicou.** Duas vezes o revert-e-veja-vermelho não reprovou porque a string de substituição não casou depois do `cargo fmt`. Uma quebra que falha em aplicar é indistinguível de um teste que não consegue reprovar — passei a afirmar a âncora antes de substituir.

---

## 7. Premissas que o mundo real corrigiu

- **B42** — o `bgutil` roda como Docker ou servidor Node **mais** um plugin Python dentro do yt-dlp. Empacotar é outra ordem de grandeza; entreguei o lado do app e registrei a dívida.
- **B43** — o yt-dlp empacotado **já tem** `curl_cffi`, com 40 alvos. O caso comum é presença, não ausência.
- **B47** — não havia CI onde colocar o `check-i18n`. O item virou "ter CI".
- **B50** — os 177 warnings eram contagem dupla por target, não 90 abençoados. Dedup → 92.
- **B57/B58** — nenhum dos dois bugs suspeitados existia. O achado real era menor: saída silenciosa do single-instance.
- **B36** — `silenceremove` dessincroniza A/V, então Smart Speed sai como áudio. O **B52** é a versão que alcança o número que o B36 não alcança.

---

## 8. Atribuição

| Feature | Origem | Licença |
|---|---|---|
| B36/B52/B53 Smart Speed, Voice Boost | Overcast | proprietário — só a descrição pública |
| B50 baseline de lint | `golangci-lint`, `ruff` | MIT |
| B33 flight recorder | caixa-preta de aviação | conceito |
| B34 pre-flight | checklist pré-voo | conceito |
| B35 concorrência adaptativa | controle de congestionamento de CDN | conceito |
| B37 pin com rollback | Nix, pnpm | conceito |
| B38 store por conteúdo | restic, borg | BSD — só o padrão |
| B39 diff entre versões | git diff | conceito |
| B40 motor de regras | Sieve | conceito |
| B44 painel de causa raiz | controle de tráfego aéreo | conceito |
| B46 streaming de torrent | rqbit, qBittorrent | conceito |

Nenhum trecho copiado. Uma dependência nova: **fs4 1.1.0** (MIT OR Apache-2.0), verificada no crates.io.

---

## 9. Dívida e riscos abertos

1. **O B49 está em `main` sem validação de Windows.** Risco de startup, não de feature. #209.
2. **Doze módulos sem UI.** Código testado que ninguém pode usar ainda. É acúmulo deliberado, mas acumula.
3. **Empacotar o provedor de PO token** ficou de fora, com o motivo no código.
4. **Os três baselines de clippy são idênticos hoje** — custo triplo sem benefício até o código condicional divergir.
5. **O B32 não está ligado**, e ligá-lo exige decidir a migração de quem já tem `recovery.json`.
6. **`pnpm check` subiu de 107 para 109 warnings** por a11y no `TelegramUploadModal.svelte`, da #197.

---

## 10. Próximo passo

A estabilização da 0.8.0, nesta ordem:

1. **`docs/VALIDACAO-0.8.0.md`** — 12 linhas, começando pelo caso 1 da #209.
2. **B55** — smoke test que sobe o app, com o critério de aceite que reverter o B49 deixa o job vermelho.
3. **#217 (B32)** — revisar, decidir a migração, ligar ao `queue.rs`.
4. Ligar as 12 features à interface.

**A v0.7.8 não deve ser tagueada.** A 0.8.0 vem com o remake, e é lá que a estabilização acontece.
