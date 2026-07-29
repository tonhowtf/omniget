# Validação manual — estabilização da 0.8.0

Lista do que precisa ser clicado antes da 0.8.0. Cada linha vem de uma feature
que foi implementada e testada por baixo, mas cuja superfície visual não foi
exercitada — por decisão explícita do modo feature, não por esquecimento.

Formato: **o que clicar** → o que precisa acontecer.

## Bloqueantes

- **#209 caso 1 — portátil no Windows.** `portable.txt` ao lado do `.exe`, apagar
  `%LOCALAPPDATA%\wtf.tonho.omniget`, abrir → janela abre, a pasta não volta,
  `<app>\data\webview` existe. É o único caso do B49 nunca observado funcionando.
- **#209 caso 3 — segunda abertura.** Abrir duas vezes nos dois modos → settings e
  tamanho de janela persistem.

## Onda 1

- **Smart Speed / Voice Boost (B36, B53).** Numa aula real: "Medir silêncio" estima,
  "Cortar silêncio" devolve `.m4a` com duração batendo com a estimativa, "Nivelar voz"
  devolve `.mp4` com vídeo idêntico e áudio nivelado. Os três botões nunca foram clicados.

## Onda D

- **B35 concorrência adaptativa** → quando ligado: baixar do mesmo host várias vezes e
  confirmar que o `-N` efetivo sobe quando há banda e **cai na hora** ao tomar 429.
  Confirmar que o número em Config passou a se comportar como teto, não valor fixo.
- **B46 streaming de torrent** → quando ligado: começar um torrent de vídeo e confirmar
  que o player abre antes de terminar, que o seek para o meio espera a janela de lá em
  vez de travar, e que a troca (pior para o enxame) está dita na interface.

## Onda C

- **B44 painel de causa raiz** → quando a tela existir: forçar cada uma das sete causas
  e confirmar que aparece causa nomeada **e** o botão da ação correspondente, não texto
  cru. Uma falha desconhecida tem que dizer que não sabe, em vez de inventar.
- **B43 impersonation** → confirmar que o botão "tentar imitando um navegador" usa um
  alvo de desktop e que a segunda tentativa passa em site que bloqueia por fingerprint.
- **B42 PO token** → com um provedor bgutil rodando, confirmar que o estado aparece como
  pronto e que o download do YouTube deixa de degradar. **Sem provedor rodando, confirmar
  que nada é passado ao yt-dlp** — apontar para endereço morto faz cada download esperar
  o timeout.

## Onda B

- **B34 pre-flight de lote** → quando ligado ao botão de lote: colar 3 links sendo
  um privado e um já baixado, confirmar que o resumo aparece **antes** de começar,
  que diz quantos vão baixar, e que o link privado sugere importar cookies.
  Encher o disco e confirmar que o lote para em vez de começar.
- **B52 Smart Speed no player** → quando ligado ao player de curso: numa aula com
  silêncio, ligar o toggle e confirmar que a reprodução pula sem travar e sem
  cortar o início da fala. Desligar e confirmar que volta ao normal na hora,
  **sem reprocessar nada**. Assistir de novo não pode recomputar o mapa.

## Onda A

Nenhum dos cinco tem superfície visual ainda — são módulos de lógica sem UI
conectada. O que precisa de validação aparece quando cada um for ligado:

- **B33 flight recorder** → quando existir o botão "reportar bug": confirmar que o
  despejo não contém cookie, token nem nome de usuário em path.
- **B40 motor de regras** → quando existir a tela de regras: confirmar que a ordem
  da lista é a prioridade e que regra desligada é pulada.
- **B39 diff entre versões** → quando ligado ao re-download: confirmar que um vídeo
  reeditado mostra o diff e não sobrescreve em silêncio.
- **B37 pin com rollback** → quando existir o botão de voltar versão: confirmar que
  o yt-dlp anterior volta e funciona.
- **B38 store endereçado por conteúdo** → quando ligado ao download: confirmar que
  o mesmo vídeo em duas qualidades não dobra o espaço em disco **no mesmo volume**,
  e que numa pasta de saída em outro disco ele copia sem erro.
