<script lang="ts">
  import { goto } from "$app/navigation";
  import { t } from "$lib/i18n";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import { searchPowers } from "$lib/superpowers/search";
  import NavIcon from "$components/shell/NavIcon.svelte";

  type Power = {
    id: string;
    icon: string;
    title: string;
    descKey: string;
    keywords: string;
    href: string;
    enabled: boolean;
  };

  let powers = $derived<Power[]>([
    {
      id: "music",
      icon: "music",
      title: $t("music.title") as string,
      descKey: "superpowers.music_desc",
      keywords: "musica player library biblioteca radio lyrics letras playlist equalizer equalizador karaoke karaokê separar voz vocal instrumental acapella a capella playback stems remover voz tirar voz cantar junto vocal remover voice remover sing along",
      href: "/music",
      enabled: getSettings()?.music?.enabled ?? true,
    },
    {
      id: "video",
      icon: "video",
      title: $t("video.title") as string,
      descKey: "superpowers.video_desc",
      keywords: "filmes movies series shows anime cursos courses aulas lessons iptv jellyfin tv player",
      href: "/video",
      enabled: getSettings()?.video?.enabled ?? true,
    },
    {
      id: "global",
      icon: "global",
      title: $t("global_dl.title") as string,
      descKey: "superpowers.global_desc",
      keywords: "navegador browser sites conectores connectors baixar download",
      href: "/global",
      enabled: true,
    },
    {
      id: "tube",
      icon: "tube",
      title: $t("tube.title") as string,
      descKey: "superpowers.tube_desc",
      keywords: "youtube music sponsorblock shorts inscricoes subscriptions",
      href: "/youtube",
      enabled: getSettings()?.tube?.enabled ?? true,
    },
    {
      id: "read",
      icon: "read",
      title: $t("read.title") as string,
      descKey: "superpowers.read_desc",
      keywords: "livros books ebook epub quadrinhos comics manga kindle leitor reader",
      href: "/read",
      enabled: getSettings()?.read?.enabled ?? true,
    },
    {
      id: "league",
      icon: "league",
      title: "League of Legends",
      descKey: "superpowers.league_desc",
      keywords: "lol partida match scout",
      href: "/league",
      enabled: getSettings()?.league?.enabled ?? true,
    },
    {
      id: "telegram",
      icon: "telegram",
      title: "Telegram",
      descKey: "superpowers.telegram_desc",
      keywords: "chats canais channels proxy",
      href: "/telegram",
      enabled: getSettings()?.telegram?.enabled ?? true,
    },
    {
      id: "washmybelly",
      icon: "washmybelly",
      title: "washmybelly",
      descKey: "superpowers.washmybelly_desc",
      keywords: "limpeza cleaner apps login privacidade privacy bateria battery fps",
      href: "/washmybelly",
      enabled: getSettings()?.washmybelly?.enabled ?? true,
    },
    {
      id: "converter",
      icon: "converter",
      title: $t("converter.title") as string,
      descKey: "superpowers.converter_desc",
      keywords: "converter conversor convert converta formato format json yaml csv markdown pdf png jpg webp mp3 mp4 gif docx heic heif jxl jpeg xl psd psb photoshop camadas layers raw cr2 cr3 nef arw dng raf orf rw2 svg vetorizar vectorize trace dxf dwg cad autocad planta",
      href: "/converter",
      enabled: true,
    },
    {
      id: "pdf",
      icon: "pdf",
      title: $t("pdf.title") as string,
      descKey: "superpowers.pdf_desc",
      keywords: "pdf juntar merge combinar unir separar split dividir extrair extract páginas pages girar rotate reordenar reorder apagar delete página em branco blank comprimir compress reduzir tamanho shrink imagens images fotos jpg png scan escanear para pdf to pdf",
      href: "/pdf",
      enabled: true,
    },
    {
      id: "photos",
      icon: "photos",
      title: $t("photos.title") as string,
      descKey: "superpowers.photos_desc",
      keywords: "fotos photos galeria gallery imagens images pictures biblioteca library duplicadas duplicatas duplicates parecidas similar limpar cleanup lixeira trash apagar delete mover move editar edit ajustes adjust exposição exposure contraste contrast saturação saturation temperatura warmth recortar crop girar rotate exportar export lote batch heic raw cr2 cr3 nef arw dng psd jxl jpg png webp tiff miniaturas thumbnails",
      href: "/photos",
      enabled: true,
    },
    {
      id: "cutout",
      icon: "cutout",
      title: $t("cutout.title") as string,
      descKey: "superpowers.cutout_desc",
      keywords: "recortar recorte cutout remover fundo remove background bg sem fundo transparente transparent png sticker figurinha recortar pessoa produto foto frame video",
      href: "/cutout",
      enabled: true,
    },
    {
      id: "palette",
      icon: "palette",
      title: $t("palette.title") as string,
      descKey: "superpowers.palette_desc",
      keywords: "palette paleta cores cor color colors oklch oklab contraste contrast wcag apca gradiente gradient degrade escala scale tokens tailwind css",
      href: "/palette",
      enabled: true,
    },
    {
      id: "fonts",
      icon: "fonts",
      title: $t("fonts.title") as string,
      descKey: "superpowers.fonts_desc",
      keywords: "type tipografia typography fonte fontes font fonts inter geist jetbrains mono escala scale instalar install fontsource ofl",
      href: "/fonts",
      enabled: true,
    },
    {
      id: "icons",
      icon: "icons",
      title: $t("icons.title") as string,
      descKey: "superpowers.icons_desc",
      keywords: "icons icones ícones icon svg lucide phosphor tabler heroicons iconify material sprite png",
      href: "/icons",
      enabled: true,
    },
    {
      id: "motion",
      icon: "motion",
      title: $t("motion.title") as string,
      descKey: "superpowers.motion_desc",
      keywords: "motion animação animacao animation easing curva curve bezier spring mola timeline keyframes lottie dotlottie css gsap",
      href: "/motion",
      enabled: true,
    },
    {
      id: "board",
      icon: "board",
      title: $t("board.title") as string,
      descKey: "superpowers.board_desc",
      keywords: "board quadro quadro-branco lousa whiteboard desenho drawing diagrama diagram rascunho sketch fluxograma flowchart mapa mental mind map excalidraw wireframe",
      href: "/board",
      enabled: true,
    },
    {
      id: "components",
      icon: "components",
      title: $t("components.title") as string,
      descKey: "superpowers.components_desc",
      keywords: "components componentes shadcn registry registro ui blocks blocos magic ui react tailwind design tokens dtcg tokens studio figma penpot",
      href: "/components",
      enabled: true,
    },
    {
      id: "capture",
      icon: "capture",
      title: $t("capture.title") as string,
      descKey: "superpowers.capture_desc",
      keywords: "capture captura gravar tela gravação de tela screen recorder screen recording record gravador cortar vídeo cortador cut video trim lossless cut sem perda keyframe segmentos segments snapshot microfone microphone ffmpeg obs losslesscut",
      href: "/capture",
      enabled: true,
    },
    {
      id: "studio",
      icon: "studio",
      title: $t("studio.title") as string,
      descKey: "superpowers.studio_desc",
      keywords: "studio estúdio speech to text transcribe transcrever transcrição transcription tools ferramentas legendas subtitles captions legenda automática auto captions whisper srt vtt karaoke vertical 9:16 reels shorts tiktok reframe cartela title card vídeo video",
      href: "/studio",
      enabled: true,
    },
    {
      id: "dictation",
      icon: "dictation",
      title: $t("dictation.title") as string,
      descKey: "superpowers.dictation_desc",
      keywords: "dictation ditado ditar voz voice typing digitar por voz speech to text fala transcrever whisper atalho push to talk microfone microphone handy superwhisper",
      href: "/dictation",
      enabled: true,
    },
    {
      id: "whatsapp",
      icon: "whatsapp",
      title: $t("whatsapp.title") as string,
      descKey: "superpowers.whatsapp_desc",
      keywords: "whatsapp zap zapzap whats mensagens conversas grupos baixar mídias fotos vídeos áudios documentos figurinhas exportar conversa backup qr code messages chats groups download media photos videos audio voice notes documents stickers export chat backup",
      href: "/whatsapp",
      enabled: true,
    },
    {
      id: "discord",
      icon: "discord",
      title: $t("discord.title") as string,
      descKey: "superpowers.discord_desc",
      keywords: "discord servidores servers canais channels mensagens messages dm conversas chats responder reply exportar export historico histórico history baixar download midia mídia media anexos attachments",
      href: "/discord",
      enabled: true,
    },
    {
      id: "x",
      icon: "x",
      title: $t("social.x.title") as string,
      descKey: "superpowers.x_desc",
      keywords: "x twitter tweets tweet posts timeline feed perfil profile seguir follow curtir like repost retweet responder reply menções mentions bookmarks salvos busca search agente agents mcp",
      href: "/x",
      enabled: true,
    },
    {
      id: "instagram",
      icon: "instagram",
      title: $t("social.instagram.title") as string,
      descKey: "superpowers.instagram_desc",
      keywords: "instagram insta ig feed reels stories posts perfil profile seguir follow curtir like comentar comment direct dm mensagens messages salvos saved busca search hashtags agente agents mcp",
      href: "/instagram",
      enabled: true,
    },
    {
      id: "tiktok",
      icon: "tiktok",
      title: $t("social.tiktok.title") as string,
      descKey: "superpowers.tiktok_desc",
      keywords: "tiktok tik tok videos vídeos for you fyp perfil profile seguir follow curtir like comentar comment busca search agente agents mcp",
      href: "/tiktok",
      enabled: true,
    },
  ]);

  let query = $state("");

  let matches = $derived(
    searchPowers(
      powers.map((power) => ({ ...power, description: $t(power.descKey) as string })),
      query,
    ),
  );

  function settingsHref(power: Power): string {
    return `/settings?tab=integrations&section=superpowers&setting=settings.integrations.superpowers.${power.id}`;
  }

  function onSearchKey(event: KeyboardEvent) {
    if (event.key === "Escape" && query) {
      event.preventDefault();
      query = "";
    } else if (event.key === "Enter" && !event.isComposing && query.trim() && matches[0]) {
      event.preventDefault();
      const first = matches[0];
      void goto(first.enabled ? first.href : settingsHref(first));
    }
  }
</script>

<section class="powers">
  <header class="powers-head">
    <NavIcon icon="superpowers" size={56} />
    <div>
      <h1>{$t("nav.superpowers")}</h1>
      <p class="powers-subtitle">{$t("superpowers.subtitle")}</p>
    </div>
  </header>

  <input
    class="input input-search powers-search"
    type="search"
    placeholder={$t("superpowers.search") as string}
    aria-label={$t("superpowers.search") as string}
    bind:value={query}
    onkeydown={onSearchKey}
  />

  <div class="powers-grid">
    {#each matches as power (power.id)}
      <article class="power-card" class:off={!power.enabled}>
        <NavIcon icon={power.icon} size={64} />
        <div class="power-copy">
          <h2>{power.title}</h2>
          <p>{power.description}</p>
        </div>
        {#if power.enabled}
          <a class="btn btn-primary power-action" href={power.href}>{$t("superpowers.open")}</a>
        {:else}
          <p class="power-off">
            <span class="power-off-badge">{$t("superpowers.disabled")}</span>
            {$t("superpowers.disabled_hint")}
          </p>
          <a class="btn btn-secondary power-action" href={settingsHref(power)}>{$t("superpowers.open_settings")}</a>
        {/if}
      </article>
    {:else}
      <p class="powers-empty" role="status">{$t("superpowers.no_results")}</p>
    {/each}
  </div>
</section>

<style>
  .powers {
    display: flex;
    flex-direction: column;
    gap: var(--space-6);
    width: 100%;
    max-width: 1040px;
    margin-inline: auto;
    padding: var(--space-4) var(--space-5) var(--space-9);
  }

  .powers-head {
    display: flex;
    align-items: center;
    gap: var(--space-4);
  }

  .powers-head h1 {
    margin: 0 0 var(--space-1);
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
    color: var(--text);
  }

  .powers-subtitle {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-base);
  }

  .powers-search {
    width: 100%;
    max-width: 360px;
  }

  .powers-empty {
    grid-column: 1 / -1;
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-base);
  }

  .powers-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: var(--space-4);
  }

  .power-card {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-3);
    padding: var(--space-5);
    border-radius: 14px;
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .power-card.off :global(.nav-art) {
    filter: grayscale(1);
    opacity: 0.55;
  }

  .power-copy h2 {
    margin: 0 0 var(--space-1);
    font-size: var(--text-lg);
    font-weight: 650;
    color: var(--text);
  }

  .power-copy p,
  .power-off {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--text-sm);
    line-height: 1.45;
  }

  .power-off-badge {
    display: inline-block;
    margin-inline-end: 6px;
    padding: 1px 7px;
    border-radius: var(--radius-full);
    background: var(--fill-2);
    color: var(--text-muted);
    font-size: var(--text-xs);
    font-weight: 600;
  }

  .power-action {
    margin-top: auto;
  }
</style>
