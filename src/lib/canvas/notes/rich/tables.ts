// Copyable tables (Maestri 0.24.1): Copy as Table puts HTML + RTF + TSV on
// the clipboard so a spreadsheet, Pages or Word receives a real table; Copy
// Markdown gives the source back; a double-click on a cell copies that cell.
// Pure string builders live here (tested); the clipboard write is in
// `clipboard.ts`.

export interface TableData {
  header: string[];
  rows: string[][];
  align: ('left' | 'center' | 'right' | null)[];
}

const cellText = (el: Element) => (el.textContent ?? '').replace(/\s+/g, ' ').trim();

export function readTable(table: HTMLTableElement): TableData {
  const all = Array.from(table.rows);
  const headRow = table.tHead?.rows[0] ?? all[0];
  const header = headRow ? Array.from(headRow.cells).map(cellText) : [];
  const align = headRow
    ? Array.from(headRow.cells).map((c) => {
        const a = (c.getAttribute('align') || (c as HTMLElement).style?.textAlign || '').toLowerCase();
        return a === 'left' || a === 'center' || a === 'right' ? a : null;
      })
    : [];
  const rows = all.filter((r) => r !== headRow).map((r) => Array.from(r.cells).map(cellText));
  return { header, rows, align };
}

const tsvCell = (s: string) => s.replace(/[\t\r\n]+/g, ' ');

export function toTsv(t: TableData): string {
  return [t.header, ...t.rows].map((r) => r.map(tsvCell).join('\t')).join('\n');
}

const mdCell = (s: string) => s.replace(/\|/g, '\\|').replace(/\n/g, ' ');

export function toMarkdown(t: TableData): string {
  const cols = Math.max(t.header.length, ...t.rows.map((r) => r.length));
  const pad = (r: string[]) => Array.from({ length: cols }, (_, i) => mdCell(r[i] ?? ''));
  const sep = Array.from({ length: cols }, (_, i) => {
    const a = t.align[i];
    return a === 'center' ? ':---:' : a === 'right' ? '---:' : a === 'left' ? ':---' : '---';
  });
  const line = (cells: string[]) => `| ${cells.join(' | ')} |`;
  return [line(pad(t.header)), line(sep), ...t.rows.map((r) => line(pad(r)))].join('\n');
}

const htmlEsc = (s: string) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');

export function toHtml(t: TableData): string {
  const th = t.header.map((c) => `<th>${htmlEsc(c)}</th>`).join('');
  const body = t.rows.map((r) => `<tr>${r.map((c) => `<td>${htmlEsc(c)}</td>`).join('')}</tr>`).join('');
  return `<table border="1" cellspacing="0" cellpadding="4"><thead><tr>${th}</tr></thead><tbody>${body}</tbody></table>`;
}

function rtfEsc(s: string): string {
  let out = '';
  for (const ch of s) {
    const code = ch.codePointAt(0) ?? 0;
    if (ch === '\\' || ch === '{' || ch === '}') out += `\\${ch}`;
    else if (code > 127) {
      // RTF \u takes a signed 16-bit value; astral chars go as surrogates.
      for (const unit of ch.length === 2 ? [ch.charCodeAt(0), ch.charCodeAt(1)] : [code]) {
        out += `\\u${unit > 32767 ? unit - 65536 : unit}?`;
      }
    } else out += ch;
  }
  return out;
}

export function toRtf(t: TableData): string {
  const cols = Math.max(1, t.header.length, ...t.rows.map((r) => r.length));
  const width = 2000;
  const row = (cells: string[], bold: boolean) => {
    let s = '\\trowd\\trgaph60';
    for (let i = 1; i <= cols; i++) s += `\\clbrdrt\\brdrs\\clbrdrl\\brdrs\\clbrdrb\\brdrs\\clbrdrr\\brdrs\\cellx${i * width}`;
    for (let i = 0; i < cols; i++) s += `\\pard\\intbl${bold ? '\\b ' : ' '}${rtfEsc(cells[i] ?? '')}${bold ? '\\b0' : ''}\\cell`;
    return `${s}\\row`;
  };
  return `{\\rtf1\\ansi\\deff0{\\fonttbl{\\f0 Helvetica;}}\\f0\\fs22 ${row(t.header, true)}${t.rows.map((r) => row(r, false)).join('')}\\pard}`;
}
