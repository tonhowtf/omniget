# Validação manual — estabilização da 0.8.0

Lista do que precisa ser clicado antes da 0.8.0. Cada linha vem de uma feature
que foi implementada e testada por baixo, mas cuja superfície visual não foi
exercitada — por decisão explícita do modo feature, não por esquecimento.

Formato: **o que clicar** → o que precisa acontecer.

## Bloqueantes

**Nenhum.** Os dois casos da #209 viraram o job `smoke` da CI, que roda em
windows-latest e ubuntu-latest a cada PR e reprova se o modo portátil escrever
no perfil do usuário. Verificado por reversão: sem o `set_var` do B49, o job
fica vermelho com o caminho vazado no texto.

Sobrou uma lacuna real, registrada na **#227**: o macOS não é coberto, porque o
`wry` não lê `data_directory` no WKWebView. Não é bloqueante da 0.8.0 — é o
modo portátil não cumprindo o que promete numa das três plataformas.

## Ligados nesta versão — nunca clicados

- **Rollback de yt-dlp (B37).** Config → Plugins → yt-dlp → `Update`, depois `...`
  → deve aparecer "Versões anteriores" com a data. Clicar → volta e a versão na
  tabela muda.
- **Pré-flight de lote (B34).** Colar 3 links, sendo um repetido e um de site sem
  suporte → deve avisar quantos serão pulados e enfileirar só o resto. Colar
  apenas links sem suporte → deve abortar com a mensagem, sem enfileirar nada.
- **Caixa-preta (B33).** Baixar algo, abrir o painel de debug → `Copiar relatório`
  → o texto deve conter `--- Download Trace` e **não** deve conter cookie, token
  nem caminho com o nome do usuário. Este último ponto é o que importa.
- **Concorrência adaptativa (B35).** Baixar duas vezes do mesmo host: a segunda
  pode usar concorrência diferente. Sem forma de observar pela interface hoje —
  só pelo log. É a mais fraca da lista.
- **Impersonation (B43) e PO token (B42).** Só disparam em falha real: um site que
  bloqueia por fingerprint, ou um vídeo que exige PO token. Não dá para provocar
  sinteticamente.

## Issues de usuário

- **#218 — runtime WebView2 ao lado do exe (Windows).** Numa máquina Windows **sem**
  WebView2 instalado: descompactar o Fixed Version Runtime ao lado do `omniget.exe`,
  abrir → a janela abre. Depois renomear a pasta para algo que não case → o app volta
  a exigir o runtime do sistema, em vez de falhar sem janela. A lógica de descoberta
  tem 6 testes, mas o `set_var` está atrás de `cfg(windows)` e nunca rodou no Windows.
- **#222 — seção de extensão traduzida.** Trocar o idioma para inglês (ou qualquer um
  que não seja português) → Config → Plugins: o bloco "Browser extension" e as três
  dicas de instalação aparecem no idioma escolhido, não em português.
- **#222 — menu de dependência inteiro.** Config → Plugins → linha do **PDFium**
  (a última da tabela) → `...` → as opções aparecem completas, sem corte na borda do
  card, e "escolher arquivo" é clicável.

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
