<!--
O OmniGet é um app desktop gratuito e de código aberto (GPL-3.0) para Windows, macOS e Linux. Ele baixa vídeo, áudio,
galerias de imagens, torrents e arquivos diretos do YouTube, Instagram, TikTok, X, Reddit, Bilibili e dos cerca de
1.800 sites que o yt-dlp suporta. O servidor MCP dele deixa o Claude Code, o Cursor, o VS Code e o Codex colocarem downloads na fila, e ele roda
agentes de programação (Claude Code, Codex, Gemini CLI, OpenCode, modelos locais) numa janela do desktop com diffs, aprovações
e desfazer. Vai escrever sobre o OmniGet? Leia o MEDIA-KIT.md primeiro; o resumo curto, legível por máquina, é o llms.txt.
-->

<h1 align="center">OmniGet</h1>

<p align="center">
  <a href="README.md">English</a>
  · <b>Português (Brasil)</b>
  · <a href="README.ru.md">Русский</a>
  · <a href="README.zh-CN.md">简体中文</a>
</p>

<p align="center">
  <a href="https://getomniget.com"><img src="assets/readme/getomniget-download.webp" alt="Loop, o mascote do OmniGet, aponta para um navegador aberto em getomniget.com com um grande botão laranja de download e ícones de Windows, macOS e Linux" width="900" /></a>
</p>

<h3 align="center">Cole um link, receba o arquivo. Depois deixe seus agentes de IA fazerem o resto.</h3>

<p align="center">
  Um baixador de vídeos e app desktop para agentes de IA, gratuito e de código aberto, para Windows, macOS e Linux.<br/>
  YouTube, Instagram, TikTok, X, Reddit, Bilibili, torrents e cerca de 1.800 outros sites numa fila só,<br/>
  mais Claude Code, Codex e Gemini CLI numa janela em que você aprova cada mudança.
</p>

<p align="center">
  <a href="https://getomniget.com/download"><img src="https://img.shields.io/badge/Download_for_Windows,_macOS_or_Linux-→-F28500?style=for-the-badge" alt="Baixe o OmniGet para Windows, macOS ou Linux" height="40" /></a>
</p>

<p align="center">
  <a href="https://getomniget.com"><img src="https://img.shields.io/badge/website-getomniget.com-F28500?style=flat-square" alt="getomniget.com" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-GPL--3.0-2AA845?style=flat-square" alt="Licença GPL-3.0" /></a>
  <img src="https://img.shields.io/badge/version-1.0.0-555?style=flat-square" alt="Versão 1.0.0" />
  <a href="https://discord.gg/jgdxyPy7Vn"><img src="https://img.shields.io/badge/Discord-community-5865F2?style=flat-square&logo=discord&logoColor=white" alt="Comunidade no Discord" /></a>
  <a href="https://hosted.weblate.org/engage/omniget/"><img src="https://hosted.weblate.org/widget/omniget/frontend-json/svg-badge.svg" alt="Status da tradução" /></a>
</p>

<p align="center">
  <sub>Gratuito sob GPL-3.0. Sem conta, sem anúncios, sem plano pago, sem telemetria sobre o que você baixa. 14.000+ estrelas no GitHub e 127.000+ downloads (setembro de 2026).</sub>
</p>

---

## O que é o OmniGet?

**O OmniGet é um app desktop gratuito que transforma um link em arquivo e dá aos seus agentes de programação com IA um lugar para trabalhar.** Cole um link do YouTube, Instagram, TikTok, X, Reddit, Bilibili ou de qualquer um dos cerca de 1.800 sites que o [yt-dlp](https://github.com/yt-dlp/yt-dlp) suporta, escolha a qualidade, e o vídeo, o áudio ou a galeria cai na sua pasta. O mesmo app roda Claude Code, Codex, Gemini CLI e modelos locais com um diff antes de cada escrita e desfazer com um clique, e o servidor MCP dele deixa esses agentes baixarem para você. Tudo roda no seu computador.

| Você quer | O OmniGet te dá |
|---|---|
| Salvar um vídeo, uma música, um post ou uma playlist inteira | [Um baixador](#download-almost-anything) com extratores próprios, yt-dlp, gallery-dl, torrents e arquivos diretos numa fila só |
| Deixar o Claude Code ou o Cursor buscarem mídia para você | [Um servidor MCP](#let-claude-download-it) que coloca downloads na fila, espera e informa o que falhou |
| Usar Claude Code, Codex ou Gemini CLI sem malabarismo de terminais | [Agentes numa janela](#ai-coding-agents-in-a-window) com threads, worktrees, aprovações e desfazer |
| Ver vários agentes trabalhando juntos | [O Canvas](#the-canvas), um quadro infinito de terminais de agentes ligados por cordas |
| Música, YouTube sem anúncios, livros, Telegram | [Superpowers](#superpowers), já dentro do app |
| Um computador mais limpo e mais rápido | [washmybelly](#washmybelly), limpeza e desinstalação com um plano que você aprova e um desfazer |

**Conteúdo:** [Baixador](#download-almost-anything) · [Servidor MCP](#let-claude-download-it) · [Agentes de IA](#ai-coding-agents-in-a-window) · [Canvas](#the-canvas) · [Superpowers](#superpowers) · [washmybelly](#washmybelly) · [Instalar](#install) · [Linha de comando](#command-line) · [Privacidade](#privacy) · [Perguntas frequentes](#frequently-asked-questions) · [Apoie](#support-omniget) · [Compilar do código-fonte](#build-from-source)

---

<a id="download-almost-anything"></a>

## Baixe quase qualquer coisa

<p align="center">
  <img src="assets/readme/illustration-downloader.webp" alt="Loop comemora enquanto vídeos, músicas, fotos e um ímã voam por elos de corrente de janelas do navegador para uma bandeja laranja de downloads" width="820" />
</p>

Um site para o Instagram, outro para o X, uma cola do yt-dlp porque as flags nunca ficam na cabeça, e nenhum deles lembra do seu login. O OmniGet põe tudo isso atrás de uma caixa só: cole um link, veja o título e as qualidades, aperte Enter. yt-dlp, FFmpeg, aria2 e gallery-dl se instalam sozinhos, são verificados e ficam atualizados, então não tem Python, nem PATH, nem terminal.

### Sites e formatos

| O quê | De onde vem |
|---|---|
| **Vídeo e áudio** | Extratores próprios para YouTube (vídeos, playlists, capítulos, legendas, SponsorBlock), Instagram (posts, reels, carrosséis), TikTok (vídeos e slideshows de fotos), X/Twitter, Reddit, Threads, Bluesky, Pinterest, clipes da Twitch, Vimeo e Douyin |
| **Bilibili** | Vídeos, bangumi, favoritos, coleções, Assistir Depois e histórico; login por QR, até a qualidade que sua conta permite, danmaku em XML, ASS ou JSON |
| **Todo o resto** | Cerca de 1.800 sites pelo yt-dlp, incluindo Kuaishou, Xiaohongshu, iQiyi, Youku e VODs da Twitch |
| **Galerias de imagens** | Galerias e perfis inteiros pelo [gallery-dl](https://github.com/mikf/gallery-dl): DeviantArt, Pixiv, ArtStation, Flickr, Tumblr, Imgur, Wallhaven e outros |
| **Arquivos** | Links magnet e arquivos `.torrent` com um cliente BitTorrent embutido, links diretos baixados em partes paralelas, streams HLS, gravação de lives |
| **Telegram** | Mensagens, álbuns, intervalos de links como `t.me/channel/10-20` e chats inteiros, com login na sua própria conta |
| **De pessoa para pessoa** | Mande um arquivo para outro OmniGet com um código curto de palavras |

Vídeo de 360p até 4K, ou só o áudio em M4A, MP3, Opus, FLAC ou WAV. Legendas embutidas ou salvas ao lado do arquivo, divisão por capítulos, trechos do SponsorBlock cortados ou marcados.

### Seu primeiro download

1. Abra o OmniGet. A tela de boas-vindas pergunta seu idioma e sua pasta e instala os motores de download com um clique.
2. Copie um link: um vídeo do YouTube, um reel do Instagram, um post do X, uma coleção do Bilibili, um link magnet, a URL direta de um arquivo.
3. Cole na tela inicial, escolha a qualidade e aperte Enter.

<p align="center">
  <img src="assets/readme/downloads.png" alt="Página Downloads do OmniGet com um download em 4K ativo mostrando fase, velocidade, tempo restante e o comando exato do yt-dlp, além de itens na fila e concluídos" width="900" />
</p>

### Uma fila que não desiste

- **Retomar e tentar de novo.** Downloads interrompidos continuam de onde pararam, limites de taxa são tentados de novo com espera crescente, e a fila sobrevive a um reinício ou a um travamento.
- **Progresso honesto.** Velocidade, fase e tempo restante são lidos direto do motor, então um download parado parece parado.
- **Erros que se explicam.** Um download que falhou vem com a causa provável e uma sugestão de correção. Cada item guarda o comando exato do yt-dlp que rodou: abra, mude uma flag, tente de novo.
- **Regras.** Mande um site, um canal ou um padrão de URL para a pasta, a qualidade e as legendas que você escolheu.
- **Canais seguidos.** Verificados em segundo plano, com envios novos baixados sozinhos se você quiser. Importe suas inscrições do YouTube, NewPipe ou FreeTube.
- **Seus logins, suas contas.** Um gerenciador de cookies por site, várias contas por site com um seletor na tela inicial, e um botão de teste por domínio.
- **Nem abra a janela.** Copie um link em qualquer lugar e aperte **Ctrl+Shift+D** (**Cmd+Shift+D** no macOS) para baixar em segundo plano, ou **Ctrl+Shift+M** para só o áudio. Os dois vêm desligados até você ligar em **Configurações → Downloads**.

Também vem junto: modelos de nome de arquivo, uma pasta por plataforma, pular arquivos que já existem, metadados e capa embutidos, arquivos NFO para servidores de mídia, limite de velocidade, proxy, e ferramentas de download para cortar, juntar, normalizar o volume, gravar uma live, escolher faixas de um `.m3u8` ou `.mpd`, transformar um comando cURL em download e desligar o computador quando a fila terminar.

### A extensão do navegador

<p align="center">
  <img src="assets/readme/extension.png" alt="Loop liga um cabo de uma janela do navegador à janela do app OmniGet, com cookies passando por ele e um cadeado em cima: o pareamento fica na sua própria máquina" width="100%" />
</p>

Para Chrome, Edge, Brave e Firefox. Mande a página, um link ou até 20 links selecionados para o OmniGet com um clique ou **Alt+O**. Em qualquer outro site ela observa a página atrás de streams MP4, HLS, DASH, WebM e de áudio e lista cada um com o tamanho. Ela repassa seus cookies, então o que você abre logado baixa do mesmo jeito. A **Busca profunda** pega players que o detector não enxerga, e **Forçar H.264** ajuda computadores que travam com VP9 e AV1.

<details>
<summary>Instalar e parear a extensão</summary>

1. No OmniGet, abra **Configurações → Integrações → Extensão de navegador** e clique em **Instalar** ao lado do seu navegador. O OmniGet copia a extensão para uma pasta e abre essa pasta.
2. Chrome, Edge ou Brave: abra `chrome://extensions`, ligue o **Modo do desenvolvedor**, clique em **Carregar sem compactação** e escolha essa pasta. Firefox: abra `about:debugging#/runtime/this-firefox` e carregue o `manifest.json` dela.
3. De volta ao OmniGet, clique em **Parear extensão**. Alguns segundos depois o app mostra "Extensão conectada".

O app só escuta em `127.0.0.1` (portas 47720 a 47729) e o token de pareamento é gerado para a sua instalação, então nada sai da sua máquina. Com o OmniGet fechado, os cliques caem no link `omniget://` e a URL entra na fila na próxima vez que você abrir o app.

</details>

### OmniGet comparado com o yt-dlp e com sites de download

| | OmniGet | Só o yt-dlp | Sites de download |
|---|---|---|---|
| Sites | Extratores próprios, galerias, torrents, arquivos diretos, Telegram, mais tudo o que o yt-dlp suporta | Cerca de 1.800 | Normalmente um |
| Instalação | Instalar um app | Python ou um binário, FFmpeg, PATH, flags | Nenhuma |
| Conteúdo que exige seu login | Seus cookies, pela extensão ou por um arquivo | Exportar cookies na mão | Raramente |
| Fila | Retomar, tentar de novo com espera crescente, regras, canais seguidos | Um comando por vez | Não |
| Agentes de IA podem usar | Sim, por MCP | Por um shell | Não |
| Preço | Gratuito, GPL-3.0 | Gratuito, Unlicense | Gratuito com anúncios |

O yt-dlp é o motor em que o OmniGet roda, e o OmniGet não existiria sem ele.

---

<a id="let-claude-download-it"></a>

## Deixe o Claude baixar

<p align="center">
  <img src="assets/readme/illustration-mcp-downloads.webp" alt="Loop faz joinha enquanto um pequeno robô digita num terminal e um vídeo, uma música e uma foto caem numa pasta laranja" width="820" />
</p>

**O OmniGet é um servidor MCP para downloads.** Ligue e o Claude Code, o Cursor, o VS Code, o Codex, o Goose ou o Claude Desktop conseguem olhar um link, colocar na fila, esperar e te dizer o que falhou e por quê. O download roda no OmniGet, com a fila, as novas tentativas e os cookies que você já deu a ele, então o agente nunca precisa instalar o yt-dlp nem adivinhar as flags.

Peça em palavras simples:

```text
Baixe esta playlist como áudio e me avise quando todos os arquivos terminarem: <url>
Veja quais formatos este vídeo tem e depois baixe o melhor até 1080p.
Aqui estão 12 links. Coloque todos na fila, espere e me diga quais falharam e por quê.
Tem alguma coisa travada na minha fila do OmniGet? Tente de novo o que der para tentar.
```

### Conecte em um minuto

Com o OmniGet aberto, cole isto no Claude Code, Codex, Gemini CLI, Cursor ou outro agente:

```text
Conecte ao OmniGet: rode curl -s "http://127.0.0.1:47720/connect" e siga os passos.
```

O OmniGet vem para a frente com o nome da ferramenta e um código curto. Confira se a ferramenta mostra o mesmo código e clique em **Permitir**. O OmniGet grava o servidor dele na própria configuração dessa ferramenta, então o token nunca passa pelo chat. Reinicie a ferramenta e as ferramentas do OmniGet estarão lá.

Prefere clicar? **LLM → Configurar → MCP** lista as ferramentas de IA instaladas no seu computador, cada uma com um botão **Conectar**, mais trechos de configuração manual para Cursor, VS Code, Codex, Goose e Claude Desktop e o adaptador stdio `omniget-mcp` para clientes que precisam de um.

### O que um agente consegue fazer

| Grupo | Ferramentas |
|---|---|
| Colocar downloads na fila | Um link ou até 20 de uma vez, como vídeo (até 2160p) ou áudio |
| Acompanhar | Listar a fila, ler um download, esperar uma mudança, pausar, retomar, cancelar, tentar de novo |
| Antes de baixar | Listar os formatos de um vídeo, paginar uma playlist, verificar o link e o espaço livre |
| Quando algo falha | Ler os logs do motor já limpos, receber um diagnóstico e as opções de recuperação |
| Depois de baixar | Ler os detalhes dos arquivos prontos e ganhar acesso temporário a eles |
| Seus agentes | Listar agentes e espaços de trabalho, iniciar e acompanhar missões e jobs, ler diffs, ver aprovações pendentes |
| Memória, biblioteca, Telegram | Consultar e ensinar a memória do OmniGet, buscar nos seus livros, ler e baixar do Telegram |
| Global Downloader | Abrir o navegador dele, ver a página, prever o que uma página rende, baixar cursos e sites inteiros, criar, testar e instalar conectores |
| Seu computador | O washmybelly escaneia e planeja; executar ou desfazer um plano sempre pergunta a você na janela do OmniGet |

### Cursos e sites inteiros, com o Global Downloader

Tem coisa que não é um link só: um curso com quarenta aulas, uma área de membros, uma série atrás de login. Marque **Global Downloader** ao clicar em **Permitir** (ou peça para o agente solicitar) e o agente passa a controlar o navegador do próprio OmniGet:

```text
Baixe esse curso inteiro com o OmniGet: <url da página do curso>
Esse site ainda não tem conector. Crie um, teste e baixe tudo.
```

Você faz login uma vez nessa janela, com as suas mãos. O agente abre a página, mostra o que encontrou (módulos, aulas, PDFs, arquivos) e baixa tudo, em ordem e em pastas, pela fila do OmniGet. Quando nenhum conector conhece o site, ele cria um com você: lê as chamadas de API do site nesse navegador, escreve um `connector.js` pequeno, testa sem baixar nada e instala. Mídia com DRM aparece na lista e é pulada.

### Continua sob seu controle

- O servidor fica desligado até você ligar, e só escuta em `127.0.0.1`. Pedidos vindos de páginas web são recusados.
- Cada cliente recebe o próprio token e só as permissões que você marcar. Revogue um sem mexer nos outros.
- As aprovações dos seus agentes são respondidas na janela do OmniGet, nunca pelo MCP.

---

<a id="ai-coding-agents-in-a-window"></a>

## Agentes de programação com IA numa janela

<p align="center">
  <img src="assets/readme/central.gif" alt="Loop está sentado numa mesa de controle de missão com cinco monitores; em cada um, um robô agente de programação diferente trabalha na própria janela com linhas de diff verdes e vermelhas, enquanto Loop levanta um cartão de aprovação brilhante" width="900" />
</p>

**Um app desktop para Claude Code, Codex, Gemini CLI e outros 30 agentes de programação.** Anexe uma pasta, peça uma mudança, leia o diff antes de qualquer coisa ser gravada e desfaça o turno inteiro com um clique. Os agentes usam o login e o plano que você já paga.

| Você quer | Abra | O que acontece |
|---|---|---|
| Comandar Claude Code, Codex ou OpenCode numa interface de verdade | **LLM → Threads** | Uma thread por tarefa, na própria worktree do git, com o diff de cada turno, aprovações que esperam por você, um terminal e um botão de pull request |
| Usar várias contas do Claude Code ou do Codex | **LLM → Configurar → Conexões** | Seus logins atuais lado a lado, o uso na tela, e a troca para a próxima conta quando um limite acaba |
| Rodar agentes offline na sua própria máquina | **LLM → Configurar → Modelos e limites** | Ollama, LM Studio ou um llama.cpp gerenciado com modelos GGUF do Hugging Face, sem chave |
| Deixar trabalho rodando | **LLM → Trabalho** | Jobs que sobrevivem ao fechar a janela, Loops que repetem até o `npm test` passar, missões com critérios de conclusão, gatilhos por cron e webhook |
| Pôr um prompt para competir entre vários agentes | **LLM → Trabalho → Arena** | Cada agente na própria worktree; compare tempo, custo e diffs, depois faça o merge do vencedor |
| Instalar skills, agentes, comandos, hooks e servidores MCP em todo lugar | **LLM → Catálogo** | 3.800+ componentes convertidos para o formato de cada uma das 34 ferramentas, com um diff antes de qualquer coisa ser gravada |
| Instalar, atualizar e fazer login nas CLIs dos agentes | **LLM → Ferramentas** | 34 CLIs de programação detectadas, instaladas e atualizadas numa tela só |
| Ver quanto cada agente te custou | **LLM → Mais** | Tokens e custo por dia, modelo, ferramenta, conta e projeto a partir dos logs que suas ferramentas já guardam, e busca de texto completo nas sessões de 22 ferramentas |
| Aprovar pelo celular | **LLM → Mais → Acesso remoto** | Desligado por padrão; pareie com um QR code e responda aprovações longe da mesa |

### Threads: Claude Code, Codex e qualquer agente ACP

- **Drivers de verdade, não um invólucro.** O Claude Code roda como um processo longo por thread, o Codex pelo `codex app-server`, o OpenCode pelo `opencode serve`, e Gemini CLI, Qwen Code, Copilot, Cursor, goose, Kiro e outros por um cliente completo do [Agent Client Protocol](https://agentclientprotocol.com).
- **Cada turno é um checkpoint.** Os turnos ficam salvos em refs ocultas do git; **Editar daqui** volta os arquivos e a conversa para qualquer turno.
- **Aprovações que esperam.** Uma edição de arquivo mostra o diff completo, um comando de shell mostra o comando e a pasta, e o pedido espera por você em vez de expirar.
- **Traga suas sessões do terminal.** Uma sessão do Claude Code ou do Codex que você rodou num terminal pode ser importada e retomada no driver certo.

### Um agente de programação com permissões, sandbox e desfazer

- **Uma pasta.** O agente embutido lê, busca, edita e roda comandos só dentro da pasta que você anexou; um caminho fora dela vira uma pergunta à parte.
- **Permissão com a evidência na tela.** Tudo o que grava pergunta antes. **Sempre** guarda uma regra pelo prefixo do comando (`git status *`, `npm run test *`), e comandos encadeados precisam de uma regra para cada parte.
- **Um shell em sandbox** no macOS: sem rede, gravando só dentro da pasta.
- **Desfazer volta o turno inteiro.** Antes da primeira gravação o OmniGet tira um snapshot da pasta num git paralelo que nunca toca no seu repositório.
- **Orçamentos por agente:** dólares por dia, tokens por turno, chamadas de ferramenta por turno.
- **Suas próprias chaves** para OpenAI, Anthropic, OpenRouter, Gemini, DeepSeek, Groq, xAI, Mistral e outros, com um roteador que passa para o próximo quando uma cota acaba.

Medido num projeto de demonstração com um teste falhando (build de release, Apple Silicon): o Claude Code pelo OmniGet corrige em cerca de 15 segundos; o `qwen3:8b` no Ollama faz o mesmo em cerca de 3 minutos.

### Jobs, Loops e missões

<p align="center">
  <img src="assets/readme/agents-loop.gif" alt="Página Loops do OmniGet: um Loop rodado pelo Claude Code passa de Rodando para Concluído com o motivo de parada check_passed depois de uma de três rodadas" width="900" />
</p>

- **Um job é um turno de agente que sobrevive à janela.** Ele guarda o estado, o log, o diff e quanto custou, na própria worktree, que você integra ou descarta.
- **Um Loop repete até uma verificação passar.** Dê a ele um prompt e um comando (`npm test`, `cargo test`); ele para quando a verificação sai com 0 ou quando acabam as rodadas ou os minutos, e retoma a rodada depois de um travamento.
- **Gatilhos.** Uma linha de cron ou um webhook na ponte local inicia um job.

### Memória que aprende

- Depois de cada turno, o OmniGet guarda os fatos que valem a pena: o que você afirmou, o que aconteceu e quando, e as pessoas e coisas envolvidas.
- A busca funciona por palavras, significado, ligações e tempo, em inglês, português e cerca de 100 outros idiomas, com modelos de busca que rodam no seu computador.
- Só as suas próprias palavras viram uma preferência declarada; todo o resto fica marcado como palpite. Corrija uma memória e a antiga sai de cena; esqueça e ela some do índice e dos vetores.
- Claude Code, Codex ou Cursor conectados por MCP também podem consultar e ensinar essa memória.
- Inspirada no [Hindsight](https://github.com/vectorize-io/hindsight), num único arquivo SQLite. Numa conversa do LoCoMo, com o mesmo modelo e o mesmo juiz para os dois, o OmniGet respondeu 67 a 68% das perguntas e o Hindsight 64,5%.

### O Catálogo

<p align="center">
  <img src="assets/readme/illustration-catalog.webp" alt="Loop toca uma pequena loja com prateleiras de cartões brilhantes e entrega um cartão a uma fila de robôs de formatos diferentes; cada cartão muda para caber no encaixe do peito daquele robô" width="820" />
</p>

Mais de 3.800 skills, agentes, comandos, hooks, servidores MCP e plugins do [claude-code-templates](https://github.com/davila7/claude-code-templates), [anthropics/skills](https://github.com/anthropics/skills), [openai/skills](https://github.com/openai/skills), [obra/superpowers](https://github.com/obra/superpowers) e outros, cada um convertido para o formato da ferramenta que você escolher: Claude Code, Codex, Gemini CLI, Cursor, Copilot, OpenCode, Windsurf, Zed e outras 26. Antes de qualquer instalação você vê o diff de cada arquivo e cada comando que um hook vai rodar, e um scanner com 127 regras procura injeção de prompt, leitura de credenciais e `curl | sh`. Desinstalar remove só o que o OmniGet gravou.

### Fique de olho nos seus limites

<p align="center">
  <img src="assets/readme/illustration-usage.webp" alt="Loop segura uma lupa diante de um painel do tamanho de uma parede com um gráfico de barras, um mapa de calor de calendário e um gráfico de rosca, enquanto rolos de logs chegam de pequenas casas de robôs" width="820" />
</p>

Uma faixa fina na borda da tela mostra um anel por assistente de programação (Claude, Codex, Copilot, Cursor, Grok, Kimi, OpenCode): quanto de cada limite já foi usado e quando ele reinicia. Os mesmos números ficam num ícone da barra de menus e no `omniget-cli usage`. Cada leitor só abre o login que aquela ferramenta já guarda na sua máquina, em modo somente leitura.

---

<a id="the-canvas"></a>

## O Canvas

**Um quadro infinito em que seus agentes trabalham lado a lado.** Cada agente roda no próprio nó de terminal (Claude Code, Codex, Gemini CLI, OpenCode ou um modelo local), ao lado de notas, arquivos, árvores de arquivos, links e prévias de navegador ao vivo.

- **Cordas entre agentes.** Desenhe uma corda e dois agentes podem fazer perguntas um ao outro, trocar mensagens ou passar trabalho adiante. A corda acende enquanto uma mensagem está a caminho.
- **Maestro.** Um agente orquestrador recruta agentes, dá papéis a eles e liga as cordas para você.
- **Andares.** Quadros separados num mesmo espaço de trabalho, cada um numa pasta comum, numa worktree do git ou num clone, com mudanças que você aplica ou descarta.
- **Rotinas.** Mande um prompt para um agente uma vez, a cada poucas horas, todo dia ou toda semana.
- **Ambientes.** Rode um nó localmente, no tmux, por SSH, no Docker, numa sandbox ou no WSL.
- **Visitas.** **Abrir para visitas** te dá um código curto. Um amigo digita o código e assiste ao seu quadro, somente leitura; os terminais ficam privados a menos que você compartilhe um. Suas chaves e seus agentes ficam na sua máquina.
- **Omni.** Um pequeno companheiro que avisa quando um agente terminou, travou ou falhou, e um bichinho de desktop que reage ao que seus agentes fazem e responde aos pedidos de permissão deles.

---

<a id="superpowers"></a>

## Superpowers

Já vêm dentro do app, em **Superpoderes** na barra lateral. Cada um pode ser desligado em **Configurações → Integrações**.

| | O que faz |
|---|---|
| **Música** | Um app de música para as faixas do seu computador: reprodução sem intervalos, ReplayGain, equalizador, letras sincronizadas, rádio pela internet, playlists inteligentes e ferramentas em lote para tags, volume, andamento, tom, conversão e duplicatas |
| **OmniTube** | YouTube e YouTube Music com a sua conta ou sem conta: sem anúncios, SponsorBlock, DeArrow, Return YouTube Dislike, esconder Shorts e comentários, bloquear canais e palavras, e regras de download automático para os canais que você segue |
| **Watch** | Seus filmes, séries e cursos com progresso, notas, transcrições e flashcards, TV ao vivo de playlists M3U, Jellyfin e Chromecast |
| **Ler** | Uma sala de leitura para EPUB, MOBI, PDF, quadrinhos e mangá, com livros gratuitos do Project Gutenberg e do Standard Ebooks, MangaDex e MANGA Plus com aviso de capítulo novo, catálogos OPDS, sincronização com o KOReader e envio para Kindle ou Kobo |
| **Telegram** | Seus chats, busca de mídia e downloads por mensagem, álbum, intervalo ou chat inteiro, exportações, regras de monitoramento e transcrição local, com opções de proxy para redes que bloqueiam o Telegram |
| **Global Downloader** | Um navegador controlado pelo OmniGet que captura a mídia que cada aba toca, e conectores que baixam um curso ou site inteiro em ordem. O Claude Code consegue controlá-lo e criar conectores novos |
| **League of Legends** | Um ajudante para o seu cliente do League aberto: scouting, ouro ao vivo, runas com um clique. Toda automação fica desligada até você ligar |

---

<a id="washmybelly"></a>

## washmybelly

<p align="center">
  <img src="assets/readme/washmybelly-clean-up.webp" alt="A tela de Limpeza do washmybelly no macOS: 39,2 GB de lixo encontrados, divididos em Desenvolvimento, Apps, Navegadores, Privacidade, Pacotes, Sistema, Logs e Seus arquivos" width="900" />
</p>

**Limpeza, desinstalação, privacidade e desempenho, numa tela só e com desfazer.** Ele só mostra o que existe no seu computador: itens de Mac num Mac, itens de Windows no Windows, itens de Linux no Linux.

- **Limpeza:** caches, logs, sobras de navegadores e de gerenciadores de pacotes, pastas `node_modules` e `target`, duplicatas e pastas vazias.
- **Disco:** o que ocupa o espaço, num mapa que você pode ir abrindo, e a saúde de cada disco.
- **Desinstalar:** um app junto com o que ele deixa para trás, e as sobras de apps que você já apagou. Conhece Homebrew, winget, MSI, Steam, Flatpak, Snap e os gerenciadores de pacotes do Linux.
- **Inicialização:** o que abre no login, desligado sem ser apagado.
- **Privacidade:** o catálogo do [privacy.sexy](https://privacy.sexy) para o seu sistema com o estado atual de cada ajuste, as permissões de apps do Mac e um bloqueio de rastreadores.
- **Ajustes do Windows:** debloat e configurações do Win11Debloat, winutil e Sophia Script, cada um com "Voltar ao padrão do Windows".
- **Atualizações, energia e jogos:** atualizações pendentes de todos os gerenciadores de pacotes numa lista só, saúde da bateria, um modo de jogo, e números honestos de FPS e frametime.

**Nada roda sozinho.** Ele escaneia, mostra o plano (o que sai, quanto espaço, qual o risco), você aprova, ele executa e guarda um recibo. Arquivos vão para uma quarentena ou para a Lixeira, configurações são salvas antes de mudar, e o **Histórico** desfaz tudo. Sem "acelerador de RAM", sem "limpador" de registro: o que ele informa é medido.

---

<a id="install"></a>

## Instalar

**[Baixe o OmniGet em getomniget.com](https://getomniget.com/download)**. A página escolhe o arquivo certo para o seu sistema e lista o SHA-256 de cada arquivo. As atualizações chegam dentro do app.

| Sistema | Arquivos | Também |
|---|---|---|
| **Windows 10 e 11** | Instalador `.exe`, `.exe` portátil, `.msi` | `winget install -e --id tonhowtf.OmniGet` |
| **macOS 10.15 ou mais novo** | `.dmg` para Apple Silicon e para Intel | |
| **Linux** | `.deb`, `.rpm` e `.AppImage`, para x86_64 e ARM64 | AUR: `omniget-bin` |

<a id="the-first-launch-warning"></a>

### O aviso da primeira abertura

O OmniGet é distribuído sem um certificado pago de assinatura de código, então cada sistema pede confirmação uma vez. Confira o hash na página de download se quiser ter certeza de que o arquivo é nosso.

- **Windows:** o SmartScreen mostra uma caixa azul. Clique em **Mais informações** e depois em **Executar assim mesmo**.
- **macOS:** se o Gatekeeper disser que o app está "danificado", arraste o OmniGet para Aplicativos e rode:

  ```bash
  xattr -cr /Applications/omniget.app
  codesign --force --deep --sign - /Applications/omniget.app
  ```

- **AppImage do Linux no Debian 12+ ou Ubuntu 24.04+:** se falhar com um erro de libfuse, rode `sudo apt install libfuse2`, ou use o `.deb`.

<details>
<summary>Linux: uma janela vazia quando um player de vídeo abre</summary>

O WebKitGTK toca mídia pelo GStreamer. A maioria dos desktops tem os plugins; o Arch e imagens mínimas podem não ter:

```bash
sudo apt install gstreamer1.0-plugins-good gstreamer1.0-plugins-bad gstreamer1.0-libav        # Debian, Ubuntu
sudo dnf install gstreamer1-plugins-good gstreamer1-plugins-bad-free gstreamer1-plugin-libav  # Fedora
sudo pacman -S gst-plugins-good gst-plugins-bad gst-libav                                     # Arch
```

</details>

**Modo portátil (Windows).** Coloque um arquivo vazio chamado `portable.txt` ao lado do `.exe`. Configurações, cookies, o banco de dados e os motores de download vão para uma pasta `data` ao lado dele, então o app inteiro cabe num pendrive.

---

<a id="command-line"></a>

## Linha de comando

O `omniget-cli` vem em toda release, ao lado do `omniget-mcp`, o adaptador stdio para clientes MCP. Adicione `--json` a qualquer comando para ter uma saída legível por máquina.

```bash
omniget-cli info <url>                       # título, formatos e tamanho; não baixa nada
omniget-cli download <url> -q 1080 -o ~/Videos
omniget-cli download <url> --audio-only --subs en,pt
omniget-cli batch links.txt -m 3             # uma URL por linha, 3 de cada vez
omniget-cli import-cookies cookies.txt       # formato Netscape
omniget-cli tg dl https://t.me/channel/10-20  # mensagens, álbuns e intervalos do Telegram

# pelo app desktop aberto
omniget-cli claude [account]                 # Claude Code numa das suas contas
omniget-cli usage --watch 60                 # janelas de 5 horas e 7 dias e gasto por conta
omniget-cli agent run "Fix the failing test" --workspace .
omniget-cli agent loop "Make the tests pass" --check "npm test" --rounds 5
omniget-cli agentkit search <term>           # o Catálogo pelo terminal
omniget-cli wmb scan junk                    # washmybelly; executar pergunta na janela do OmniGet
```

### O plugin para o Claude Code

A pasta [`claude-plugin/`](claude-plugin/omniget) é um plugin para o [Claude Code](https://claude.com/claude-code) que funciona com ou sem o app desktop. Cole o link de um vídeo junto com um pedido e as skills dele buscam ou transcrevem o vídeo.

```text
/omniget:setup                       # instala yt-dlp, FFmpeg e omniget-cli depois de uma confirmação
/omniget:fetch <url> [--audio]       # o arquivo de mídia, em ~/Downloads/omniget
/omniget:transcribe <url|file>       # legendas, depois whisper.cpp local, depois Gemini ou OpenAI se você adicionou uma chave
/omniget:research <url>              # uma nota em Markdown com referências [mm:ss]
/omniget:connect                     # conecta o Claude Code ao servidor MCP do app desktop
/omniget:doctor                      # o que está instalado e o que falta
```

---

<a id="privacy"></a>

## Privacidade

<p align="center">
  <img src="assets/readme/illustration-privacy.webp" alt="Loop segura um notebook com o ícone de download do OmniGet e um cadeado verde, na frente de um escudo laranja" width="640" />
</p>

Tudo roda no seu computador. Não tem conta, não tem servidor nosso no meio e não tem telemetria sobre o que você baixa. Cookies e chaves de API ficam no seu perfil local. O OmniGet só se conecta à internet por conta própria para acessar os sites que você pediu, o getomniget.com para atualizações, e o provedor de IA que você configurou, quando você usa.

O OmniGet baixa o que a sua própria sessão já consegue abrir. Ele não contorna DRM, não fura paywalls e não compartilha credenciais. Ele serve para cópias pessoais, backups e conteúdo que você tem o direito de guardar; você é responsável por respeitar os direitos autorais e os termos de cada plataforma. O texto completo está no app em **Sobre → Termos**.

---

<a id="frequently-asked-questions"></a>

## Perguntas frequentes

### O OmniGet é gratuito?
Sim. Gratuito e de código aberto sob GPL-3.0, sem plano pago, sem anúncios e sem conta.

### O OmniGet é uma interface gráfica do yt-dlp?
Em parte. Ele instala o yt-dlp, verifica, mantém atualizado e põe as opções dele numa janela. Além disso, tem extratores próprios, gallery-dl, torrents, Telegram, uma fila com retomada e novas tentativas, uma extensão de navegador, um servidor MCP e agentes de IA.

### Como baixo um vídeo ou uma playlist sem terminal?
Cole o link na tela inicial, escolha a qualidade e aperte Enter. Playlists, legendas, capítulos e só o áudio são opções na mesma janela.

### Ele retoma downloads interrompidos?
Sim. Arquivos parciais são mantidos e continuados, limites de taxa são tentados de novo com espera crescente, e a fila sobrevive a um reinício.

### O Claude Code pode baixar vídeos para mim?
Sim. Ligue o servidor MCP em **LLM → Configurar → MCP**, conecte o Claude Code com o prompt de uma linha acima e peça em palavras simples. O Claude coloca o download na fila do OmniGet, espera e te conta o que aconteceu.

### Posso rodar o Claude Code ou o Codex sem terminal?
Sim. Seus logins atuais aparecem em **LLM → Configurar → Conexões**, e **LLM → Threads** dá a cada tarefa uma janela com diffs, aprovações e desfazer. Gemini CLI, OpenCode, goose e outros agentes ACP entram do mesmo jeito.

### Um agente de IA pode continuar trabalhando até os testes passarem?
Sim. Um Loop repete rodadas até o seu comando de verificação, como `npm test`, sair com 0. Ele continua com a janela fechada, e o `omniget-cli agent loop` inicia um pelo terminal.

### Funciona offline com o Ollama?
Sim. Aponte para o Ollama, o LM Studio ou o llama.cpp gerenciado e os agentes rodam na sua máquina sem chave. Aí o seu código nunca sai do seu computador.

### Precisa de Python, Node ou terminal?
Não. Baixe o app, abra e cole um link.

### O que aconteceu com os plugins e a seção Tools?
Os dois foram removidos na 1.0.0. Música, Ler, Telegram e os outros recursos que eram plugins agora vêm dentro do app em **Superpoderes**. A palavra "plugin" agora só se refere ao plugin do OmniGet para o Claude Code.

### O macOS diz que o app está danificado. O que eu faço?
Rode os dois comandos da [seção sobre a primeira abertura](#the-first-launch-warning). Você só faz isso uma vez.

### Qual pacote do Linux eu escolho?
Debian e Ubuntu: `.deb`. Fedora, openSUSE e a família RHEL: `.rpm`. Qualquer outro: `.AppImage`. Arch: `omniget-bin` do AUR.

---

<a id="support-omniget"></a>

## Apoie o OmniGet

<p align="center">
  <img src="assets/readme/illustration-support.webp" alt="Loop ergue uma estrela dourada ao lado de um pote com um coração, moedas e uma planta brotando" width="520" />
</p>

O OmniGet é gratuito e continua gratuito: sem plano pago, nada bloqueado. Uma pessoa só faz o app, e os sites que ele lê mudam toda semana.

- **Doe** em [getomniget.com/community](https://getomniget.com/community): GitHub Sponsors, Afdian ou PIX.
- **Conte o que quebrou** em [getomniget.com/contact](https://getomniget.com/contact), com o seu sistema, a versão do OmniGet e o link que falhou.
- **Traduza** no [Weblate](https://hosted.weblate.org/engage/omniget/). O app fala 12 idiomas.
- **Fale com a gente** no [Discord](https://discord.gg/jgdxyPy7Vn).
- **Conte para alguém.** Vai escrever um post ou gravar um vídeo? O [MEDIA-KIT.md](MEDIA-KIT.md) tem os fatos e os links.

---

<a id="build-from-source"></a>

## Compilar do código-fonte

Você precisa de [Rust](https://rustup.rs/) (a toolchain está fixada em `rust-toolchain.toml`), [Node.js](https://nodejs.org/) (veja `.nvmrc`) e [pnpm](https://pnpm.io/).

```bash
pnpm install
pnpm tauri dev
```

O `pnpm tauri dev` usa um perfil próprio (`omniget-dev`), então nunca mexe nas configurações, chaves e histórico do OmniGet que você tem instalado. Defina `OMNIGET_DEV_REAL_PROFILE=1` para usar o perfil real.

<details>
<summary>Dependências de build no Linux</summary>

```bash
sudo apt-get install -y libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev \
  libayatana-appindicator3-dev librsvg2-dev patchelf libasound2-dev libpipewire-0.3-dev clang libclang-dev
```

</details>

Para um build de produção local sem a chave de assinatura da release:

```bash
pnpm tauri build --config '{"bundle":{"createUpdaterArtifacts":false}}'
```

Stack: Tauri 2, Rust, SvelteKit com Svelte 5, SQLite, yt-dlp, FFmpeg, gallery-dl, aria2, librqbit e grammers. Vai mudar alguma coisa? Leia o [CONTRIBUTING.md](CONTRIBUTING.md) primeiro.

### Aviso aos donos de plataformas

Se você é dono de uma plataforma e quer que o OmniGet pare de suportá-la, mande um e-mail para **tonhowtf@gmail.com** de um endereço da empresa. O site sai da documentação e do código, e os domínios dele entram numa lista de exclusão que o app aplica. O processo e a lista atual estão em [PLATFORM-OWNERS.md](PLATFORM-OWNERS.md).

### Apoiado em código aberto

O OmniGet é construído sobre [yt-dlp](https://github.com/yt-dlp/yt-dlp), [FFmpeg](https://ffmpeg.org/), [gallery-dl](https://github.com/mikf/gallery-dl), [aria2](https://aria2.github.io/), [librqbit](https://github.com/ikatson/rqbit), [grammers](https://github.com/Lonami/grammers), [SponsorBlock](https://sponsor.ajay.app/), [cat-catch](https://github.com/xifangczy/cat-catch), [whisper.cpp](https://github.com/ggml-org/whisper.cpp), [Tauri](https://tauri.app) e [Svelte](https://svelte.dev). Os agentes pegaram ideias emprestadas do [opencode](https://github.com/anomalyco/opencode), [Codex](https://github.com/openai/codex), [aider](https://github.com/Aider-AI/aider), [cline](https://github.com/cline/cline), [compozy](https://github.com/compozy/compozy), do [Agent Client Protocol](https://agentclientprotocol.com) e do [Hindsight](https://github.com/vectorize-io/hindsight); o washmybelly usa dados e ideias do [privacy.sexy](https://github.com/undergroundwires/privacy.sexy), [BleachBit](https://github.com/bleachbit/bleachbit), [Win11Debloat](https://github.com/Raphire/Win11Debloat), [winutil](https://github.com/ChrisTitusTech/winutil), [Sophia Script](https://github.com/farag2/Sophia-Script-for-Windows) e outros. A lista completa, com as licenças, está em [THIRD_PARTY.md](THIRD_PARTY.md). Obrigado a todo mundo que mantém esses projetos.

O Loop é o mascote do OmniGet. Fan art é bem-vinda; a arte original não pode ser usada comercialmente nem redistribuída modificada. As ilustrações deste README foram feitas com o [Higgsfield](https://higgsfield.ai) a partir da arte original do Loop.

<p align="center">
  <a href="https://getomniget.com/download"><b>Baixe o OmniGet</b></a> · <a href="https://getomniget.com">getomniget.com</a> · <a href="LICENSE">GPL-3.0</a>
</p>
