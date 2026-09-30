#!/usr/bin/env node
// Sobe o app com `OMNIGET_WORLD_BENCH=<cenario>`, captura a linha
// `[world-bench] {json}` e compara com `scripts/bench/world-baseline.json`.
//
// Irmao de `scripts/smoke-test.mjs`. A diferenca e a pergunta: o smoke pergunta
// "abre?", este pergunta "abre, desenha, e quanto custa?". O app faz o trabalho
// (hook em `src-tauri/src/world_bench.rs`, runner em
// `src/lib/world/bench/runner.ts`); este script so orquestra, le uma linha e
// reprova.
//
// Uso:
//   node scripts/world-bench.mjs --scenario llvmpipe --out docs/bench/out.json
//   node scripts/world-bench.mjs --help
//
// Exit codes: 0 ok · 1 regressao ou bench falhou · 2 erro de uso.

import { spawn } from "node:child_process";
import { createServer } from "node:http";
import {
  createReadStream,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { createConnection } from "node:net";
import { dirname, extname, join, normalize, resolve } from "node:path";

/**
 * Receitas de ambiente do estudo 73 §B. Sao variaveis do Mesa e do WebKitGTK:
 * no macOS nao fazem nada, e por isso o cenario `native` nao define nenhuma.
 * @type {Record<string, { env: Record<string, string>, about: string }>}
 */
export const SCENARIOS = {
  llvmpipe: {
    env: { LIBGL_ALWAYS_SOFTWARE: "1", GALLIUM_DRIVER: "llvmpipe", LP_NUM_THREADS: "2" },
    about: "WebGL sobre llvmpipe com compositing acelerado — o 'sucesso silencioso'",
  },
  nocompositing: {
    env: { LIBGL_ALWAYS_SOFTWARE: "1", WEBKIT_DISABLE_COMPOSITING_MODE: "1" },
    about: "idem, sem compositing acelerado — receita NVIDIA/VM/Wayland",
  },
  shm: {
    env: { LIBGL_ALWAYS_SOFTWARE: "1", WEBKIT_DMABUF_RENDERER_FORCE_SHM: "1" },
    about: "DMABUF so por memoria compartilhada (fallback se o WebGL2 nao criar)",
  },
  nodmabuf: {
    env: { LIBGL_ALWAYS_SOFTWARE: "1", WEBKIT_DISABLE_DMABUF_RENDERER: "1" },
    about: "martelo: sem renderer DMABUF (2.46+ derruba a aceleracao inteira)",
  },
  native: {
    env: {},
    about: "sem receita nenhuma — a maquina como ela e (macOS/Windows)",
  },
};

/**
 * As metricas comparadas com o baseline. `dir` diz o que e piorar.
 * @type {{ path: string, dir: 'lower' | 'higher', label: string }[]}
 */
export const METRICS = [
  { path: "calibrate_median_ms", dir: "lower", label: "calibracao (mediana ms)" },
  { path: "first_frame_ms", dir: "lower", label: "primeiro frame (ms)" },
  { path: "scene.median_frame_ms", dir: "lower", label: "frame mediano (ms)" },
  { path: "scene.p95_frame_ms", dir: "lower", label: "frame p95 (ms)" },
  { path: "scene.cpu_ms", dir: "lower", label: "CPU de render (ms)" },
  { path: "scene.fps", dir: "higher", label: "fps" },
  { path: "rss_extra_mb", dir: "lower", label: "RSS extra (MB)" },
  { path: "ipc.2kb_p50_ms", dir: "lower", label: "IPC 2 KB p50 (ms)" },
  { path: "ipc.50kb_p50_ms", dir: "lower", label: "IPC 50 KB p50 (ms)" },
  { path: "ipc.200kb_p50_ms", dir: "lower", label: "IPC 200 KB p50 (ms)" },
  { path: "ipc.200kb_15hz_cpu_pct", dir: "lower", label: "IPC 200 KB @15 Hz (CPU %)" },
];

/**
 * Metricas da rota `/city` (`--route city`): o 3D no tier 0 e o modo leve,
 * medidos na mesma rodada. `scene.*` repete o modo leve, que e o que uma
 * maquina fraca recebe, e passa pelo mesmo portao de sanidade.
 * @type {{ path: string, dir: 'lower' | 'higher', label: string }[]}
 */
export const CITY_METRICS = [
  { path: "calibrate_median_ms", dir: "lower", label: "calibracao 3D (mediana ms)" },
  { path: "city.three3d.p50_ms", dir: "lower", label: "3D tier 0 p50 (ms)" },
  { path: "city.three3d.p95_ms", dir: "lower", label: "3D tier 0 p95 (ms)" },
  { path: "city.lite.p50_ms", dir: "lower", label: "modo leve p50 (ms)" },
  { path: "city.lite.p95_ms", dir: "lower", label: "modo leve p95 (ms)" },
  { path: "city.lite.first_frame_ms", dir: "lower", label: "modo leve primeiro frame (ms)" },
  { path: "scene.fps", dir: "higher", label: "fps (modo leve)" },
  { path: "rss_extra_mb", dir: "lower", label: "RSS extra (MB)" },
];

/**
 * Rotas que o hook sabe abrir (`OMNIGET_WORLD_BENCH_ROUTE`).
 * @type {Record<string, string>}
 */
export const ROUTES = { world: "/world", city: "/city" };

/**
 * Chave do baseline: `llvmpipe` para `/world`, `city-llvmpipe` para `/city`.
 * @param {string} route @param {string} scenario
 */
export function baselineKey(route, scenario) {
  return route === "city" ? `city-${scenario}` : scenario;
}

/** Regressao maior que isto reprova o job. */
export const DEFAULT_TOLERANCE_PCT = 10;

/**
 * Porta do `devUrl` de `tauri.conf.json`. Um binario debug aponta para ela, e
 * nao para `frontendDist`: sem alguem servindo ali, a janela abre numa pagina
 * que nunca carrega e o bench so consegue estourar o timeout. Foi exatamente o
 * que aconteceu no container: `[world-bench] navigating to
 * http://localhost:1420/world?bench=llvmpipe` e nada depois. O smoke test nao
 * pega isso porque so exige que a janela exista.
 */
export const DEV_PORT = 1420;

/** @type {Record<string, string>} */
const MIME = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".mjs": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".png": "image/png",
  ".jpg": "image/jpeg",
  ".svg": "image/svg+xml",
  ".webp": "image/webp",
  ".woff": "font/woff",
  ".woff2": "font/woff2",
  ".ttf": "font/ttf",
  ".wasm": "application/wasm",
  ".ico": "image/x-icon",
  ".map": "application/json; charset=utf-8",
};

/**
 * Socket do `tauri-plugin-single-instance`. O caminho e fixo no plugin
 * (`/tmp/{identifier com . e - virando _}_si.sock`, `platform_impl/macos.rs`),
 * sem variavel de ambiente que o desvie: enquanto ele existir e alguem estiver
 * escutando, uma segunda instancia sai em silencio com codigo 0 antes de criar
 * a janela. O `omniget.app` do dono costuma estar aberto, e nao e o bench que
 * decide fechar o app de ninguem.
 *
 * `--si-socket-aside` renomeia o socket enquanto o bench roda e devolve no
 * lugar depois — mesmo inode, o app que estava escutando continua escutando no
 * descritor que ja tem. A restauracao acontece no `finally` e tambem em SIGINT
 * e SIGTERM, para que um Ctrl-C nao deixe o socket fora do lugar.
 * @param {string} identifier
 */
export function singleInstanceSocketPath(identifier) {
  return `/tmp/${identifier.replace(/[.-]/g, "_")}_si.sock`;
}

/** @param {number} port @returns {Promise<boolean>} */
export function portInUse(port) {
  return new Promise((done) => {
    const socket = createConnection({ port, host: "127.0.0.1" });
    socket.on("connect", () => {
      socket.destroy();
      done(true);
    });
    socket.on("error", () => done(false));
    socket.setTimeout(500, () => {
      socket.destroy();
      done(false);
    });
  });
}

/**
 * Servidor estatico minimo do `build/`, com o fallback para `index.html` que o
 * `adapter-static` exige (SPA). Sem dependencia nova: `node:http` e `node:fs`.
 * @param {string} dir @param {number} port
 */
export function serveStatic(dir, port) {
  const root = resolve(dir);
  const server = createServer((req, res) => {
    const url = new URL(req.url ?? "/", "http://localhost");
    // `normalize` + prefixo: o servidor so existe para o bench, mas um
    // `..` no caminho nao deve sair da pasta de build mesmo assim.
    let file = join(root, normalize(decodeURIComponent(url.pathname)));
    if (!file.startsWith(root)) file = root;
    if (!existsSync(file) || statSync(file).isDirectory()) {
      const index = join(file, "index.html");
      file = existsSync(index) ? index : join(root, "index.html");
    }
    if (!existsSync(file)) {
      res.writeHead(404).end("not found");
      return;
    }
    res.writeHead(200, { "content-type": MIME[extname(file)] ?? "application/octet-stream" });
    createReadStream(file).pipe(res);
  });
  return new Promise((done) => server.listen(port, "127.0.0.1", () => done(server)));
}

/**
 * Le `a.b.c` de um objeto sem explodir no caminho.
 * @param {unknown} obj
 * @param {string} path
 * @returns {number | undefined}
 */
export function pick(obj, path) {
  /** @type {unknown} */
  let cursor = obj;
  for (const part of path.split(".")) {
    if (cursor === null || typeof cursor !== "object") return undefined;
    cursor = /** @type {Record<string, unknown>} */ (cursor)[part];
  }
  return typeof cursor === "number" && Number.isFinite(cursor) ? cursor : undefined;
}

/**
 * Funcao pura do portao: compara um resultado com o baseline do mesmo cenario.
 *
 * Regras, todas deliberadas:
 * - metrica sem baseline nao reprova (dia um de um cenario novo nao pinta de
 *   vermelho); aparece como `skipped: 'sem baseline'`;
 * - baseline 0 tambem nao reprova: nao existe percentual sobre zero;
 * - `error` no resultado reprova sempre, mesmo com todas as metricas boas — um
 *   bench que nao desenhou nada mede 0 ms de frame e "0" nunca e regressao;
 * - **`scene.frames == 0` ou `scene.fps == 0` reprova sempre**, com ou sem
 *   baseline. Esta regra existe porque a alternativa aconteceu: uma rodada em
 *   que o rAF nunca disparou saiu com `frames: 0, fps: 0, draw_calls: 0`,
 *   imprimiu OK e devolveu 0. Um bench que nao mediu nada precisa parecer
 *   diferente de um bench que mediu;
 * - piorar mais que `tolerancePct` reprova; melhorar nunca reprova.
 *
 * @param {unknown} baselineFile conteudo de `world-baseline.json`
 * @param {unknown} result o JSON de uma linha `[world-bench] {...}`
 * @param {{ scenario?: string, tolerancePct?: number, metrics?: typeof METRICS }} [opts]
 * @returns {{ scenario: string, tolerancePct: number, ok: boolean, baselineEmpty: boolean, pumped: boolean, rows: {
 *   metric: string, label: string, dir: 'lower' | 'higher', baseline: number | undefined,
 *   value: number | undefined, deltaPct: number | undefined, ok: boolean, skipped?: string }[],
 *   failures: string[], sanityFailures: string[] }}
 */
export function compareToBaseline(baselineFile, result, opts = {}) {
  const scenario =
    opts.scenario ?? (typeof pickString(result, "scenario") === "string" ? pickString(result, "scenario") : "") ?? "";
  const file = /** @type {Record<string, any>} */ (baselineFile ?? {});
  const tolerancePct = opts.tolerancePct ?? (typeof file.tolerance_pct === "number" ? file.tolerance_pct : DEFAULT_TOLERANCE_PCT);
  const baseline = file.scenarios?.[scenario]?.metrics ?? {};
  const baselineEmpty = Object.keys(baseline).length === 0;

  /** @type {string[]} */
  const failures = [];
  /**
   * Falhas que dizem "isto nao e uma medicao", separadas das que dizem "isto
   * piorou". `--no-compare` desliga as segundas e nunca as primeiras.
   * @type {string[]}
   */
  const sanityFailures = [];

  // Portao de sanidade, antes e independente do baseline: um relatorio sem
  // frames nao e uma medicao, e nenhuma comparacao percentual o pegaria — zero
  // nunca e "10% pior" que nada.
  const frames = pick(result, "scene.frames");
  const fps = pick(result, "scene.fps");
  if (frames === undefined || frames === 0) {
    sanityFailures.push(
      `o bench nao desenhou nenhum frame (scene.frames=${frames ?? "ausente"}) — ` +
        "medicao invalida, nao regressao; veja raf_stalls/raf_pumped e o campo error no JSON",
    );
  }
  if (fps === undefined || fps === 0) {
    sanityFailures.push(`fps zerado (scene.fps=${fps ?? "ausente"}) — o loop de medicao nao rodou`);
  }
  const rows = (opts.metrics ?? METRICS).map((metric) => {
    const value = pick(result, metric.path);
    const base = pick(baseline, metric.path);
    if (base === undefined || base === 0) {
      return { metric: metric.path, label: metric.label, dir: metric.dir, baseline: base, value, ok: true, skipped: "sem baseline", deltaPct: undefined };
    }
    if (value === undefined) {
      failures.push(`${metric.label}: ausente no resultado (baseline ${base})`);
      return { metric: metric.path, label: metric.label, dir: metric.dir, baseline: base, value, ok: false, deltaPct: undefined };
    }
    // Sinal normalizado: positivo = pior, em qualquer direcao.
    const raw = metric.dir === "lower" ? (value - base) / base : (base - value) / base;
    const deltaPct = Math.round(raw * 1000) / 10;
    const ok = deltaPct <= tolerancePct;
    if (!ok) failures.push(`${metric.label}: ${value} vs baseline ${base} (${deltaPct > 0 ? "+" : ""}${deltaPct}% pior)`);
    return { metric: metric.path, label: metric.label, dir: metric.dir, baseline: base, value, deltaPct, ok };
  });

  // Aviso, nao reprovacao: uma rodada com o relogio de frame bombeado mediu
  // vazao do renderer, nao frames apresentados. O numero e real e util; so nao
  // e comparavel com um orcamento de fps em vsync.
  const pumped = (/** @type {Record<string, unknown>} */ (result ?? {})).raf_pumped === true;

  const error = pickString(result, "error");
  if (error) sanityFailures.push(`o bench reportou erro: ${error}`);

  const all = [...sanityFailures, ...failures];
  return {
    scenario,
    tolerancePct,
    ok: all.length === 0,
    baselineEmpty,
    pumped,
    rows,
    failures: all,
    sanityFailures,
  };
}

/**
 * @param {unknown} obj
 * @param {string} path
 * @returns {string | undefined}
 */
export function pickString(obj, path) {
  /** @type {unknown} */
  let cursor = obj;
  for (const part of path.split(".")) {
    if (cursor === null || typeof cursor !== "object") return undefined;
    cursor = /** @type {Record<string, unknown>} */ (cursor)[part];
  }
  return typeof cursor === "string" && cursor.length > 0 ? cursor : undefined;
}

/**
 * Tabela em markdown, para o `$GITHUB_STEP_SUMMARY` e para o terminal.
 * @param {ReturnType<typeof compareToBaseline>} comparison
 * @returns {string}
 */
export function formatTable(comparison) {
  const lines = [
    `### world-bench — cenario \`${comparison.scenario}\` (tolerancia ${comparison.tolerancePct}%)`,
    "",
  ];
  if (comparison.pumped) {
    lines.push(
      "> **rAF bombeado:** o webview nao entregou frames de vsync (janela sem foco,",
      "> ocluida ou tela apagada). `fps` aqui e **vazao do renderer**, nao frames",
      "> apresentados — compare com outra rodada bombeada, nunca com um alvo de vsync.",
      "",
    );
  }
  if (comparison.baselineEmpty) {
    lines.push(
      "> **baseline vazio: sem comparacao.** Os numeros abaixo foram medidos mas nao",
      "> confrontados com nada. Quem preenche `scripts/bench/world-baseline.json` e o",
      "> orquestrador, ao fechar a fase.",
      "",
    );
  }
  lines.push("| metrica | baseline | agora | delta |", "| --- | --- | --- | --- |");
  for (const row of comparison.rows) {
    const delta = row.skipped
      ? row.skipped
      : row.deltaPct === undefined
        ? "—"
        : `${row.deltaPct > 0 ? "+" : ""}${row.deltaPct}% ${row.ok ? "ok" : "REPROVA"}`;
    lines.push(`| ${row.label} | ${row.baseline ?? "—"} | ${row.value ?? "—"} | ${delta} |`);
  }
  if (comparison.failures.length > 0) {
    lines.push("", "**Reprovou:**", ...comparison.failures.map((f) => `- ${f}`));
  }
  return lines.join("\n");
}

/**
 * Extrai o ultimo objeto `[world-bench] {...}` da saida do app.
 * @param {string} output
 * @returns {unknown}
 */
export function extractReport(output) {
  const marker = "[world-bench] ";
  /** @type {unknown} */
  let found;
  for (const line of output.split(/\r?\n/)) {
    const at = line.indexOf(marker);
    if (at === -1) continue;
    try {
      found = JSON.parse(line.slice(at + marker.length));
    } catch {
      // linha de log que por acaso cita o marcador; nao e o relatorio
    }
  }
  return found;
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

const HELP = `world-bench — bench de pior caso do mundo (Fase 6)

  node scripts/world-bench.mjs [opcoes]

  --scenario <nome>   ${Object.keys(SCENARIOS).join(" | ")}  (padrao: llvmpipe)
  --route <rota>      world | city  (padrao: world; city mede o 3D tier 0 e o
                      modo leve da cidade de exemplo, baseline city-<cenario>)
  --bin <caminho>     binario do app (padrao: src-tauri/target/debug/omniget)
  --baseline <arq>    padrao: scripts/bench/world-baseline.json
  --out <arq>         grava o JSON do resultado
  --summary <arq>     grava a tabela markdown (use $GITHUB_STEP_SUMMARY)
  --timeout <ms>      teto do app (padrao: 120000, 300000 na cidade; tambem vira OMNIGET_WORLD_BENCH_TIMEOUT_MS)
  --serve <dir>       pasta servida na porta do devUrl (padrao: build)
  --si-socket-aside   tira o socket do single-instance do caminho enquanto mede
                      e devolve depois (macOS/Linux; use quando o app ja estiver
                      aberto, em vez de fechar o app)
  --no-serve          nao subir o servidor estatico (use com 'pnpm dev' aberto)
  --no-compare        so mede e grava, nao reprova
  --help

Cenarios:
${Object.entries(SCENARIOS)
  .map(([name, s]) => `  ${name.padEnd(14)} ${s.about}`)
  .join("\n")}

O cenario vira variaveis de ambiente (estudo 73 §B) e chega ao app como
OMNIGET_WORLD_BENCH=<nome>. O app abre /world?bench=<nome>, mede, imprime uma
linha [world-bench] {json} e sai.`;

/**
 * @param {string[]} argv
 * @returns {Record<string, string | boolean>}
 */
export function parseArgs(argv) {
  /** @type {Record<string, string | boolean>} */
  const out = {};
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (!arg.startsWith("--")) continue;
    const key = arg.slice(2);
    const next = argv[i + 1];
    if (next === undefined || next.startsWith("--")) {
      out[key] = true;
    } else {
      out[key] = next;
      i++;
    }
  }
  return out;
}

async function main() {
  const args = parseArgs(process.argv.slice(2));

  if (args.help) {
    console.log(HELP);
    return 0;
  }

  const scenario = typeof args.scenario === "string" ? args.scenario : "llvmpipe";
  if (!(scenario in SCENARIOS)) {
    console.error(`cenario desconhecido: ${scenario}\nconhecidos: ${Object.keys(SCENARIOS).join(", ")}`);
    return 2;
  }

  const bin = resolve(
    typeof args.bin === "string"
      ? args.bin
      : process.platform === "win32"
        ? "src-tauri/target/debug/omniget.exe"
        : "src-tauri/target/debug/omniget",
  );
  if (!existsSync(bin)) {
    console.error(`binario nao existe: ${bin}`);
    return 2;
  }

  const route = typeof args.route === "string" ? args.route : "world";
  if (!(route in ROUTES)) {
    console.error(`rota desconhecida: ${route}\nconhecidas: ${Object.keys(ROUTES).join(", ")}`);
    return 2;
  }
  const timeoutMs = Number(typeof args.timeout === "string" ? args.timeout : route === "city" ? 300_000 : 120_000);
  const baselinePath = resolve(typeof args.baseline === "string" ? args.baseline : "scripts/bench/world-baseline.json");

  // Perfil limpo, no molde do `smoke-test.mjs`. Nao e higiene: as configuracoes
  // reais do dono mudam o que o bench mede. A deteccao de area de transferencia
  // do `+layout.svelte` faz `goto("/")` quando encontra um link copiado — e foi
  // isso que arrancou o bench da rota `/world` no meio da medicao, sem deixar
  // rastro nenhum a nao ser um timeout. Um bench que depende do que esta na area
  // de transferencia de quem o roda nao e um bench.
  const workdir = mkdtempSync(join(tmpdir(), "omniget-world-bench-"));
  const profile = join(workdir, "profile");
  mkdirSync(profile, { recursive: true });
  const env = {
    ...process.env,
    RUST_LOG: "info",
    OMNIGET_WORLD_BENCH: scenario,
    OMNIGET_WORLD_BENCH_ROUTE: ROUTES[route],
    OMNIGET_WORLD_BENCH_TIMEOUT_MS: String(timeoutMs),
    OMNIGET_DATA_DIR: join(workdir, "data"),
    LOCALAPPDATA: join(profile, "Local"),
    APPDATA: join(profile, "Roaming"),
    XDG_DATA_HOME: join(profile, "share"),
    ...SCENARIOS[scenario].env,
  };

  console.log(`[world-bench] cenario ${scenario} na rota ${ROUTES[route]}: ${SCENARIOS[scenario].about}`);
  console.log(`[world-bench] env: ${Object.entries(SCENARIOS[scenario].env).map(([k, v]) => `${k}=${v}`).join(" ") || "(nenhuma)"}`);
  console.log(`[world-bench] binario: ${bin}`);

  // O binario debug aponta para o devUrl; sem servidor ali a pagina nao existe.
  /** @type {import('node:http').Server | null} */
  let server = null;
  if (!args["no-serve"]) {
    const dir = typeof args.serve === "string" ? args.serve : "build";
    if (await portInUse(DEV_PORT)) {
      console.log(`[world-bench] porta ${DEV_PORT} ja tem alguem (pnpm dev?) — nao vou servir ${dir}`);
    } else if (!existsSync(resolve(dir))) {
      console.error(`[world-bench] ${resolve(dir)} nao existe — rode \`pnpm build\` antes`);
      return 2;
    } else {
      server = /** @type {import('node:http').Server} */ (await serveStatic(dir, DEV_PORT));
      console.log(`[world-bench] servindo ${resolve(dir)} em http://localhost:${DEV_PORT}`);
    }
  }

  // Socket do single-instance de lado, se pedido.
  /** @type {null | { from: string, to: string }} */
  let aside = null;
  /** @type {() => void} */
  const restoreSocket = () => {
    if (!aside) return;
    const { from, to } = aside;
    aside = null;
    try {
      if (existsSync(to)) renameSync(to, from);
    } catch (e) {
      console.error(`[world-bench] NAO consegui devolver o socket para ${from}: ${e}`);
    }
  };
  if (args["si-socket-aside"]) {
    const identifier = JSON.parse(readFileSync(resolve("src-tauri/tauri.conf.json"), "utf8")).identifier;
    const from = singleInstanceSocketPath(identifier);
    if (existsSync(from)) {
      const to = `${from}.world-bench-aside`;
      renameSync(from, to);
      aside = { from, to };
      process.on("SIGINT", restoreSocket);
      process.on("SIGTERM", restoreSocket);
      process.on("exit", restoreSocket);
      console.log(`[world-bench] socket do single-instance de lado: ${from} -> ${to}`);
    } else {
      console.log(`[world-bench] nenhum socket de single-instance em ${from} — nada a tirar do caminho`);
    }
  }

  const child = spawn(bin, [], { env, stdio: ["ignore", "pipe", "pipe"] });
  let out = "";
  const cap = (/** @type {Buffer} */ chunk) => {
    const text = chunk.toString();
    out += text;
    process.stdout.write(text);
  };
  child.stdout.on("data", cap);
  child.stderr.on("data", cap);

  // Teto do lado de fora, alem do teto que o proprio app arma: se o processo
  // travar antes do setup, o timer de dentro nunca chega a existir.
  const killer = setTimeout(() => {
    console.error(`\n[world-bench] o app nao terminou em ${timeoutMs + 30_000}ms — matando`);
    child.kill("SIGKILL");
  }, timeoutMs + 30_000);

  const code = await new Promise((done) => {
    child.on("exit", (c) => {
      clearTimeout(killer);
      done(c ?? 1);
    });
    child.on("error", (e) => {
      clearTimeout(killer);
      console.error(`[world-bench] falha ao executar: ${e.message}`);
      done(127);
    });
  });

  server?.close();
  restoreSocket();
  try {
    rmSync(workdir, { recursive: true, force: true });
  } catch {
    /* limpeza best-effort; nao e motivo para reprovar a medicao */
  }

  const report = extractReport(out);
  if (report === undefined) {
    console.error(`\n[world-bench] REPROVADO: nenhuma linha "[world-bench] {json}" na saida (app saiu com ${code})`);
    return 1;
  }

  if (typeof args.out === "string") {
    mkdirSync(dirname(resolve(args.out)), { recursive: true });
    writeFileSync(resolve(args.out), `${JSON.stringify(report, null, 2)}\n`);
    console.log(`[world-bench] JSON em ${resolve(args.out)}`);
  }

  /** @type {unknown} */
  let baselineFile = {};
  if (existsSync(baselinePath)) {
    baselineFile = JSON.parse(readFileSync(baselinePath, "utf8"));
  } else {
    console.warn(`[world-bench] sem baseline em ${baselinePath} — nada a comparar`);
  }

  const key = baselineKey(route, scenario);
  const comparison = compareToBaseline(baselineFile, report, {
    scenario: key,
    metrics: route === "city" ? CITY_METRICS : METRICS,
  });
  if (comparison.baselineEmpty) {
    console.log(`[world-bench] baseline vazio: sem comparacao (cenario ${key})`);
  }
  const table = formatTable(comparison);
  console.log(`\n${table}`);
  if (typeof args.summary === "string") {
    writeFileSync(resolve(args.summary), `${table}\n`, { flag: "a" });
  }

  if (comparison.sanityFailures.length > 0) {
    // Nunca silenciado por `--no-compare`: nao ha o que comparar quando nao
    // houve medicao, e sair 0 aqui e o bug que o verificador pegou.
    console.error("\n[world-bench] REPROVADO: a rodada nao produziu uma medicao valida");
    for (const f of comparison.sanityFailures) console.error(`  - ${f}`);
    return 1;
  }
  if (args["no-compare"]) {
    console.log("[world-bench] --no-compare: nao reprova por regressao");
    return 0;
  }
  if (!comparison.ok) {
    console.error("\n[world-bench] REPROVADO");
    return 1;
  }
  if (code !== 0) {
    console.error(`\n[world-bench] REPROVADO: app saiu com ${code}`);
    return 1;
  }
  console.log("\n[world-bench] OK");
  return 0;
}

// Sem `import.meta.main` em Node 22; o teste importa este arquivo e nao pode
// disparar o CLI.
if (process.argv[1] && resolve(process.argv[1]).endsWith("world-bench.mjs")) {
  process.exit(await main());
}
