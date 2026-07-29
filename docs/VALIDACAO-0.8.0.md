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
