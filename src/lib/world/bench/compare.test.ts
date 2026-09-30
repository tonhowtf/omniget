// Tests for the pure half of `scripts/world-bench.mjs`: the baseline
// comparison that decides whether the world-bench job is red. It lives here and
// not next to the script because `vitest.config.ts` only looks at
// `src/**/*.test.ts`; the logic itself stays in the script so the CLI has no
// build step. Owned by `f6-bench`.

import { describe, expect, it } from "vitest";
import {
  METRICS,
  CITY_METRICS,
  baselineKey,
  SCENARIOS,
  compareToBaseline,
  extractReport,
  formatTable,
  parseArgs,
  pick,
  singleInstanceSocketPath,
} from "../../../../scripts/world-bench.mjs";
import { percentile, round } from "./ipc-bench";

const baselineFile = {
  tolerance_pct: 10,
  scenarios: {
    llvmpipe: {
      metrics: {
        calibrate_median_ms: 100,
        first_frame_ms: 2000,
        scene: { median_frame_ms: 40, p95_frame_ms: 60, cpu_ms: 8, fps: 25 },
        rss_extra_mb: 100,
        ipc: { "2kb_p50_ms": 1, "50kb_p50_ms": 2, "200kb_p50_ms": 5, "200kb_15hz_cpu_pct": 10 },
      },
    },
  },
};

/** A result that matches the baseline exactly. */
function goodResult() {
  return {
    scenario: "llvmpipe",
    calibrate_median_ms: 100,
    first_frame_ms: 2000,
    scene: { median_frame_ms: 40, p95_frame_ms: 60, cpu_ms: 8, fps: 25, frames: 480 },
    rss_extra_mb: 100,
    ipc: { "2kb_p50_ms": 1, "50kb_p50_ms": 2, "200kb_p50_ms": 5, "200kb_15hz_cpu_pct": 10 },
  };
}

describe("pick", () => {
  it("walks a dotted path and only returns finite numbers", () => {
    expect(pick({ a: { b: 3 } }, "a.b")).toBe(3);
    expect(pick({ a: { b: "3" } }, "a.b")).toBeUndefined();
    expect(pick({ a: null }, "a.b")).toBeUndefined();
    expect(pick(undefined, "a")).toBeUndefined();
    expect(pick({ a: { b: Infinity } }, "a.b")).toBeUndefined();
  });
});

describe("compareToBaseline", () => {
  it("passes when the result equals the baseline", () => {
    const c = compareToBaseline(baselineFile, goodResult());
    expect(c.scenario).toBe("llvmpipe");
    expect(c.tolerancePct).toBe(10);
    expect(c.ok).toBe(true);
    expect(c.failures).toEqual([]);
    expect(c.rows).toHaveLength(METRICS.length);
  });

  it("tolerates a regression up to the tolerance and fails past it", () => {
    const withinTolerance = goodResult();
    withinTolerance.scene.median_frame_ms = 44; // +10%
    expect(compareToBaseline(baselineFile, withinTolerance).ok).toBe(true);

    const past = goodResult();
    past.scene.median_frame_ms = 44.5; // +11.2%
    const c = compareToBaseline(baselineFile, past);
    expect(c.ok).toBe(false);
    expect(c.failures.join(" ")).toContain("frame mediano");
  });

  it("never fails an improvement, in either direction", () => {
    const better = goodResult();
    better.scene.median_frame_ms = 4; // 10x faster
    better.scene.fps = 250; // 10x more
    better.rss_extra_mb = 1;
    const c = compareToBaseline(baselineFile, better);
    expect(c.ok).toBe(true);
    const fps = c.rows.find((r) => r.metric === "scene.fps");
    expect(fps?.deltaPct).toBeLessThan(0);
  });

  it("knows that fewer fps is worse while fewer ms is better", () => {
    const slower = goodResult();
    slower.scene.fps = 20; // -20%
    const c = compareToBaseline(baselineFile, slower);
    expect(c.ok).toBe(false);
    expect(c.rows.find((r) => r.metric === "scene.fps")?.deltaPct).toBe(20);
  });

  it("skips metrics with no baseline instead of failing them", () => {
    const c = compareToBaseline({ scenarios: { llvmpipe: { metrics: {} } } }, goodResult());
    expect(c.ok).toBe(true);
    expect(c.baselineEmpty).toBe(true);
    expect(c.rows.every((r) => r.skipped === "sem baseline")).toBe(true);
    expect(formatTable(c)).toContain("baseline vazio: sem comparacao");
  });

  it("skips an unknown scenario the same way", () => {
    const c = compareToBaseline(baselineFile, { ...goodResult(), scenario: "native" });
    expect(c.scenario).toBe("native");
    expect(c.ok).toBe(true);
  });

  it("fails a metric that the baseline has and the result does not", () => {
    const partial = goodResult();
    // @ts-expect-error - on purpose: a report that forgot a field
    delete partial.scene.p95_frame_ms;
    const c = compareToBaseline(baselineFile, partial);
    expect(c.ok).toBe(false);
    expect(c.failures.join(" ")).toContain("ausente no resultado");
  });

  it("fails on a reported error even when every number is perfect", () => {
    const c = compareToBaseline(baselineFile, { ...goodResult(), error: "ERR_WORLD_RENDER_STUB" });
    expect(c.ok).toBe(false);
    expect(c.baselineEmpty).toBe(false);
    expect(c.failures.join(" ")).toContain("ERR_WORLD_RENDER_STUB");
  });

  it("honours an explicit tolerance over the one in the file", () => {
    const worse = goodResult();
    worse.scene.cpu_ms = 12; // +50%
    expect(compareToBaseline(baselineFile, worse, { tolerancePct: 100 }).ok).toBe(true);
    expect(compareToBaseline(baselineFile, worse, { tolerancePct: 1 }).ok).toBe(false);
  });

  it("does not divide by a zero baseline", () => {
    const zeroed = { scenarios: { llvmpipe: { metrics: { scene: { fps: 0, frames: 0 } } } } };
    const c = compareToBaseline(zeroed, goodResult());
    expect(c.ok).toBe(true);
    expect(c.rows.find((r) => r.metric === "scene.fps")?.skipped).toBe("sem baseline");
  });
});

// A regra que faltava quando o verificador pegou uma rodada verde medindo zero:
// `frames: 0, fps: 0, draw_calls: 0` saiu com OK e codigo 0, porque nenhuma
// comparacao percentual reprova o zero — nao existe "10% pior que nada".
describe("portao de sanidade (medicao valida)", () => {
  it("reprova frames == 0 mesmo sem baseline nenhum", () => {
    const zeroed = { scenario: "native", scene: { frames: 0, fps: 0, draw_calls: 0, cpu_ms: 0 }, raf_stalls: 3 };
    const c = compareToBaseline({}, zeroed);
    expect(c.baselineEmpty).toBe(true);
    expect(c.ok).toBe(false);
    expect(c.sanityFailures).toHaveLength(2);
    expect(c.sanityFailures.join(" ")).toContain("nenhum frame");
  });

  it("reprova frames == 0 mesmo com todas as outras metricas iguais ao baseline", () => {
    const zeroed = goodResult();
    zeroed.scene.frames = 0;
    const c = compareToBaseline(baselineFile, zeroed);
    expect(c.ok).toBe(false);
    expect(c.sanityFailures.join(" ")).toContain("scene.frames=0");
  });

  it("reprova fps == 0 separadamente de frames", () => {
    const stopped = goodResult();
    stopped.scene.fps = 0;
    const c = compareToBaseline(baselineFile, stopped);
    expect(c.ok).toBe(false);
    expect(c.sanityFailures.join(" ")).toContain("fps zerado");
  });

  it("trata campo ausente como zero, nao como 'sem dado'", () => {
    const c = compareToBaseline({}, { scenario: "native", scene: {} });
    expect(c.ok).toBe(false);
    expect(c.sanityFailures.join(" ")).toContain("ausente");
  });

  it("poe o erro reportado entre as falhas de sanidade, nao entre as de regressao", () => {
    const c = compareToBaseline(baselineFile, { ...goodResult(), error: "ERR_WORLD_NO_WEBGL" });
    expect(c.sanityFailures.join(" ")).toContain("ERR_WORLD_NO_WEBGL");
    expect(c.ok).toBe(false);
  });

  it("nao inventa falha quando a rodada mediu de verdade", () => {
    const c = compareToBaseline(baselineFile, goodResult());
    expect(c.sanityFailures).toEqual([]);
    expect(c.ok).toBe(true);
  });

  it("separa sanidade de regressao: uma regressao pura nao vira falha de sanidade", () => {
    const slower = goodResult();
    slower.scene.fps = 20;
    const c = compareToBaseline(baselineFile, slower);
    expect(c.ok).toBe(false);
    expect(c.sanityFailures).toEqual([]);
    expect(c.failures.join(" ")).toContain("fps");
  });
});

describe("aviso de rAF bombeado", () => {
  it("avisa sem reprovar quando o relogio de frame nao foi vsync", () => {
    const c = compareToBaseline(baselineFile, { ...goodResult(), raf_pumped: true });
    expect(c.pumped).toBe(true);
    expect(c.ok).toBe(true);
    expect(formatTable(c)).toContain("rAF bombeado");
  });

  it("nao avisa numa rodada normal", () => {
    const c = compareToBaseline(baselineFile, goodResult());
    expect(c.pumped).toBe(false);
    expect(formatTable(c)).not.toContain("rAF bombeado");
  });
});

describe("singleInstanceSocketPath", () => {
  it("reproduz a regra do plugin: '.' e '-' viram '_'", () => {
    expect(singleInstanceSocketPath("wtf.tonho.omniget")).toBe("/tmp/wtf_tonho_omniget_si.sock");
    expect(singleInstanceSocketPath("com.my-app.thing")).toBe("/tmp/com_my_app_thing_si.sock");
  });
});

describe("formatTable", () => {
  it("prints one row per metric and lists the failures", () => {
    const worse = goodResult();
    worse.scene.fps = 10;
    const table = formatTable(compareToBaseline(baselineFile, worse));
    expect(table).toContain("| metrica | baseline | agora | delta |");
    expect(table).toContain("REPROVA");
    expect(table).toContain("**Reprovou:**");
    for (const metric of METRICS) expect(table).toContain(metric.label);
  });
});

describe("extractReport", () => {
  it("takes the last well-formed marked line and ignores the rest", () => {
    const output = [
      "2026-09-17 INFO omniget starting",
      "[world-bench] nao e json",
      '[world-bench] {"scenario":"llvmpipe","scene":{"fps":1}}',
      "algum log no meio",
      '2026-09-17 INFO prefixo antes do [world-bench] {"scenario":"llvmpipe","scene":{"fps":2}}',
    ].join("\n");
    expect(extractReport(output)).toEqual({ scenario: "llvmpipe", scene: { fps: 2 } });
  });

  it("returns undefined when the app never printed a report", () => {
    expect(extractReport("nada aqui\n")).toBeUndefined();
  });
});

describe("parseArgs", () => {
  it("reads values and bare flags", () => {
    expect(parseArgs(["--scenario", "llvmpipe", "--no-compare", "--out", "a.json"])).toEqual({
      scenario: "llvmpipe",
      "no-compare": true,
      out: "a.json",
    });
    expect(parseArgs(["--help"])).toEqual({ help: true });
    expect(parseArgs([])).toEqual({});
  });
});

describe("SCENARIOS", () => {
  it("carries the verified recipes of estudo 73 §B", () => {
    expect(SCENARIOS.llvmpipe.env).toEqual({
      LIBGL_ALWAYS_SOFTWARE: "1",
      GALLIUM_DRIVER: "llvmpipe",
      LP_NUM_THREADS: "2",
    });
    expect(SCENARIOS.nocompositing.env.WEBKIT_DISABLE_COMPOSITING_MODE).toBe("1");
    // No macOS essas variaveis nao fazem nada: o cenario nativo nao mente
    // dizendo que mediu o pior caso.
    expect(SCENARIOS.native.env).toEqual({});
  });
});

describe("percentile / round", () => {
  it("returns a real sample, never an interpolation", () => {
    const values = [5, 1, 4, 2, 3];
    expect(percentile(values, 50)).toBe(3);
    expect(percentile(values, 95)).toBe(5);
    expect(percentile(values, 0)).toBe(1);
    expect(values).toEqual([5, 1, 4, 2, 3]); // nao reordena o array do chamador
  });

  it("answers 0 for an empty sample instead of NaN", () => {
    expect(percentile([], 50)).toBe(0);
  });

  it("rounds to the asked number of decimals", () => {
    expect(round(1.23456)).toBe(1.235);
    expect(round(1.26, 1)).toBe(1.3);
  });
});

describe("city route", () => {
  it("compares the city metrics under their own baseline key", () => {
    expect(baselineKey("city", "llvmpipe")).toBe("city-llvmpipe");
    expect(baselineKey("world", "llvmpipe")).toBe("llvmpipe");
    const report = {
      scenario: "llvmpipe",
      route: "city",
      city: { three3d: { p50_ms: 30, p95_ms: 50 }, lite: { p50_ms: 10, p95_ms: 20, first_frame_ms: 5 } },
      scene: { frames: 240, fps: 100, median_frame_ms: 10, p95_frame_ms: 20 },
    };
    const base = { scenarios: { "city-llvmpipe": { metrics: { city: { lite: { p95_ms: 10 } } } } } };
    const c = compareToBaseline(base, report, { scenario: "city-llvmpipe", metrics: CITY_METRICS });
    expect(c.rows).toHaveLength(CITY_METRICS.length);
    expect(c.ok).toBe(false);
    expect(c.failures.join()).toContain("modo leve p95");
  });
});
