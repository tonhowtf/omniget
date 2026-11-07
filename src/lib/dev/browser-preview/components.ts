import { emit } from "@tauri-apps/api/event";

type Args = Record<string, unknown>;

const REGISTRIES = [
  { id: "shadcn", name: "shadcn/ui", homepage: "https://ui.shadcn.com", index_url: "https://ui.shadcn.com/r/styles/new-york-v4/registry.json", item_url: "https://ui.shadcn.com/r/styles/new-york-v4/{name}.json", docs_url: "https://ui.shadcn.com/docs/components/{name}" },
  { id: "magicui", name: "Magic UI", homepage: "https://magicui.design", index_url: "https://magicui.design/r/registry.json", item_url: "https://magicui.design/r/{name}.json", docs_url: "https://magicui.design/docs/components/{name}" },
  { id: "react-bits", name: "React Bits", homepage: "https://reactbits.dev", index_url: "https://reactbits.dev/r/registry.json", item_url: "https://reactbits.dev/r/{name}.json", docs_url: null },
  { id: "cult-ui", name: "Cult UI", homepage: "https://www.cult-ui.com", index_url: "https://cult-ui.com/r/registry.json", item_url: "https://cult-ui.com/r/{name}.json", docs_url: "https://www.cult-ui.com/docs/components/{name}" },
];

type Item = { name: string; type: string; title?: string; description?: string; dependencies?: string[]; registryDependencies?: string[]; categories?: string[] };

const INDEX: Record<string, Item[]> = {
  shadcn: [
    { name: "button", type: "registry:ui", description: "Displays a button or a component that looks like a button.", dependencies: ["radix-ui", "class-variance-authority"], registryDependencies: ["utils"] },
    { name: "card", type: "registry:ui", description: "Displays a card with header, content, and footer." },
    { name: "dialog", type: "registry:ui", description: "A window overlaid on either the primary window or another dialog window.", dependencies: ["radix-ui"], registryDependencies: ["button"] },
    { name: "input", type: "registry:ui", description: "Displays a form input field or a component that looks like an input field." },
    { name: "accordion", type: "registry:ui", description: "A vertically stacked set of interactive headings that each reveal a section of content.", dependencies: ["radix-ui"] },
    { name: "login-03", type: "registry:block", title: "Login page with muted background", description: "A login page with a muted background color.", registryDependencies: ["button", "card", "input", "label"], categories: ["authentication", "login"] },
    { name: "sidebar-07", type: "registry:block", title: "Sidebar that collapses to icons", description: "A sidebar that collapses to icons.", registryDependencies: ["sidebar", "breadcrumb", "avatar"], categories: ["sidebar", "dashboard"] },
    { name: "use-mobile", type: "registry:hook", description: "Detects whether the viewport is mobile sized." },
    { name: "utils", type: "registry:lib", description: "The cn helper.", dependencies: ["clsx", "tailwind-merge"] },
    { name: "button-demo", type: "registry:example" },
  ],
  magicui: [
    { name: "marquee", type: "registry:ui", title: "Marquee", description: "An infinite scrolling component that can be used to display text, images, or videos." },
    { name: "shimmer-button", type: "registry:ui", title: "Shimmer Button", description: "A button with a shimmering light that travels around the perimeter." },
    { name: "globe", type: "registry:ui", title: "Globe", description: "An autorotating, interactive, and highly performant globe made using WebGL.", dependencies: ["cobe", "motion"] },
    { name: "number-ticker", type: "registry:ui", title: "Number Ticker", description: "Animate numbers to count up or down to a target number.", dependencies: ["motion"] },
  ],
  "react-bits": [
    { name: "AnimatedContent-TS-TW", type: "registry:component", title: "AnimatedContent", description: "Wrapper that reveals its content on scroll with slides, blur, 3D tilts and staggered children.", dependencies: ["gsap@^3.13.0"] },
    { name: "SplitText-TS-TW", type: "registry:component", title: "SplitText", description: "Splits text into characters or words and animates them in.", dependencies: ["gsap@^3.13.0", "@gsap/react"] },
    { name: "Aurora-TS-TW", type: "registry:component", title: "Aurora", description: "Flowing aurora gradient background rendered with WebGL.", dependencies: ["ogl"] },
  ],
  "cult-ui": [
    { name: "text-animate", type: "registry:ui", description: "Animated text component with customizable reveal effects and timing.", dependencies: ["motion"] },
    { name: "dynamic-island", type: "registry:ui", description: "An iOS style dynamic island with animated size transitions.", dependencies: ["motion"] },
  ],
};

const BUTTON = `import * as React from "react"
import { Slot } from "radix-ui"
import { cva, type VariantProps } from "class-variance-authority"

import { cn } from "@/lib/utils"

const buttonVariants = cva(
  "inline-flex items-center justify-center gap-2 rounded-md text-sm font-medium transition-all",
  {
    variants: {
      variant: {
        default: "bg-primary text-primary-foreground hover:bg-primary/90",
        outline: "border bg-background hover:bg-accent",
        ghost: "hover:bg-accent hover:text-accent-foreground",
      },
      size: { default: "h-9 px-4 py-2", sm: "h-8 px-3", lg: "h-10 px-6" },
    },
    defaultVariants: { variant: "default", size: "default" },
  }
)

function Button({ className, variant, size, asChild = false, ...props }: React.ComponentProps<"button"> & VariantProps<typeof buttonVariants> & { asChild?: boolean }) {
  const Comp = asChild ? Slot.Root : "button"
  return <Comp data-slot="button" className={cn(buttonVariants({ variant, size, className }))} {...props} />
}

export { Button, buttonVariants }
`;

const CSS: Record<string, string> = {
  "/preview/Projects/acme-web::app/globals.css": `@import "tailwindcss";

:root {
  --radius: 0.625rem;
  --background: oklch(1 0 0);
  --foreground: oklch(0.145 0 0);
  --primary: oklch(0.205 0 0);
  --primary-foreground: oklch(0.985 0 0);
  --border: oklch(0.922 0 0);
}

.dark {
  --background: oklch(0.145 0 0);
  --foreground: oklch(0.985 0 0);
}

@layer base {
  body {
    @apply bg-background text-foreground;
  }
}
`,
};

const OFFLINE = new Set(["https://cult-ui.com/r/registry.json"]);
const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
const cancelled = new Set<string>();

function itemDetail(url: string) {
  const reg = REGISTRIES.find((r) => url.startsWith(r.item_url.replace("{name}.json", "")));
  const name = decodeURIComponent(url.split("/").pop()?.replace(/\.json$/, "") ?? "");
  const item = reg ? INDEX[reg.id]?.find((i) => i.name === name) : undefined;
  if (!item) throw { code: "http_status", detail: "404" };
  const path = item.type === "registry:block" ? `registry/blocks/${name}/page.tsx` : item.type === "registry:hook" ? `registry/hooks/${name}.ts` : `registry/ui/${name}.tsx`;
  const content =
    name === "button"
      ? BUTTON
      : `"use client"\n\nimport * as React from "react"\nimport { cn } from "@/lib/utils"\n\nexport function ${name.replace(/(^|-)([a-z])/g, (_, __, c: string) => c.toUpperCase()).replace(/[^A-Za-z0-9]/g, "")}({ className, ...props }: React.ComponentProps<"div">) {\n  return <div className={cn("relative", className)} {...props} />\n}\n`;
  const files = [{ path, type: item.type, content }];
  if (item.type === "registry:block") files.push({ path: `registry/blocks/${name}/components/login-form.tsx`, type: "registry:component", content: BUTTON.replace(/Button/g, "LoginForm") });
  return { ...item, files, cssVars: name === "sidebar-07" ? { light: { sidebar: "oklch(0.985 0 0)" }, dark: { sidebar: "oklch(0.205 0 0)" } } : undefined };
}

async function fetchJson({ url, force }: Args) {
  const u = String(url ?? "");
  await sleep(u.endsWith("registry.json") ? 500 + Math.random() * 600 : 350);
  if (!u.startsWith("https://")) throw { code: "url_not_https", detail: null };
  const now = Math.floor(Date.now() / 1000);
  if (OFFLINE.has(u) && !force) {
    const reg = REGISTRIES.find((r) => r.index_url === u)!;
    return { url: u, body: { name: reg.name, homepage: reg.homepage, items: INDEX[reg.id] }, fetched_at: now - 30 * 3600, from_cache: true, stale: true, offline_error: { code: "network", detail: "dns error" } };
  }
  const reg = REGISTRIES.find((r) => r.index_url === u);
  if (reg) return { url: u, body: { name: reg.name, homepage: reg.homepage, items: INDEX[reg.id] }, fetched_at: now - 2 * 3600, from_cache: !force, stale: false, offline_error: null };
  if (u.includes("example.com")) throw { code: "http_status", detail: "404" };
  return { url: u, body: itemDetail(u), fetched_at: now, from_cache: false, stale: false, offline_error: null };
}

function inspectProject({ path }: Args) {
  const p = String(path ?? "");
  const fresh = p.endsWith("new-app");
  return {
    path: p,
    name: p.split("/").pop() ?? p,
    has_package_json: true,
    lockfiles: fresh ? ["package-lock.json"] : ["pnpm-lock.yaml"],
    has_components_json: !fresh,
    components_json_error: false,
    css_path: fresh ? null : "app/globals.css",
    css_exists: !fresh,
    tailwind_config: null,
    yarn_classic: false,
  };
}

async function runCommand({ runId, args }: Args) {
  const id = String(runId);
  const list = (args as string[]) ?? [];
  const init = list.includes("init");
  const url = list.find((a) => a.startsWith("https://")) ?? "";
  const name = url.split("/").pop()?.replace(/\.json$/, "") ?? "component";
  const lines = init
    ? ["- Preflight checks.", "✔ Preflight checks.", "✔ Verifying framework. Found Next.js.", "✔ Validating Tailwind CSS config. Found v4.", "✔ Writing components.json.", "✔ Checking registry.", "✔ Updating CSS variables in app/globals.css", "✔ Installing dependencies.", "✔ Created 1 file:", "  - lib/utils.ts", "", "Success! Project initialization completed."]
    : name === "globe"
      ? ["- Checking registry.", "✔ Checking registry.", "- Installing dependencies.", "npm error code ERESOLVE", "npm error ERESOLVE unable to resolve dependency tree: cobe@0.6.3 needs react@^18"]
      : ["- Checking registry.", "✔ Checking registry.", "- Installing dependencies.", "✔ Installing dependencies.", "- Updating files.", "✔ Created 1 file:", `  - components/ui/${name}.tsx`, "ℹ Skipped 1 file: (files might be identical, use --overwrite to overwrite)", "  - lib/utils.ts"];
  const output: { stream: string; line: string }[] = [];
  for (const line of lines) {
    if (cancelled.has(id)) {
      cancelled.delete(id);
      return { ok: false, code: null, cancelled: true, error: "run_cancelled", output };
    }
    await sleep(line.startsWith("-") ? 700 : 260);
    const entry = { stream: line.startsWith("npm error") ? "stderr" : "stdout", line };
    output.push(entry);
    await emit("components-output", { run_id: id, ...entry });
  }
  if (name === "globe") return { ok: false, code: 1, cancelled: false, error: "run_exit", output };
  return { ok: true, code: 0, cancelled: false, error: null, output };
}

export const componentsFixtures: Record<string, (args: Args) => unknown> = {
  components_registries: () => REGISTRIES,
  components_fetch: fetchJson,
  components_cache_clear: () => 0,
  components_inspect_project: inspectProject,
  components_read_css: ({ project, rel }) => CSS[`${project}::${rel}`] ?? "",
  components_write_css: ({ project, rel, content }) => {
    CSS[`${project}::${rel}`] = String(content ?? "");
    return `${project}/${rel}`;
  },
  components_run: runCommand,
  components_cancel: ({ runId }) => {
    cancelled.add(String(runId));
    return null;
  },
};

let picks = 0;

export function componentsDialog(options: Record<string, unknown> | undefined): unknown {
  if (!location.pathname.startsWith("/components")) return undefined;
  if (options?.directory) return picks++ % 2 === 0 ? "/preview/Projects/acme-web" : "/preview/Projects/new-app";
  return null;
}
