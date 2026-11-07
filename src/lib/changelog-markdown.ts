import { escapeHtml } from "$lib/markdown";

export function renderMarkdown(md: string): string {
  const out: string[] = [];
  let inFence = false;
  let fenceLines: string[] = [];
  for (const line of md.split("\n")) {
    if (line.trim().startsWith("```")) {
      if (inFence) {
        out.push(`<pre><code>${escapeHtml(fenceLines.join("\n"))}</code></pre>`);
        fenceLines = [];
      }
      inFence = !inFence;
      continue;
    }
    if (inFence) {
      fenceLines.push(line);
      continue;
    }
    if (/^\s*(---|\*\*\*|___)\s*$/.test(line)) {
      out.push("<hr />");
    } else if (line.startsWith("### ")) {
      out.push(`<h4>${escapeHtml(line.slice(4))}</h4>`);
    } else if (line.startsWith("## ")) {
      out.push(`<h3>${escapeHtml(line.slice(3))}</h3>`);
    } else if (line.startsWith("# ")) {
      out.push(`<h2>${escapeHtml(line.slice(2))}</h2>`);
    } else if (line.startsWith("- ") || line.startsWith("* ")) {
      out.push(`<li>${formatInline(line.slice(2))}</li>`);
    } else if (line.trim() === "") {
      out.push("<br />");
    } else {
      out.push(`<p>${formatInline(line)}</p>`);
    }
  }
  if (inFence && fenceLines.length) {
    out.push(`<pre><code>${escapeHtml(fenceLines.join("\n"))}</code></pre>`);
  }
  return out.join("");
}

function formatInline(str: string): string {
  let result = escapeHtml(str);
  result = result.replace(/\*\*(.+?)\*\*/g, "<strong>$1</strong>");
  result = result.replace(/`(.+?)`/g, "<code>$1</code>");
  result = result.replace(/\[(.+?)\]\((.+?)\)/g, (_, label: string, url: string) =>
    /^(https?:|mailto:)/i.test(url)
      ? `<a href="${url}" target="_blank" rel="noopener">${label}</a>`
      : label
  );
  return result;
}
