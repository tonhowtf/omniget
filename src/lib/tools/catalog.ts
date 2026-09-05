/**
 * Catálogo da seção Tools.
 *
 * Cada ferramenta pertence a uma categoria (YouTube, Edição de vídeo,
 * Instagram…) e declara em quais sistemas roda. A busca do hub é genérica:
 * "instagram" acha a categoria e tudo que está dentro dela, "legenda" acha
 * ferramentas de qualquer categoria. Para isso cada entrada carrega uma
 * lista de palavras-chave em português e inglês além do nome traduzido.
 *
 * Nome e descrição ficam no i18n (`tools.categories.<id>.*` e
 * `tools.catalog.<id>.*`); aqui só vive o que não muda com o idioma.
 */

export type OsName = "windows" | "macos" | "linux";
export const ALL_OS: OsName[] = ["windows", "macos", "linux"];

export type ToolStatus = "ready" | "beta" | "soon";

/** Filtro de plataforma do hub. `cross` = roda nos três sistemas. */
export type PlatformFilter = "all" | "cross" | OsName;

export type ToolIconSpec = {
  /**
   * `brand:<arquivo em static/brands>`, `glyph:<arquivo em static/icons>` ou
   * `app:<arquivo .png em static/apps>` (ícone de app pronto, sem gradiente).
   */
  icon: string;
  from: string;
  to: string;
  /** Gradiente com três paradas (Instagram). */
  via?: string;
};

export type ToolCategory = ToolIconSpec & {
  id: string;
  keywords: string[];
  order: number;
};

export type ToolEntry = ToolIconSpec & {
  id: string;
  category: string;
  keywords: string[];
  platforms: OsName[];
  status: ToolStatus;
  /**
   * Para onde o tile leva. Ferramentas que já existem em outra parte do app
   * apontam para a rota delas; as que rodam dentro do Tools apontam para
   * `/tools/<categoria>/<id>` e o runner decide o que montar.
   */
  href?: string;
  /**
   * Qual componente a página da ferramenta monta. Os `yt-*` vêm do painel de
   * downloads; o resto é uma chave do mapa RUNNERS em
   * `routes/tools/[category]/[tool]/+page.svelte`.
   */
  runner?: string;
};

const P = {
  red: { from: "#FF6B6B", to: "#E0303A" },
  orange: { from: "#FFB340", to: "#F28500" },
  yellow: { from: "#FFD426", to: "#E0A800" },
  green: { from: "#4CD964", to: "#2AA845" },
  teal: { from: "#48CFDF", to: "#1A9EB5" },
  blue: { from: "#5AA9FF", to: "#1E6FE8" },
  indigo: { from: "#6E8CFF", to: "#3D5BF0" },
  purple: { from: "#C77DFF", to: "#8E3FD8" },
  pink: { from: "#FF5E7A", to: "#E0203F" },
  brown: { from: "#D8A15C", to: "#A66A24" },
  gray: { from: "#A3A3A8", to: "#6F6F75" },
  ink: { from: "#4A4A52", to: "#1C1C21" },
} as const;

export const CATEGORIES: ToolCategory[] = [
  { id: "youtube", icon: "brand:youtube", from: "#FF4E45", to: "#E10E0E", order: 10, keywords: ["youtube", "yt", "video", "vídeo", "canal", "channel"] },
  { id: "video", icon: "glyph:film-slate", ...P.purple, order: 20, keywords: ["video", "vídeo", "edicao", "edição", "editor", "editar", "capcut", "cortar", "timeline", "render"] },
  { id: "instagram", icon: "brand:instagram", from: "#8A3AB9", via: "#E1306C", to: "#FCAF45", order: 30, keywords: ["instagram", "insta", "ig", "seguidores", "followers", "reels", "stories", "unfollow", "baixar", "download"] },
  { id: "x", icon: "app:x", from: "#1D9BF0", to: "#0F6FB8", order: 32, keywords: ["x", "twitter", "tweet", "post", "thread", "grok", "xai", "bookmarks", "favoritos", "seguidores", "followers", "unfollow", "arquivo", "archive"] },
  { id: "pinterest", icon: "app:pinterest", from: "#FF5B6E", to: "#E60023", order: 35, keywords: ["pinterest", "pin", "pins", "board", "boards", "secao", "seção", "section", "moodboard", "inspiracao", "inspiração", "ideias", "ideas"] },
  { id: "spotify", icon: "brand:spotify", from: "#3BE477", to: "#1DB954", order: 40, keywords: ["spotify", "spicetify", "musica", "música", "music", "tema", "theme", "playlist"] },
  { id: "pdf", icon: "glyph:file-pdf", ...P.red, order: 50, keywords: ["pdf", "documento", "document", "pagina", "página", "page", "juntar", "merge"] },
  { id: "system", icon: "glyph:broom", ...P.teal, order: 60, keywords: ["sistema", "system", "limpeza", "limpar", "clean", "cleaner", "cache", "disco", "disk", "pc", "windows", "otimizar", "optimize"] },
  { id: "automation", icon: "glyph:cursor-click", ...P.orange, order: 70, keywords: ["automacao", "automação", "automation", "clique", "click", "macro", "bot"] },
  { id: "ai", icon: "glyph:key", ...P.indigo, order: 80, keywords: ["ia", "ai", "inteligencia artificial", "chave", "key", "api", "openai", "claude", "chatgpt", "modelo", "model", "llm", "ollama", "custo", "cost"] },
  { id: "speech", icon: "glyph:waveform", ...P.pink, order: 15, keywords: ["voz", "voice", "fala", "speech", "transcrever", "transcribe", "whisper", "legenda", "subtitle", "tts", "dublagem", "dub", "traduzir", "translate"] },
  { id: "documents", icon: "glyph:file-text", ...P.blue, order: 55, keywords: ["documento", "document", "slides", "slideshare", "calameo", "scribd", "google docs", "apresentacao", "apresentação", "galeria", "gallery", "imagens", "images"] },
  { id: "images", icon: "glyph:images", ...P.yellow, order: 57, keywords: ["imagem", "image", "imagens", "foto", "photo", "upscale", "redimensionar", "resize", "ocr"] },
  { id: "files", icon: "glyph:folder-simple", ...P.brown, order: 62, keywords: ["arquivo", "arquivos", "file", "files", "duplicado", "duplicate", "renomear", "rename", "buscar", "search", "pasta", "folder"] },
  { id: "downloads", icon: "glyph:cloud-arrow-down", ...P.gray, order: 65, keywords: ["download", "baixar", "aria2", "acelerado", "hls", "dash", "m3u8", "mpd", "manifesto", "manifest"] },
  { id: "phone", icon: "glyph:device-mobile", ...P.green, order: 75, keywords: ["celular", "phone", "telefone", "android", "iphone", "kde connect", "enviar", "send", "compartilhar", "share"] },
];

export const TOOLS: ToolEntry[] = [
  // ── YouTube ──────────────────────────────────────────────────────────
  { id: "yt-download", category: "youtube", icon: "glyph:download-simple", ...P.red, platforms: ALL_OS, status: "ready", href: "/", keywords: ["baixar", "download", "video", "vídeo", "mp4", "mp3", "audio", "áudio", "playlist"] },
  { id: "yt-metadata", category: "youtube", icon: "glyph:article", ...P.blue, platforms: ALL_OS, status: "ready", runner: "yt-metadata", keywords: ["metadata", "metadados", "info", "descricao", "descrição", "description", "titulo", "título", "json"] },
  { id: "yt-thumbnails", category: "youtube", icon: "glyph:image-square", ...P.orange, platforms: ALL_OS, status: "ready", runner: "yt-thumbnails", keywords: ["thumbnail", "thumb", "capa", "cover", "imagem", "image", "miniatura"] },
  { id: "yt-subtitles", category: "youtube", icon: "glyph:subtitles", ...P.green, platforms: ALL_OS, status: "ready", runner: "yt-subtitles", keywords: ["legenda", "legendas", "subtitle", "subtitles", "srt", "vtt", "bilingue", "bilíngue", "cc", "closed caption"] },
  { id: "yt-comments", category: "youtube", icon: "glyph:chat-text", ...P.teal, platforms: ALL_OS, status: "ready", runner: "yt-comments", keywords: ["comentarios", "comentários", "comments", "capitulos", "capítulos", "chapters", "csv", "json", "exportar", "export"] },
  { id: "yt-livechat", category: "youtube", icon: "glyph:record", ...P.pink, platforms: ALL_OS, status: "ready", runner: "yt-livechat", keywords: ["live", "chat", "ao vivo", "stream", "replay", "csv", "json"] },
  { id: "yt-workshop", category: "youtube", icon: "glyph:text-t", ...P.purple, platforms: ALL_OS, status: "ready", runner: "yt-workshop", keywords: ["legenda", "subtitle", "workshop", "traduzir", "translate", "editar legenda", "sincronizar", "sync"] },
  { id: "yt-sponsorblock", category: "youtube", icon: "glyph:fast-forward", ...P.green, platforms: ALL_OS, status: "ready", runner: "sponsorblock", keywords: ["sponsorblock", "patrocinio", "patrocínio", "sponsor", "pular", "skip", "intro", "outro", "segmentos", "segments", "sem anuncio"] },
  { id: "yt-dislikes", category: "youtube", icon: "glyph:thumbs-down", ...P.gray, platforms: ALL_OS, status: "ready", runner: "ryd", keywords: ["dislike", "dislikes", "deslike", "return youtube dislike", "likes", "avaliacao", "avaliação", "rating"] },
  { id: "yt-frames", category: "youtube", icon: "glyph:film-strip", ...P.orange, platforms: ALL_OS, status: "ready", runner: "yt-frames", keywords: ["thumbnail", "frame", "quadro", "clickbait", "capa real", "hq1", "hq2", "hq3", "miniatura"] },
  { id: "yt-codec", category: "youtube", icon: "glyph:cpu", ...P.ink, platforms: ALL_OS, status: "ready", runner: "codec", keywords: ["h264", "codec", "vp9", "av1", "60fps", "extensao", "extensão", "h264ify", "hardware"] },

  // ── Edição de vídeo ──────────────────────────────────────────────────
  { id: "video-clip", category: "video", icon: "glyph:scissors", ...P.purple, platforms: ALL_OS, status: "ready", href: "/misc/file-clip", keywords: ["cortar", "corte", "clip", "trim", "trecho", "recortar", "cut"] },
  { id: "video-convert", category: "video", icon: "glyph:arrows-clockwise", ...P.red, platforms: ALL_OS, status: "ready", href: "/convert", keywords: ["converter", "convert", "reencodar", "reencode", "formato", "format", "mp4", "mkv", "webm", "gif", "compress", "comprimir"] },
  { id: "video-record", category: "video", icon: "glyph:video-camera", ...P.pink, platforms: ALL_OS, status: "beta", runner: "record", keywords: ["gravar", "record", "tela", "screen", "captura", "capture", "replay", "estudio", "estúdio", "studio"] },
  { id: "video-editor", category: "video", icon: "glyph:film-strip", ...P.indigo, platforms: ALL_OS, status: "soon", keywords: ["editor", "timeline", "capcut", "multi-track", "transicao", "transição", "transition", "montagem"] },
  { id: "video-captions", category: "video", icon: "glyph:closed-captioning", ...P.green, platforms: ALL_OS, status: "ready", href: "/tools/speech/speech-transcribe", keywords: ["legenda automatica", "legenda automática", "auto caption", "whisper", "transcrever", "transcribe", "transcricao", "transcrição"] },
  { id: "video-tts", category: "video", icon: "glyph:speaker-high", ...P.orange, platforms: ALL_OS, status: "ready", href: "/tools/speech/speech-tts", keywords: ["tts", "texto para voz", "text to speech", "narracao", "narração", "voz", "voice", "locucao", "locução"] },

  // ── X / Twitter ─────────────────────────────────────────────────────
  { id: "x-download", category: "x", icon: "glyph:download-simple", from: "#1D9BF0", to: "#0F6FB8", platforms: ALL_OS, status: "ready", href: "/", keywords: ["baixar", "download", "video", "vídeo", "imagem", "image", "gif", "post", "tweet", "mp4"] },
  { id: "x-thread", category: "x", icon: "glyph:text-align-left", ...P.blue, platforms: ALL_OS, status: "ready", runner: "x-thread", keywords: ["thread", "unroll", "desenrolar", "fio", "thread reader", "markdown", "ler", "exportar", "export", "pdf"] },
  { id: "x-card", category: "x", icon: "glyph:image-square", ...P.indigo, platforms: ALL_OS, status: "ready", runner: "x-card", keywords: ["imagem", "image", "screenshot", "card", "print", "tweet to image", "png", "instagram", "compartilhar", "share"] },
  { id: "x-profile", category: "x", icon: "glyph:chart-line-up", ...P.green, platforms: ALL_OS, status: "ready", runner: "x-profile", keywords: ["perfil", "profile", "analytics", "estatisticas", "estatísticas", "engajamento", "engagement", "melhor horario", "melhor horário", "best time", "seguidores", "followers", "raio-x"] },
  { id: "x-media", category: "x", icon: "glyph:images", ...P.orange, platforms: ALL_OS, status: "ready", runner: "x-media", keywords: ["midia", "mídia", "media", "fotos", "photos", "videos", "vídeos", "perfil inteiro", "em massa", "bulk", "batch", "original", "galeria", "gallery"] },
  { id: "x-search", category: "x", icon: "glyph:magnifying-glass", ...P.teal, platforms: ALL_OS, status: "ready", runner: "x-search", keywords: ["busca", "buscar", "search", "avancada", "avançada", "advanced", "from:", "since:", "until:", "min_faves", "filter", "trends", "assuntos do momento", "trending"] },
  { id: "x-bookmarks", category: "x", icon: "glyph:bookmark-simple", ...P.yellow, platforms: ALL_OS, status: "beta", runner: "x-bookmarks", keywords: ["favoritos", "bookmarks", "salvos", "saved", "exportar", "export", "pastas", "folders", "json", "csv", "markdown", "html"] },
  { id: "x-unfollow", category: "x", icon: "glyph:user-minus", ...P.red, platforms: ALL_OS, status: "beta", runner: "x-unfollow", keywords: ["unfollow", "quem nao me segue", "quem não me segue", "nao segue de volta", "não segue de volta", "not following back", "seguidores", "followers", "seguindo", "following", "deixar de seguir", "whitelist"] },
  { id: "x-archive", category: "x", icon: "glyph:folder-open", ...P.brown, platforms: ALL_OS, status: "ready", runner: "x-archive", keywords: ["arquivo", "archive", "baixar seus dados", "your data", "zip", "tweets.js", "historico", "histórico", "estatisticas", "estatísticas", "offline"] },
  { id: "x-grok", category: "x", icon: "app:grok", from: "#2B2B2B", to: "#0A0A0A", platforms: ALL_OS, status: "beta", runner: "x-grok", keywords: ["grok", "xai", "ia", "ai", "chat", "resumir", "summarize", "busca no x", "x search", "live search", "api key", "perguntar"] },

  // ── Instagram ────────────────────────────────────────────────────────
  // Tudo pela sessão web (cookies capturados pela extensão). Estudos 67-68.
  { id: "ig-download", category: "instagram", icon: "glyph:download-simple", from: "#FCAF45", to: "#F77737", platforms: ALL_OS, status: "ready", runner: "ig-download", keywords: ["baixar", "download", "post", "foto", "photo", "video", "vídeo", "reel", "reels", "igtv", "carrossel", "carousel", "link", "url"] },
  { id: "ig-bulk", category: "instagram", icon: "glyph:list-checks", from: "#F77737", to: "#E1306C", platforms: ALL_OS, status: "ready", runner: "ig-bulk", keywords: ["baixar varios", "baixar vários", "bulk", "lote", "lista de links", "muitos links", "txt"] },
  { id: "ig-audio", category: "instagram", icon: "glyph:music-notes", from: "#E1306C", to: "#C13584", platforms: ALL_OS, status: "ready", runner: "ig-audio", keywords: ["audio", "áudio", "mp3", "m4a", "som do reel", "musica", "música", "extrair audio"] },
  { id: "ig-stories", category: "instagram", icon: "glyph:record", from: "#833AB4", to: "#5851DB", platforms: ALL_OS, status: "ready", runner: "ig-stories", keywords: ["stories", "story", "melhores amigos", "close friends", "anonimo", "anônimo", "sem ser visto", "ver stories"] },
  { id: "ig-highlights", category: "instagram", icon: "glyph:bookmark-simple", from: "#5851DB", to: "#405DE6", platforms: ALL_OS, status: "ready", runner: "ig-highlights", keywords: ["highlights", "destaques", "stories salvos", "baixar destaques"] },
  { id: "ig-story-viewers", category: "instagram", icon: "glyph:binoculars", from: "#F56040", to: "#FCAF45", platforms: ALL_OS, status: "ready", runner: "ig-story-viewers", keywords: ["quem viu", "viewers", "visualizacoes", "visualizações", "meus stories", "story views"] },
  { id: "ig-viewer", category: "instagram", icon: "glyph:binoculars", from: "#405DE6", to: "#5B51D8", platforms: ALL_OS, status: "ready", runner: "ig-viewer", keywords: ["viewer", "ver perfil", "perfil", "profile", "bio", "seguidores", "followers", "privado", "private", "anonimo", "anônimo"] },
  { id: "ig-avatar", category: "instagram", icon: "glyph:image-square", from: "#C13584", to: "#833AB4", platforms: ALL_OS, status: "ready", runner: "ig-avatar", keywords: ["foto de perfil", "profile picture", "avatar", "hd", "alta resolucao", "alta resolução", "zoom"] },
  { id: "ig-profile-media", category: "instagram", icon: "glyph:images", from: "#FD1D1D", to: "#E1306C", platforms: ALL_OS, status: "ready", runner: "ig-profile-media", keywords: ["baixar perfil", "todos os posts", "all posts", "reels do perfil", "marcados", "tagged", "salvos", "saved", "em massa", "bulk", "backup"] },
  { id: "ig-unfollowers", category: "instagram", icon: "glyph:user-minus", from: "#E1306C", to: "#B31B54", platforms: ALL_OS, status: "ready", runner: "ig-unfollowers", keywords: ["unfollow", "unfollowers", "nao me segue", "não me segue", "quem nao segue", "seguidores", "followers", "deixar de seguir", "whitelist", "lista branca"] },
  { id: "ig-fans", category: "instagram", icon: "glyph:users-three", from: "#F56040", to: "#FD1D1D", platforms: ALL_OS, status: "ready", runner: "ig-fans", keywords: ["fas", "fãs", "fans", "eu nao sigo", "me seguem", "mutuos", "mútuos", "mutuals", "remover seguidor", "remove follower"] },
  { id: "ig-mutuals", category: "instagram", icon: "glyph:users-three", from: "#833AB4", to: "#E1306C", platforms: ALL_OS, status: "ready", runner: "ig-mutuals", keywords: ["mutuos", "mútuos", "mutuals", "seguem de volta", "amigos", "friends"] },
  { id: "ig-unfollowed", category: "instagram", icon: "glyph:clock-counter-clockwise", from: "#B31B54", to: "#833AB4", platforms: ALL_OS, status: "ready", runner: "ig-unfollowed", keywords: ["quem deixou de me seguir", "unfollowed me", "perdi seguidores", "lost followers", "novos seguidores", "historico", "histórico", "snapshot"] },
  { id: "ig-ghosts", category: "instagram", icon: "glyph:eye-slash", from: "#5B51D8", to: "#405DE6", platforms: ALL_OS, status: "ready", runner: "ig-ghosts", keywords: ["fantasmas", "ghost followers", "inativos", "inactive", "nao curtem", "não curtem", "engajamento", "top fans"] },
  { id: "ig-whitelist", category: "instagram", icon: "glyph:shield-check", from: "#405DE6", to: "#5851DB", platforms: ALL_OS, status: "ready", runner: "ig-whitelist", keywords: ["whitelist", "lista branca", "proteger", "nunca deixar de seguir", "excecoes", "exceções"] },
  { id: "ig-export", category: "instagram", icon: "glyph:package", from: "#833AB4", to: "#C13584", platforms: ALL_OS, status: "ready", runner: "ig-export", keywords: ["export", "baixe suas informacoes", "baixe suas informações", "download your information", "zip", "json", "offline", "pedidos pendentes", "pending requests", "bloqueados", "blocked", "melhores amigos"] },
  { id: "ig-analytics", category: "instagram", icon: "glyph:chart-line-up", from: "#F77737", to: "#FCAF45", platforms: ALL_OS, status: "ready", runner: "ig-analytics", keywords: ["analytics", "analise", "análise", "engajamento", "engagement", "metricas", "métricas", "melhor horario", "melhor horário", "best time", "hashtags", "top posts", "estatisticas", "estatísticas"] },
  { id: "ig-benchmark", category: "instagram", icon: "glyph:chart-bar", from: "#FCAF45", to: "#F56040", platforms: ALL_OS, status: "ready", runner: "ig-benchmark", keywords: ["benchmark", "comparar", "compare", "concorrentes", "competitors", "lado a lado", "perfis"] },
  { id: "ig-hashtag", category: "instagram", icon: "glyph:hash", from: "#E1306C", to: "#F56040", platforms: ALL_OS, status: "ready", runner: "ig-hashtag", keywords: ["hashtag", "tag", "explorar", "explore", "recentes", "populares", "top", "relacionadas", "related"] },
  { id: "ig-comments", category: "instagram", icon: "glyph:chat-text", from: "#5851DB", to: "#833AB4", platforms: ALL_OS, status: "ready", runner: "ig-comments", keywords: ["comentarios", "comentários", "comments", "exportar", "export", "csv", "quem comentou"] },
  { id: "ig-likers", category: "instagram", icon: "glyph:users-three", from: "#C13584", to: "#E1306C", platforms: ALL_OS, status: "ready", runner: "ig-likers", keywords: ["curtidas", "likes", "likers", "quem curtiu", "exportar", "csv"] },
  { id: "ig-giveaway", category: "instagram", icon: "glyph:sparkle", from: "#FD1D1D", to: "#F56040", platforms: ALL_OS, status: "ready", runner: "ig-giveaway", keywords: ["sorteio", "giveaway", "sortear", "comment picker", "ganhador", "winner", "promocao", "promoção"] },
  { id: "ig-publish", category: "instagram", icon: "glyph:paper-plane-tilt", from: "#405DE6", to: "#E1306C", platforms: ALL_OS, status: "beta", runner: "ig-publish", keywords: ["publicar", "postar", "publish", "post", "upload", "reel", "story", "carrossel", "carousel", "graph api", "api oficial"] },
  { id: "ig-schedule", category: "instagram", icon: "glyph:timer", from: "#5B51D8", to: "#C13584", platforms: ALL_OS, status: "beta", runner: "ig-schedule", keywords: ["agendar", "agendamento", "schedule", "scheduler", "postar automaticamente", "auto post", "fila", "queue"] },

  // ── Pinterest ─────────────────────────────────────────────────────
  { id: "pin-download", category: "pinterest", icon: "glyph:download-simple", ...P.red, platforms: ALL_OS, status: "ready", runner: "pin-download", keywords: ["baixar", "download", "original", "hd", "qualidade", "quality", "video", "vídeo", "gif", "carrossel", "carousel", "story", "webp", "png", "pin.it"] },
  { id: "pin-board", category: "pinterest", icon: "glyph:cloud-arrow-down", ...P.orange, platforms: ALL_OS, status: "ready", runner: "pin-board", keywords: ["backup", "board", "secao", "seção", "section", "exportar", "export", "sincronizar", "sync", "csv", "json", "cookies", "secreto", "secret", "privado", "private", "em massa", "bulk", "todos os pins"] },
  { id: "pin-profile", category: "pinterest", icon: "glyph:users-three", ...P.pink, platforms: ALL_OS, status: "ready", runner: "pin-profile", keywords: ["perfil", "profile", "usuario", "usuário", "user", "todos os boards", "criados", "created", "backup", "conta", "account"] },
  { id: "pin-search", category: "pinterest", icon: "glyph:magnifying-glass", ...P.blue, platforms: ALL_OS, status: "ready", runner: "pin-search", keywords: ["buscar", "search", "pesquisar", "sem ia", "ai", "ia", "slop", "anuncio", "anúncio", "ads", "promovido", "promoted", "filtro", "filter", "videos", "vídeos"] },
  { id: "pin-related", category: "pinterest", icon: "glyph:sparkle", ...P.purple, platforms: ALL_OS, status: "ready", runner: "pin-related", keywords: ["parecidos", "similar", "similares", "more like this", "mais como este", "relacionados", "related", "visual"] },
  { id: "pin-source", category: "pinterest", icon: "glyph:link", ...P.teal, platforms: ALL_OS, status: "ready", runner: "pin-source", keywords: ["fonte", "source", "origem", "artista", "artist", "credito", "crédito", "busca reversa", "reverse image search", "google lens", "tineye", "yandex", "link quebrado", "dead link", "wayback", "pin.it", "expandir", "expand", "estatisticas", "stats"] },
  { id: "pin-dupes", category: "pinterest", icon: "glyph:copy", ...P.brown, platforms: ALL_OS, status: "ready", runner: "pin-dupes", keywords: ["duplicados", "duplicates", "duplicate", "repetidos", "iguais", "parecidos", "limpar board", "clean", "unsave", "desfazer save", "dhash"] },
  { id: "pin-palette", category: "pinterest", icon: "glyph:swatches", ...P.yellow, platforms: ALL_OS, status: "ready", runner: "pin-palette", keywords: ["paleta", "palette", "cores", "colors", "cor", "color", "hex", "css", "moodboard", "dominante", "dominant"] },
  { id: "pin-export", category: "pinterest", icon: "glyph:file-arrow-down", ...P.indigo, platforms: ALL_OS, status: "ready", runner: "pin-export", keywords: ["exportar", "export", "galeria", "gallery", "html", "offline", "pdf", "csv", "json", "moodboard", "imprimir", "print", "planilha", "spreadsheet", "eagle", "notion"] },
  { id: "pin-keywords", category: "pinterest", icon: "glyph:hash", ...P.green, platforms: ALL_OS, status: "ready", runner: "pin-keywords", keywords: ["palavras-chave", "keywords", "keyword", "seo", "sugestoes", "sugestões", "suggestions", "typeahead", "hashtags", "tendencias", "tendências", "trends", "criador", "creator"] },

  // ── Spotify ──────────────────────────────────────────────────────────
  { id: "spotify-spicetify", category: "spotify", icon: "glyph:palette", ...P.green, platforms: ALL_OS, status: "beta", runner: "spicetify", keywords: ["spicetify", "tema", "theme", "cor", "color", "css", "personalizar", "customize", "aparencia", "aparência"] },
  { id: "spotify-extensions", category: "spotify", icon: "glyph:puzzle-piece", ...P.teal, platforms: ALL_OS, status: "beta", runner: "spicetify", keywords: ["spicetify", "extensao", "extensão", "extension", "marketplace", "custom app", "plugin"] },

  // ── Voz e legendas ──────────────────────────────────────────────────
  { id: "speech-transcribe", category: "speech", icon: "glyph:microphone", ...P.pink, platforms: ALL_OS, status: "ready", runner: "whisper", keywords: ["transcrever", "transcribe", "transcricao", "transcrição", "whisper", "whisper.cpp", "legenda", "subtitle", "srt", "audio para texto", "speech to text", "offline", "local"] },
  { id: "speech-tts", category: "speech", icon: "glyph:speaker-high", ...P.orange, platforms: ALL_OS, status: "ready", runner: "tts", keywords: ["tts", "texto para voz", "text to speech", "narracao", "narração", "voz", "voice", "edge", "locucao", "locução", "mp3", "gratis", "grátis"] },
  { id: "speech-translate", category: "speech", icon: "glyph:translate", ...P.blue, platforms: ALL_OS, status: "ready", runner: "srt-translate", keywords: ["traduzir legenda", "translate subtitle", "srt", "libretranslate", "ia", "llm", "bilingue", "bilíngue", "traducao", "tradução"] },
  { id: "speech-dub", category: "speech", icon: "glyph:user-sound", ...P.purple, platforms: ALL_OS, status: "beta", runner: "dub", keywords: ["dublar", "dublagem", "dub", "dubbing", "voz", "voice over", "narrar video", "legenda para voz"] },
  // VoiceStudio-style features that do not exist yet (see docs/tools-reference.md, lote 2)
  { id: "speech-clone", category: "speech", icon: "glyph:record", ...P.pink, platforms: ALL_OS, status: "beta", runner: "voicestudio", keywords: ["clonar voz", "clone", "voice cloning", "zero-shot", "amostra", "sample", "elevenlabs", "voicestudio"] },
  { id: "speech-design", category: "speech", icon: "glyph:sparkle", ...P.purple, platforms: ALL_OS, status: "beta", runner: "voicestudio", keywords: ["design de voz", "voice design", "criar voz", "sotaque", "accent", "locutor", "narrador", "voicestudio"] },
  { id: "speech-isolate", category: "speech", icon: "glyph:headphones", ...P.teal, platforms: ALL_OS, status: "beta", runner: "voicestudio", keywords: ["isolar voz", "vocal isolation", "separar", "demucs", "instrumental", "acapella", "karaoke", "remover fundo"] },
  { id: "speech-dictation", category: "speech", icon: "glyph:text-aa", ...P.orange, platforms: ALL_OS, status: "beta", runner: "dictation", keywords: ["ditado", "dictation", "ditar", "speech to text", "atalho", "hotkey"] },

  // ── PDF ──────────────────────────────────────────────────────────────
  { id: "pdf-merge", category: "pdf", icon: "glyph:list-checks", ...P.red, platforms: ALL_OS, status: "ready", runner: "pdf", keywords: ["juntar", "merge", "unir", "combinar", "combine", "join"] },
  { id: "pdf-split", category: "pdf", icon: "glyph:scissors", ...P.orange, platforms: ALL_OS, status: "ready", runner: "pdf", keywords: ["dividir", "split", "separar", "paginas", "páginas", "pages", "extrair", "extract"] },
  { id: "pdf-compress", category: "pdf", icon: "glyph:package", ...P.blue, platforms: ALL_OS, status: "ready", runner: "pdf", keywords: ["comprimir", "compress", "reduzir", "tamanho", "size", "otimizar", "optimize"] },
  { id: "pdf-convert", category: "pdf", icon: "glyph:arrows-clockwise", ...P.purple, platforms: ALL_OS, status: "ready", runner: "pdf", keywords: ["converter", "convert", "imagem", "image", "word", "docx", "jpg", "png", "html"] },
  { id: "pdf-ocr", category: "pdf", icon: "glyph:text-aa", ...P.teal, platforms: ALL_OS, status: "beta", runner: "pdf", keywords: ["ocr", "texto", "text", "reconhecer", "recognize", "escaneado", "scanned", "pesquisavel", "pesquisável", "searchable"] },
  { id: "pdf-sanitize", category: "pdf", icon: "glyph:shield-check", ...P.green, platforms: ALL_OS, status: "ready", runner: "pdf", keywords: ["sanitizar", "sanitize", "seguro", "safe", "dangerzone", "malware", "javascript", "limpar pdf"] },

  // ── Documentos ──────────────────────────────────────────────────────
  { id: "doc-slideshare", category: "documents", icon: "glyph:presentation-chart", ...P.orange, platforms: ALL_OS, status: "ready", runner: "slideshare", keywords: ["slideshare", "slides", "apresentacao", "apresentação", "pdf", "baixar slides"] },
  { id: "doc-google", category: "documents", icon: "glyph:file-doc", ...P.blue, platforms: ALL_OS, status: "ready", runner: "gdocs", keywords: ["google docs", "google slides", "google sheets", "planilha", "documento", "apresentacao", "apresentação", "exportar", "export", "docx", "pptx", "xlsx"] },
  { id: "doc-calameo", category: "documents", icon: "glyph:book-open-text", ...P.teal, platforms: ALL_OS, status: "beta", runner: "calameo", keywords: ["calameo", "revista", "magazine", "svg", "paginas", "páginas", "flipbook"] },
  { id: "doc-gallery", category: "documents", icon: "glyph:images", ...P.purple, platforms: ALL_OS, status: "ready", runner: "gallery", keywords: ["gallery-dl", "galeria", "gallery", "imagens", "images", "pinterest", "instagram", "artstation", "behance", "deviantart", "reddit", "twitter", "x", "pixiv", "em massa", "bulk"] },
  { id: "doc-scribd", category: "documents", icon: "glyph:book-open-text", ...P.red, platforms: ALL_OS, status: "soon", keywords: ["scribd", "everand", "ebook", "livro", "book", "audiobook"] },

  // ── Imagens ─────────────────────────────────────────────────────────
  { id: "img-upscale", category: "images", icon: "glyph:arrows-out", ...P.purple, platforms: ALL_OS, status: "beta", runner: "upscale", keywords: ["upscale", "upscayl", "aumentar resolucao", "aumentar resolução", "real-esrgan", "esrgan", "ia", "ai", "melhorar imagem", "4x", "gpu", "vulkan"] },
  { id: "img-resize", category: "images", icon: "glyph:frame-corners", ...P.yellow, platforms: ALL_OS, status: "ready", runner: "resize", keywords: ["redimensionar", "resize", "reduzir", "shrink", "lote", "batch", "largura", "width", "porcentagem", "percent", "converter imagem", "jpg", "png", "webp"] },
  { id: "img-ocr", category: "images", icon: "glyph:text-aa", ...P.teal, platforms: ALL_OS, status: "beta", runner: "ocr", keywords: ["ocr", "texto da imagem", "text extractor", "copiar texto", "powertoys", "tesseract"] },

  // ── Arquivos ────────────────────────────────────────────────────────
  { id: "files-dupes", category: "files", icon: "glyph:copy", ...P.brown, platforms: ALL_OS, status: "ready", runner: "dupes", keywords: ["duplicados", "duplicates", "duplicate", "repetidos", "czkawka", "espaco", "espaço", "space", "limpar", "clean", "hash"] },
  { id: "files-rename", category: "files", icon: "glyph:pencil-simple", ...P.indigo, platforms: ALL_OS, status: "ready", runner: "rename", keywords: ["renomear", "rename", "em massa", "bulk", "regex", "powerrename", "numerar", "contador", "counter", "aulas"] },
  { id: "files-search", category: "files", icon: "glyph:list-magnifying-glass", ...P.blue, platforms: ALL_OS, status: "ready", runner: "file-search", keywords: ["buscar", "search", "procurar", "find", "everything", "spotlight", "mdfind", "fd", "arquivo", "file", "instantaneo", "instantâneo"] },
  { id: "files-awake", category: "files", icon: "glyph:coffee", ...P.orange, platforms: ALL_OS, status: "ready", runner: "awake", keywords: ["manter acordado", "keep awake", "nao dormir", "não dormir", "suspender", "sleep", "caffeine", "powertoys awake", "tela ligada"] },

  // ── Downloads ───────────────────────────────────────────────────────
  { id: "dl-aria2", category: "downloads", icon: "glyph:rocket-launch", ...P.red, platforms: ALL_OS, status: "ready", runner: "aria2", keywords: ["aria2", "acelerado", "accelerated", "multi conexao", "multi conexão", "conexoes", "connections", "arquivo grande", "large file", "zip", "iso", "resume", "retomar"] },
  { id: "dl-manifest", category: "downloads", icon: "glyph:broadcast", ...P.indigo, platforms: ALL_OS, status: "ready", runner: "manifest", keywords: ["m3u8", "mpd", "hls", "dash", "manifesto", "manifest", "stream", "ffmpeg", "player", "aula", "referer", "cookie"] },

  // ── Celular ─────────────────────────────────────────────────────────
  { id: "phone-send", category: "phone", icon: "glyph:device-mobile", ...P.green, platforms: ALL_OS, status: "ready", runner: "kdeconnect", keywords: ["kde connect", "kdeconnect", "celular", "phone", "enviar arquivo", "send file", "compartilhar", "share", "android", "iphone", "link", "clipboard"] },

  // ── Sistema ──────────────────────────────────────────────────────────
  { id: "sys-clean", category: "system", icon: "glyph:broom", ...P.teal, platforms: ALL_OS, status: "ready", runner: "sysclean", keywords: ["limpar", "limpeza", "clean", "cleaner", "cache", "temporarios", "temporários", "temp", "logs", "lixo", "junk", "ccleaner", "navegador", "browser"] },
  { id: "sys-disk", category: "system", icon: "glyph:hard-drives", ...P.blue, platforms: ALL_OS, status: "ready", runner: "disk", keywords: ["disco", "disk", "espaco", "espaço", "space", "treemap", "analisador", "analyzer", "armazenamento", "storage", "pastas grandes"] },
  { id: "sys-startup", category: "system", icon: "glyph:lightning", ...P.yellow, platforms: ALL_OS, status: "beta", runner: "startup", keywords: ["inicializacao", "inicialização", "startup", "boot", "login items", "iniciar com o sistema", "autostart"] },
  { id: "sys-uninstall", category: "system", icon: "glyph:trash-simple", ...P.gray, platforms: ALL_OS, status: "beta", runner: "uninstall", keywords: ["desinstalar", "uninstall", "remover programa", "remove app", "sobras", "leftovers", "aplicativos", "apps"] },
  { id: "sys-debloat", category: "system", icon: "glyph:app-window", ...P.ink, platforms: ["windows"], status: "beta", runner: "debloat", keywords: ["debloat", "bloatware", "remover apps windows", "windows", "xbox", "cortana", "onedrive", "edge", "pre-instalado", "pré-instalado", "preinstalled"] },
  { id: "sys-registry", category: "system", icon: "glyph:wrench", ...P.brown, platforms: ["windows"], status: "beta", runner: "winreg", keywords: ["registro", "registry", "regedit", "chaves orfas", "chaves órfãs", "orphaned", "windows"] },
  { id: "sys-privacy", category: "system", icon: "glyph:eye-slash", ...P.green, platforms: ["windows"], status: "beta", runner: "win-tweaks", keywords: ["privacidade", "privacy", "telemetria", "telemetry", "rastreamento", "tracking", "anuncios", "anúncios", "ads", "windows", "sophia", "ajustes", "tweaks", "explorer", "taskbar", "copilot", "recall"] },
  { id: "sys-harden", category: "system", icon: "glyph:shield-check", ...P.red, platforms: ["windows"], status: "beta", runner: "win-harden", keywords: ["endurecer", "harden", "hardentools", "seguranca", "segurança", "security", "macros", "office", "autorun", "uac", "defender", "windows"] },
  { id: "sys-updater", category: "system", icon: "glyph:arrows-clockwise", ...P.indigo, platforms: ["windows"], status: "beta", runner: "updater", keywords: ["atualizar programas", "update apps", "winget", "chocolatey", "scoop", "atualizador", "updater", "windows"] },

  // ── Automação ────────────────────────────────────────────────────────
  { id: "auto-clicker", category: "automation", icon: "glyph:cursor-click", ...P.orange, platforms: ALL_OS, status: "beta", runner: "autoclick", keywords: ["autoclicker", "auto clicker", "auto click", "cliques", "clicks", "cps", "mouse", "hotkey", "atalho", "windows"] },

  // ── IA ───────────────────────────────────────────────────────────────
  { id: "ai-keys", category: "ai", icon: "glyph:key", ...P.indigo, platforms: ALL_OS, status: "ready", runner: "keys", keywords: ["chaves", "keys", "api key", "token", "saldo", "balance", "uso", "usage", "relay", "openai", "anthropic", "claude", "gemini"] },
  { id: "ai-prices", category: "ai", icon: "glyph:coins", ...P.teal, platforms: ALL_OS, status: "ready", runner: "pricing", keywords: ["preco", "preço", "price", "prices", "comparar", "compare", "custo", "cost", "tokens", "modelo", "model", "litellm", "contexto", "context"] },
  { id: "ai-usage", category: "ai", icon: "glyph:chart-bar", ...P.indigo, platforms: ALL_OS, status: "ready", runner: "usage", keywords: ["uso", "usage", "gasto", "spent", "custo", "cost", "tokens", "ledger", "quanto gastei", "ccusage", "relatorio", "relatório"] },
  { id: "ai-ollama", category: "ai", icon: "glyph:cube", ...P.gray, platforms: ALL_OS, status: "ready", runner: "ollama", keywords: ["ollama", "modelo local", "local model", "llm", "offline", "baixar modelo", "pull", "llama", "qwen", "gemma", "gguf"] },
  { id: "ai-mcp", category: "ai", icon: "glyph:plug", ...P.purple, platforms: ALL_OS, status: "beta", runner: "mcp", keywords: ["mcp", "agente", "agent", "claude", "goose", "cursor", "servidor mcp", "automatizar", "tools"] },
  { id: "ai-humanize", category: "ai", icon: "glyph:text-t", ...P.green, platforms: ALL_OS, status: "beta", runner: "humanize", keywords: ["humanizar", "humanize", "humanizer", "texto", "text", "reescrever", "rewrite", "cara de ia", "ai writing", "wikipedia", "estilo", "style"] },
];

export function categoryById(id: string): ToolCategory | undefined {
  return CATEGORIES.find((c) => c.id === id);
}

export function toolById(id: string): ToolEntry | undefined {
  return TOOLS.find((t) => t.id === id);
}

export function toolsOf(categoryId: string): ToolEntry[] {
  return TOOLS.filter((t) => t.category === categoryId);
}

/** Rota que o tile abre. */
export function toolHref(tool: ToolEntry): string {
  return tool.href ?? `/tools/${tool.category}/${tool.id}`;
}

export function isCrossPlatform(tool: ToolEntry): boolean {
  return ALL_OS.every((os) => tool.platforms.includes(os));
}

export function matchesPlatform(tool: ToolEntry, filter: PlatformFilter): boolean {
  if (filter === "all") return true;
  if (filter === "cross") return isCrossPlatform(tool);
  return tool.platforms.includes(filter);
}

/** Minúsculas e sem acento, para "vídeo" e "video" acharem a mesma coisa. */
export function normalize(s: string): string {
  return s
    .toLowerCase()
    .normalize("NFD")
    .replace(/[\u0300-\u036f]/g, "")
    .trim();
}

export type Translate = (key: string) => string;

export type IndexedTool = {
  tool: ToolEntry;
  name: string;
  desc: string;
  /** Texto normalizado onde a busca procura. */
  hay: string;
};

/**
 * Monta o índice com os textos traduzidos. O nome da categoria entra no
 * texto de cada ferramenta, então "instagram" acha tudo de Instagram mesmo
 * que a ferramenta não repita a palavra.
 */
export function buildIndex(t: Translate): IndexedTool[] {
  return TOOLS.map((tool) => {
    const cat = categoryById(tool.category);
    const name = t(`tools.catalog.${tool.id}.name`);
    const desc = t(`tools.catalog.${tool.id}.desc`);
    const catName = cat ? t(`tools.categories.${cat.id}.name`) : "";
    const parts = [name, desc, catName, ...tool.keywords, ...(cat?.keywords ?? []), ...tool.platforms];
    return { tool, name, desc, hay: normalize(parts.join(" ")) };
  });
}

/** Todas as palavras da busca precisam aparecer, em qualquer ordem. */
export function matchesQuery(entry: IndexedTool, query: string): boolean {
  const tokens = normalize(query).split(/\s+/).filter(Boolean);
  if (tokens.length === 0) return true;
  return tokens.every((tok) => entry.hay.includes(tok));
}

export type CategoryGroup = { category: ToolCategory; tools: IndexedTool[] };

/** Resultado da busca agrupado por categoria, na ordem do catálogo. */
export function search(index: IndexedTool[], query: string, filter: PlatformFilter, categoryId?: string): CategoryGroup[] {
  const groups: CategoryGroup[] = [];
  for (const category of [...CATEGORIES].sort((a, b) => a.order - b.order)) {
    if (categoryId && category.id !== categoryId) continue;
    const tools = index.filter(
      (e) => e.tool.category === category.id && matchesPlatform(e.tool, filter) && matchesQuery(e, query),
    );
    if (tools.length > 0) groups.push({ category, tools });
  }
  return groups;
}
