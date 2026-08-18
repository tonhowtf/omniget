<!--
SEO / discovery: OmniGet é um downloader livre e de código aberto para Windows, macOS e Linux.
O GitHub limita os topics a 20, então esta lista tem exatamente 20 — adicionar um significa remover outro.
Escolhidos pela chance de ranquear: o OmniGet é #1 em estrelas em course-downloader, udemy-downloader,
hotmart-downloader, bilibili-downloader, tiktok-downloader, twitter-downloader,
reddit-downloader e media-downloader. Topics amplos (rust, svelte, tauri, desktop-app,
open-source) foram deixados de fora de propósito: lá o campo tem de 10 mil a 110 mil repositórios e este
repositório não consegue ranquear, então essas vagas valem mais em outro lugar.
downloader, download-manager, media-downloader, video-downloader, youtube-downloader,
yt-dlp, yt-dlp-gui, course-downloader, udemy-downloader, hotmart-downloader,
bilibili-downloader, tiktok-downloader, instagram-downloader, twitter-downloader,
reddit-downloader, telegram-downloader, twitch-downloader, subtitle-downloader,
epub-reader, spaced-repetition
-->

<p align="center">
  <img src="static/loop.png" alt="Loop, o mascote do OmniGet" width="120" />
</p>

<h1 align="center">OmniGet</h1>

<h3 align="center">Baixe cursos da Udemy, YouTube, músicas, livros e mais de 1.800 sites em um só app. Sem terminal.</h3>

<p align="center">
  <a href="README.md">English</a>
  | <a href="README_zh_CN.md">中文</a>
  | <a href="README.ru.md">Русский</a>
  | <b>Português (BR)</b>
</p>

<p align="center">
  <a href="https://github.com/tonhowtf/omniget/releases/latest"><img src="https://img.shields.io/github/v/release/tonhowtf/omniget?style=for-the-badge&label=release" alt="Versão mais recente" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-GPL--3.0-green?style=for-the-badge" alt="Licença GPL-3.0" /></a>
  <a href="https://github.com/tonhowtf/omniget/stargazers"><img src="https://img.shields.io/github/stars/tonhowtf/omniget?style=for-the-badge" alt="Estrelas no GitHub" /></a>
  <a href="https://github.com/tonhowtf/omniget/releases"><img src="https://img.shields.io/github/downloads/tonhowtf/omniget/total?style=for-the-badge&label=downloads" alt="Total de downloads" /></a>
  <a href="https://hosted.weblate.org/engage/omniget/"><img src="https://hosted.weblate.org/widget/omniget/frontend-json/svg-badge.svg" alt="Status da tradução" /></a>
</p>

<p align="center">
  <b>O OmniGet é um aplicativo de desktop livre e de código aberto para Windows, macOS e Linux.</b> Ele baixa cursos online (Hotmart, Udemy, Kiwify, Skool, Teachable e outros), vídeo e áudio do YouTube, TikTok, Instagram, Twitter/X, Reddit e mais de 1.800 outros sites, além de músicas e livros. Tudo toca dentro do próprio app. Sem linha de comando, sem Python, sem configuração, e seus arquivos ficam no seu computador. Distribuído sob a licença GPL-3.0.
</p>

<p align="center">
  <a href="#baixar-e-instalar"><b>Baixar para Windows, macOS ou Linux</b></a>
  &nbsp;·&nbsp;
  <a href="#uma-tecla-e-o-download-já-começa"><b>Ver o atalho de um clique</b></a>
</p>

<p align="center">
  <img src="assets/readme/en/home-hero.png" alt="Tela inicial do OmniGet, um downloader gratuito de cursos, vídeos, músicas e livros para Windows, macOS e Linux" width="880" />
</p>

---

## Baixar e instalar

Escolha seu sistema, baixe a versão mais recente e abra. Não há instalador cheio de etapas nem necessidade de permissão de administrador.

<table>
  <tr>
    <th>Plataforma</th>
    <th>Como instalar</th>
  </tr>
  <tr>
    <td><strong>Windows</strong></td>
    <td>
      <a href="https://github.com/tonhowtf/omniget/releases/latest"><img alt="Baixar o OmniGet para Windows" src="https://img.shields.io/badge/Windows-Portable_EXE-0078D6?style=for-the-badge&logo=windows&logoColor=white" height="38"></a>
      <br/>
      <sub>Baixe o <code>.exe</code> em Releases e dê dois cliques. Ele é portátil, então roda de qualquer pasta. Também existe um instalador <code>.msi</code> e o comando <code>winget install -e --id tonhowtf.OmniGet</code>, se você preferir a linha de comando.</sub>
    </td>
  </tr>
  <tr>
    <td><strong>macOS</strong></td>
    <td>
      <a href="https://github.com/tonhowtf/omniget/releases/latest"><img alt="Baixar o OmniGet para macOS" src="https://img.shields.io/badge/macOS-DMG-000000?style=for-the-badge&logo=apple&logoColor=white" height="38"></a>
      <br/>
      <sub>Abra o <code>.dmg</code> e arraste o OmniGet para a pasta Aplicativos. Leia a observação sobre a primeira abertura logo abaixo.</sub>
    </td>
  </tr>
  <tr>
    <td><strong>Linux</strong></td>
    <td>
      <a href="https://github.com/tonhowtf/omniget/releases/latest"><img alt="Baixar o OmniGet para Linux em deb, rpm ou AppImage" src="https://img.shields.io/badge/Linux-deb_·_rpm_·_AppImage-FFAA33?style=for-the-badge&logo=linux&logoColor=white" height="38"></a>
      <br/>
      <sub>Debian e Ubuntu: baixe o <code>.deb</code>. Fedora e openSUSE: o <code>.rpm</code>. Qualquer outra distro: o <code>.AppImage</code>. Há builds para x86_64 e ARM64.</sub>
    </td>
  </tr>
</table>

<sub><strong>AppImage no Debian 12+ ou Ubuntu 24.04+:</strong> essas versões não incluem o FUSE 2, de que o AppImage precisa. Se <code>./omniget.AppImage</code> falhar com um erro de libfuse, rode <code>sudo apt install libfuse2</code> ou inicie com <code>./omniget.AppImage --appimage-extract-and-run</code>. O <code>.deb</code> evita isso por completo.</sub>

### ⚠️ Leia isto antes de abrir pela primeira vez

O OmniGet é de código aberto e não é assinado com um certificado pago, então na primeira vez que você abrir o app o sistema pode exibir um aviso. Isso é esperado, e os passos abaixo resolvem de vez. Em qualquer caso, seus arquivos continuam locais.

**macOS (esse é o principal, o app não abre na primeira tentativa).** O Gatekeeper do macOS bloqueia apps não assinados. Depois de mover o OmniGet para a pasta Aplicativos, abra o Terminal e rode estas duas linhas:

```bash
xattr -cr /Applications/omniget.app
codesign --force --deep --sign - /Applications/omniget.app
```

Depois abra o OmniGet normalmente. Você só faz isso uma vez.

**Windows.** O SmartScreen pode mostrar um aviso azul na primeira execução. Clique em **Mais informações** e depois em **Executar assim mesmo**. Isso é comum em apps de código aberto sem certificado pago de assinatura de código.

### Modo portátil, para pendrive ou PC com restrições

Crie um arquivo vazio chamado `portable.txt` (ou `.portable`) ao lado do `.exe` e reinicie o app. O OmniGet passa a guardar configurações, banco de dados, cookies, plugins, caches e as ferramentas embutidas yt-dlp e FFmpeg em uma pasta `data` ao lado do executável. Nada é gravado em `AppData\Roaming` nem em qualquer outra pasta do usuário, então a instalação inteira viaja no pendrive. Sem esse arquivo, o OmniGet usa o diretório de dados padrão do usuário.

Livre e de código aberto sob GPL-3.0. As atualizações rodam em segundo plano, sem incomodar. As ferramentas embutidas (yt-dlp e FFmpeg) se instalam sozinhas, e o yt-dlp é verificado por SHA256 antes de rodar. Os plugins se instalam na primeira abertura e também se atualizam sozinhos, sem nada para você configurar.

---

## Uma tecla, e o download já começa

Essa é a parte que conquista as pessoas. Copie qualquer link, um vídeo do YouTube, um tweet, uma mensagem do Discord, uma música, um magnet, e pressione o atalho global **`Ctrl+Shift+D`** (**`Cmd+Shift+D`** no macOS). O OmniGet lê sua área de transferência e baixa em segundo plano. Você nem precisa abrir a janela.

<p align="center">
  <img src="assets/readme/global-hotkey.png" alt="Atalho global de download do OmniGet: pressione o atalho e o link da área de transferência baixa direto para sua pasta" width="760" />
</p>

Funciona de qualquer lugar do sistema. Navegando, conversando, lendo, não importa qual app está na frente. Copiar, pressionar, pronto. O arquivo cai na sua pasta e a fila cuida do resto. Se você preferir ver uma prévia antes, é só colar o link na omnibox da tela inicial, conferir as opções de qualidade e clicar em baixar.

---

## O problema que isso resolve

Você já tem o yt-dlp aberto em um terminal. Achou um script de baixar cursos que quebra a cada atualização do site. Tem um app separado para música, e nenhum deles conversa entre si. Cada download vira três ferramentas e um monte de copiar e colar.

O OmniGet faz tudo isso em uma janela só. Cole um link de curso, um link do YouTube, um TikTok, um magnet, um podcast, e ele descobre o resto. O arquivo cai na sua pasta e toca ali mesmo, dentro do app.

É o único app de código aberto que baixa um curso completo da Udemy ou da Hotmart, vídeo e áudio de mais de 1.800 sites e a sua biblioteca de música, tudo em um lugar só, sem linha de comando. Ele conquistou milhares de estrelas no GitHub nos primeiros meses porque essa combinação não existia em nenhum outro lugar.

---

## Como o OmniGet se compara ao yt-dlp e a outros downloaders

Ferramentas diferentes fazem trabalhos diferentes. Isto mostra onde o OmniGet se encaixa, não é um placar.

### OmniGet vs yt-dlp

O [yt-dlp](https://github.com/yt-dlp/yt-dlp) é o motor de extração, e o OmniGet o utiliza. O yt-dlp é uma ferramenta de linha de comando: você instala o Python, aprende as flags, escreve o seletor de formato e ganha um controle incomparável sobre mais de 1.800 sites. Ele é excelente nisso, e o OmniGet não existiria sem ele.

O OmniGet é o app em volta dele. Você instala um arquivo, cola um link e vê uma prévia com opções de qualidade. O yt-dlp e o FFmpeg se instalam e se atualizam sozinhos, e o yt-dlp é verificado por SHA256 antes de rodar. Em cima disso ficam uma fila que tenta de novo e retoma downloads, um histórico, um player de cursos, um leitor e uma biblioteca de música. Se você se vira bem no terminal e só quer os arquivos, use o yt-dlp direto. Se você quer uma biblioteca que dá para assistir e ler de verdade, use o OmniGet.

### OmniGet vs downloaders de propósito único

A maioria dos downloaders faz um site bem feito. O OmniGet cobre cursos, vídeo, áudio, imagens, torrents e Telegram em uma fila só, e depois reproduz o resultado. A contrapartida é honesta: uma ferramenta dedicada a uma plataforma às vezes vai dar suporte a um caso raro primeiro.

### OmniGet vs downloaders pagos de cursos

Downloaders pagos de cursos costumam cobrar assinatura por algo a que você já tem acesso. O OmniGet é GPL-3.0, não tem conta, nem anúncios, nem plano pago, e baixa apenas o que a sua própria sessão logada já consegue acessar.

---

## O que o OmniGet baixa

Cole um link. O OmniGet detecta o site, mostra uma prévia com opções de qualidade e baixa. Se o [yt-dlp](https://github.com/yt-dlp/yt-dlp) suporta um site, o OmniGet baixa dele, o que dá cerca de mil sites a mais do que a tabela abaixo.

| Categoria | Plataformas |
|-----------|-------------|
| Cursos online | Hotmart, Udemy, Kiwify, Gumroad, Teachable, Kajabi, Skool, Wondrium, Thinkific, Rocketseat |
| Vídeo e áudio | YouTube, Instagram, TikTok, Twitter/X, Reddit, Twitch, Pinterest, Vimeo, Bluesky, Bilibili |
| Bilibili (a fundo) | Entre na conta para 4K, HDR, Dolby Vision, Hi-Res lossless, Dolby Atmos. Danmaku (XML/ASS/JSON), NFO para Kodi e Jellyfin, 11 tipos de URL (UGC, 番剧, 课程, 收藏夹, UP主, 每周必看, 稍后再看, 历史记录, b23.tv) |
| Plataformas asiáticas | Douyin (抖音), Xiaohongshu (小红书), Kuaishou (快手), Youku (优酷), iQiyi (爱奇艺), Tencent Video, Mango TV |
| Galerias de imagens | DeviantArt, Pixiv, ArtStation, Flickr, Tumblr, álbuns do Imgur, Kemono, Newgrounds, image boards |
| Perfis em lote | Subreddits inteiros e suas páginas de ordenação, perfis de usuários do Reddit e perfis do X/Twitter |
| Arquivos e transferência | Links `.torrent` e magnet, além de transferência P2P direta entre dois computadores com um código curto |

Coisas que as pessoas procuram, e que o OmniGet faz:

- **Baixar um curso online completo**, cada aula e cada PDF anexado, e depois assistir dentro do app e retomar de onde parou.
- **Baixar um vídeo do YouTube ou uma playlist inteira**, escolher a qualidade ou pegar só o áudio em MP3, M4A, Opus, FLAC ou WAV.
- **Baixar TikTok, Instagram, Twitter/X, Reddit**: posts, reels, stories, carrosséis e galerias.
- **Baixar em lote** uma lista de links de um arquivo de texto, um subreddit inteiro, um perfil de usuário do Reddit ou um perfil do X/Twitter, tudo de uma vez.
- **Baixar só um trecho do vídeo**, definindo início e fim.
- **Baixar legendas** em qualquer idioma, embutir no arquivo ou gerar com o Whisper quando não existirem.
- **Pular patrocínios** com o SponsorBlock, e embutir metadados e miniaturas automaticamente.
- **Telegram Direct Uploader & Leech Bot**: envie mídia direto para o Telegram via User Session ou Bot Token, com suporte a Chat IDs numéricos, `@usernames`, miniaturas personalizadas, edição de nome e extensão do arquivo, modos **Send As** Vídeo/Documento/Áudio e divisão automática em 2 GB (Free) / 4 GB (Premium).
- **Seletor dinâmico de qualidade do YouTube**: veja e escolha as resoluções realmente disponíveis (`4K 2160p`, `2K 1440p`, `1080p`, `720p`, `Somente áudio`), extraídas direto dos metadados do yt-dlp.
- **Fase de merge explícita e ETA suave**: saiba exatamente quando áudio e vídeo estão sendo unidos (`[Merger]`, `[ffmpeg]`), com estimativa de tempo ao vivo (`ETA ~3m 20s`).
- **Limitador de banda no cabeçalho**: limite a velocidade de download na hora, direto da barra de cabeçalho de Downloads.
- **Toast interativo de detecção da área de transferência**: detecta URLs de vídeo copiadas e mostra um banner de um clique.
- **Detecção de dependências do sistema**: detecta e usa binários de `yt-dlp`, `FFmpeg` e `PDFium` instalados no sistema, com indicadores de origem (`PATH`, `Managed`, `Flatpak`).
- **Seguir um canal** e baixar automaticamente os uploads novos, com notificação na bandeja do sistema.
- **Baixar do Bilibili na qualidade máxima**: entre na conta uma vez e libere 4K, HDR, áudio Hi-Res lossless e Dolby Atmos.

Os downloads são confiáveis, não um jogo de adivinhação. Velocidade e ETA vêm direto do downloader em vez de serem inventados a partir de uma porcentagem, então continuam corretos mesmo quando o tamanho do arquivo é desconhecido ou a transmissão é ao vivo. Um travamento aparece como travamento, e não como um "3 segundos restantes" congelado. A fila retoma downloads interrompidos e tenta de novo com backoff.

---

## Ele também reproduz tudo por dentro

Essa é a parte que ninguém espera. O OmniGet não é só onde você baixa. É onde você assiste, lê e escuta.

### Abra um curso e assista de verdade

Baixe o curso inteiro (Hotmart, Udemy, Kiwify, Skool, Teachable, Kajabi, Wondrium, Thinkific) e assista sem sair do app. Retome no segundo exato em que parou. Faça anotações que pulam para aquele momento quando você clica nelas. Leia os PDFs anexados lado a lado.

<p align="center">
  <img src="assets/screenshot-courses.png" alt="Player de cursos do OmniGet com anotações por timestamp e PDFs anexados" width="760" />
  <br/>
  <em>Player de cursos, anotações fixadas em timestamps, anexos na mesma janela.</em>
</p>

### Leia livros, livros de verdade

Solte uma pasta com PDFs e EPUBs. O OmniGet extrai as capas, busca títulos e autores e abre cada um em um leitor embutido com marcações, marcadores, modo foco e um tema com cara de papel, que descansa os olhos. Também abre quadrinhos em CBZ e arquivos TXT ou HTML.

<p align="center">
  <img src="assets/screenshot-reader.png" alt="Leitor de EPUB e PDF embutido do OmniGet com marcações e modo foco" width="760" />
  <br/>
  <em>Leitor com marcações, painel de anotações e modo foco.</em>
</p>

### Música, do jeito que você lembra

Aponte o OmniGet para a sua pasta de músicas e ele mostra suas faixas do jeito que o iTunes fazia: álbuns com capas, artistas com discografias, uma fila que se comporta.

- Toca MP3, FLAC, M4A, OGG, Opus, tudo o que você já tem.
- Busca **letras sincronizadas** que rolam junto com a música.
- Conecta ao **Spotify, SoundCloud, YouTube Music, Qobuz e Last.fm**, para suas playlists e curtidas ficarem ao lado dos arquivos locais.
- **Equalizador** com presets, variantes de tema escuro por capa de álbum, um painel de atividade com suas faixas mais tocadas e uma presença no Discord que mostra o que você está ouvindo.

<p align="center">
  <img src="assets/screenshot-music.png" alt="Player de música do OmniGet com visualização de álbuns, letras sincronizadas e fontes de streaming" width="820" />
  <br/>
  <em>Biblioteca local, letras sincronizadas, fontes de streaming, um só player.</em>
</p>

---

## Configurações que não atrapalham

As configurações são agrupadas e discretas. As opções comuns estão à mão, as mais avançadas ficam a um toque de distância, e uma caixa de busca encontra qualquer coisa em todas as categorias e destaca para você.

<p align="center">
  <img src="assets/readme/en/settings-drill.png" alt="Configurações do OmniGet com barra lateral agrupada e seções em camadas" width="820" />
  <br/>
  <em>Barra lateral agrupada, uma lista clara, cada seção abre a própria página.</em>
</p>

<p align="center">
  <img src="assets/readme/en/settings-output.png" alt="Configurações de download do OmniGet: pasta de saída, organizar por plataforma, modelo de nome de arquivo, pular arquivos existentes" width="820" />
  <br/>
  <em>Saída, qualidade, legendas e o resto, com uma dica curta embaixo de cada controle.</em>
</p>

---

## Para quem joga League of Legends, se quiser

O OmniGet tem um menu de League of Legends embutido. Ele vem **desligado**. Nada conecta, nada observa, e o menu nem aparece na barra lateral até você ativá-lo em **Configurações → Avançado → League of Legends**. Deixe desligado e o OmniGet se comporta exatamente como sempre.

Ligue e ele lê o cliente do League em execução do mesmo jeito que o próprio cliente se lê, sem conta, sem login e sem nenhum site de builds de terceiros no meio.

- **Análise da partida** para os dois times. Elo, forma recente, KDA, os campeões que cada jogador realmente joga e observações curtas que ele deduz sozinho: sequência de vitórias, one-trick, taxa de vitória baixa. Escreva sua própria nota sobre um jogador e ela volta na próxima vez que vocês se encontrarem.
- **Probabilidade de vitória** com estatística feita direito. As taxas de vitória são puxadas na direção da média conforme o tamanho da amostra, então cinquenta por cento em mil partidas e cinquenta por cento em dez não são tratados como a mesma evidência, e o resultado sempre vem com uma faixa em vez de um decimal falso. O matchmaking busca partidas equilibradas, então a resposta honesta costuma ficar perto do empate, e o app diz isso em vez de fingir o contrário.
- **Economia ao vivo** enquanto você joga. Ouro, CS e nível dos dez jogadores, e a diferença para o oponente da sua rota.
- **Metas por função**, editáveis. Um suporte não é julgado por CS e um caçador não é julgado como um atirador.
- **Runas e feitiços de invocador** recomendados pelo próprio cliente do jogo, aplicados em um clique. Ele só substitui a página que o OmniGet criou, nunca as suas.
- **Tiers de campeões** por função, com taxas de vitória, escolha e banimento.
- **Busca de jogadores** por qualquer Riot ID, com elo, histórico de campeões e maestria.
- **Automações**, todas opcionais: aceitar partidas, escolher e banir a partir da sua lista de prioridades e pegar um campeão do banco no ARAM.

<p align="center">
  <em>Tudo acima vem desligado por padrão e cada automação tem seu próprio interruptor.</em>
</p>

---

## Plugins que se instalam sozinhos

O OmniGet já vem com o conjunto completo de plugins (cursos, estudo, Telegram, conversão e outros) e eles se configuram sozinhos na primeira abertura. Também se atualizam sozinhos quando sai uma versão nova, então você nunca precisa correr atrás de um download. Ligue ou desligue qualquer um pela barra lateral, e desinstale os que não quiser. O que você remover continua removido.

<p align="center">
  <img src="assets/readme/en/plugins.png" alt="Plugins e dependências do OmniGet, pareamento da extensão de navegador e ferramentas gerenciadas em forma de tabela" width="820" />
  <br/>
  <em>Plugins e ferramentas embutidas, gerenciados para você, exibidos em uma tabela clara.</em>
</p>

---

## As pequenas coisas que fazem diferença

Discretas, ali quando você precisa.

- **Oficina de legendas** que abre SRT, VTT e ASS, com ferramentas de temporização, sincronização por dois pontos, localizar e substituir, correção automática em um clique, tradução por IA e correção gramatical por IA, além de uma forma de onda com marcadores de troca de cena.
- **Timer pomodoro** que pausa seu vídeo quando a sessão termina.
- **App de notas** com links bidirecionais, um diário e um grafo de conhecimento.
- **Painel de progresso** com contador de sequência, metas diárias e um mapa de calor anual.
- **Conversor FFmpeg** para arquivos locais, sem precisar de internet.
- **Navegador de conversas do Telegram** que permite salvar fotos, vídeos e arquivos de qualquer conversa.
- **Extensão de navegador** (Chrome e Firefox) que entrega a página atual ao OmniGet com um clique.
- **Atalho global** (`Ctrl+Shift+D`, ou `Cmd+Shift+D` no macOS) que baixa a URL que estiver na área de transferência.
- **9 idiomas** e **14 temas**, incluindo Catppuccin, Dracula, One Dark Pro e três variantes e-ink.

---

## Perguntas frequentes

**O OmniGet é gratuito?**
Sim. O OmniGet é livre e de código aberto sob GPL-3.0, sem conta, sem anúncios e sem plano pago.

**Preciso de uma conta para usar o OmniGet?**
Não. O OmniGet não pede conta nem cadastro próprio. Você só faz login em uma plataforma quando o conteúdo em si exige, como um curso pago que você comprou ou uma transmissão premium do Bilibili, e essa sessão fica no seu computador.

**O OmniGet consegue baixar uma playlist do YouTube ou um canal inteiro?**
Sim. Cole a URL da playlist e o OmniGet coloca todos os vídeos na fila. Você também pode seguir um canal, e o OmniGet baixa os uploads novos automaticamente e mostra uma notificação na bandeja.

**O OmniGet retoma um download interrompido?**
Sim. A fila guarda os arquivos parcialmente baixados e continua de onde parou em vez de recomeçar, e tenta de novo com backoff quando um site limita sua taxa. Fechar o app ou perder a conexão não joga o progresso fora.

**Em quais formatos o OmniGet salva?**
Vídeo em MP4, MKV ou WebM, e áudio em MP3, M4A, Opus, FLAC ou WAV. Legendas em SRT, VTT ou ASS, que podem ser embutidas no arquivo de vídeo. Livros abrem em PDF, EPUB, CBZ, TXT e HTML.

**O OmniGet baixa conteúdo pago que eu já comprei?**
O OmniGet baixa o que a sua própria sessão logada já consegue abrir, como um curso da Udemy ou da Hotmart que você pagou. Ele não contorna DRM, não quebra paywalls e não compartilha credenciais. Conteúdo a que você não tem acesso continua inacessível.

**Preciso de terminal ou Python para usar o OmniGet?**
Não. O OmniGet é um app de desktop comum. Baixe, abra, cole um link. O yt-dlp e o FFmpeg vêm embutidos e se atualizam sozinhos. A única vez em que você pode precisar do Terminal é no passo único da primeira abertura no macOS, descrito acima.

**O OmniGet não abre no macOS, o que eu faço?**
Rode os dois comandos de Terminal na [observação sobre a primeira abertura](#️-leia-isto-antes-de-abrir-pela-primeira-vez). O Gatekeeper bloqueia apps de código aberto não assinados, e essas linhas limpam a marcação. Você faz isso uma vez só.

**O OmniGet é só uma interface para o yt-dlp?**
O OmniGet usa o yt-dlp por baixo para os mais de 1.800 sites genéricos, com extratores nativos para as plataformas grandes, além de uma interface de verdade, uma fila, uma biblioteca e players embutidos em cima. Então sim, e bem mais do que uma interface.

**O OmniGet baixa um curso completo da Udemy ou da Hotmart?**
Sim. Você faz login uma vez na plataforma, escolhe o curso, e o OmniGet baixa cada aula e cada anexo, e depois reproduz tudo com anotações por timestamp.

**Quais sites o OmniGet suporta?**
Cursos online, YouTube, TikTok, Instagram, Twitter/X, Reddit, Twitch, Vimeo, Bilibili, Pinterest, Bluesky, as principais plataformas asiáticas, galerias de imagens, torrents e magnets, além de cerca de 1.800 outros via yt-dlp.

**O OmniGet funciona no Windows, macOS e Linux?**
Sim, nos três, tanto em x86_64 quanto em ARM64. No Windows sai como `.exe` portátil, instalador `.msi` e pacote winget. No macOS sai como `.dmg` para Apple Silicon e Intel. No Linux sai como `.deb`, `.rpm` e `.AppImage`.

**Quais distribuições Linux são suportadas?**
O OmniGet roda em qualquer Linux desktop moderno. Quem usa Debian e Ubuntu deve pegar o `.deb`, Fedora e openSUSE o `.rpm`, e qualquer outra distro o `.AppImage`. No Debian 12+ e no Ubuntu 24.04+ o AppImage precisa de `sudo apt install libfuse2`, porque essas versões deixaram de incluir o FUSE 2; o `.deb` não tem essa exigência.

**O OmniGet roda totalmente portátil de um pendrive?**
Sim. Crie um arquivo vazio chamado `portable.txt` (ou `.portable`) ao lado do `.exe` e reinicie. O OmniGet passa a guardar configurações, banco de dados, cookies, plugins, caches e o yt-dlp/FFmpeg embutidos em uma pasta `data` ao lado do executável — nada é gravado em `AppData\Roaming` nem em outras pastas do usuário. Sem esse arquivo, o app usa o diretório de dados padrão do usuário.

**O OmniGet baixa só o áudio, ou só um trecho?**
Sim. O OmniGet extrai o áudio em MP3, M4A, Opus, FLAC ou WAV, ou você define início e fim para baixar só a parte que precisa.

**Meus downloads no OmniGet são privados?**
Sim. Tudo no OmniGet roda localmente e seus arquivos nunca saem do seu computador. Não há telemetria sobre o que você baixa.

**O OmniGet baixa do Bilibili em 4K, HDR ou Hi-Res lossless?**
Sim, com uma conta do Bilibili conectada. O OmniGet conversa com a API oficial do Bilibili e respeita exatamente o que a sua assinatura 大会员 (premium) libera. Sem login, os downloads continuam funcionando via yt-dlp em qualidade padrão.

---

## Compilar a partir do código-fonte

Para desenvolvedores. Se você só quer usar o OmniGet, [pegue uma release](#baixar-e-instalar).

```bash
git clone https://github.com/tonhowtf/omniget.git
cd omniget
pnpm install
pnpm tauri dev
```

Requer [Rust](https://rustup.rs/), [Node.js](https://nodejs.org/) 18+ e [pnpm](https://pnpm.io/).

<details>
<summary>Dependências de build no Linux</summary>

```bash
sudo apt-get install -y libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev patchelf
```

</details>

Build de produção: `pnpm tauri build`.

### Interface de linha de comando (compile você mesmo)

O repositório também contém o `omniget-cli`, um binário Rust pequeno que torna o OmniGet scriptável. Ele sai junto com cada release — baixe `omniget-cli-<versão>-<alvo>` na [release mais recente](https://github.com/tonhowtf/omniget/releases/latest), para Windows, macOS (Intel e Apple Silicon) ou Linux. Para compilar a partir deste repositório:

```bash
cargo build --release -p omniget-cli
```

```bash
omniget info <url>                     # prévia de título, formatos e tamanho, sem baixar nada
omniget download <url> -q 1080 -o ~/Videos
omniget download <url> --audio-only --subs en,pt
omniget batch links.txt -m 3           # uma URL por linha, 3 por vez
omniget import-cookies cookies.txt     # formato Netscape
```

O app de desktop nunca precisa disso. Existe para cron jobs, dotfiles e scripts.

---

## Contribuir

**Comunidade.** Dúvidas, ajuda e conversas sobre releases acontecem no [Discord](https://discord.gg/jgdxyPy7Vn).

Achou um bug ou tem uma ideia de funcionalidade? [Abra uma issue](https://github.com/tonhowtf/omniget/issues). Pull requests são bem-vindos, veja o [CONTRIBUTING.md](CONTRIBUTING.md).

O OmniGet é traduzido no [Weblate](https://hosted.weblate.org/engage/omniget/). Escolha um idioma, traduza no navegador, e o Weblate abre um pull request automaticamente.

### Colaboradores

Obrigado a todos que participaram deste projeto!

[![Contributors](https://contrib.rocks/image?repo=tonhowtf/omniget)](https://github.com/tonhowtf/omniget/graphs/contributors)

### Desenvolvendo plugins

Os recursos de Cursos, Telegram e Conversão do OmniGet são todos plugins — bibliotecas dinâmicas em Rust construídas sobre o [`omniget-plugin-sdk`](src-tauri/omniget-plugin-sdk) — e plugins de terceiros são bem-vindos. O [Guia de Desenvolvimento de Plugins](docs/plugin-development.md) cobre a arquitetura, um início rápido a partir do [template de plugin](https://github.com/tonhowtf/omniget-plugin-template), o manifesto e a API do host, notas honestas sobre estabilidade de ABI e como entrar no [registro de plugins](https://github.com/tonhowtf/omniget-plugins).

## Aviso aos donos de plataformas

Se você representa uma plataforma listada e tem alguma preocupação, envie um e-mail para **tonhowtf@gmail.com** a partir de um endereço corporativo. A plataforma sai da lista na hora.

## Aspectos legais

O OmniGet é feito para uso pessoal. Respeite os direitos autorais e os termos de serviço de cada plataforma. Você é responsável pelo que baixar.

## Licença

[GPL-3.0](LICENSE). O nome OmniGet, o logo e o mascote Loop são marcas do projeto, não cobertas pela licença do código.
