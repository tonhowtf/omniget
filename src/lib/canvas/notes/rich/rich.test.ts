import { describe, expect, it } from 'vitest';
import { mermaidKind, parseFlowchart, parsePie, parseSequence, renderMermaid, resolveVars } from './mermaid';
import { langOf, tokenize } from './highlight';
import { toMarkdown, toRtf, toTsv } from './tables';
import { findPaths } from './links';
import { imageSource } from '$lib/llm/markdown';

describe('mermaid', () => {
  it('parses a one-line flowchart with shapes, labels and chains', () => {
    const g = parseFlowchart('graph TD; A[Start] -->|go| B{Ok?}; B -- yes --> C((Done)); B -.-> D; C & D ==> E');
    expect(g.dir).toBe('TB');
    expect(g.nodes.get('A')?.label).toBe('Start');
    expect(g.nodes.get('B')?.shape).toBe('diamond');
    expect(g.nodes.get('C')?.shape).toBe('circle');
    const e = g.edges.map((x) => `${x.from}>${x.to}:${x.style}:${x.label}`);
    expect(e).toEqual(['A>B:solid:go', 'B>C:solid:yes', 'B>D:dotted:', 'C>E:thick:', 'D>E:thick:']);
  });

  it('draws flowcharts, sequences and pies; refuses unknown kinds', () => {
    const f = renderMermaid('flowchart LR\n  a --> b\n  b --> a');
    expect(f?.svg.startsWith('<svg')).toBe(true);
    expect(renderMermaid('sequenceDiagram\nAlice->>Bob: Hi <b>\nBob-->>Alice: ok')?.svg).toContain('Hi &lt;b&gt;');
    expect(renderMermaid('pie title Pets\n"Dogs" : 3\n"Cats" : 1')?.kind).toBe('pie');
    expect(mermaidKind('gantt\n title x')).toBeNull();
    expect(renderMermaid('classDiagram\nA <|-- B')).toBeNull();
  });

  it('escapes node labels', () => {
    const out = renderMermaid('graph TD\nA["<script>x</script>"] --> B');
    expect(out?.svg).not.toContain('<script>');
  });

  it('parses sequence notes and frames', () => {
    const s = parseSequence('sequenceDiagram\nparticipant A as Alice\nloop every min\nA->>B: ping\nend\nNote over A,B: done');
    expect(s.parts.map((p) => p.label)).toEqual(['Alice', 'B']);
    expect(s.steps.map((x) => x.t)).toEqual(['open', 'msg', 'close', 'note']);
  });

  it('parses pie values', () => {
    expect(parsePie('pie showData\ntitle T\n"a": 1.5\n"b" : 0').slices).toEqual([{ label: 'a', value: 1.5 }]);
  });

  it('bakes css variables with fallbacks when no DOM', () => {
    expect(resolveVars('fill:var(--x, #123);stroke:var(--y)', null)).toBe('fill:#123;stroke:currentColor');
  });
});

describe('highlight', () => {
  it('knows languages by info string', () => {
    expect(langOf('ts')).toBe('ts');
    expect(langOf('Rust extra')).toBe('rust');
    expect(langOf('nope')).toBeNull();
  });
  it('tokenizes keywords, strings, comments and numbers', () => {
    const t = tokenize('const a = "x"; // hi\nlet n = 42;', 'ts');
    const kinds = (k: string) => t.filter((x) => x.kind === k).map((x) => x.text);
    expect(kinds('keyword')).toEqual(['const', 'let']);
    expect(kinds('string')).toEqual(['"x"']);
    expect(kinds('comment')).toEqual(['// hi']);
    expect(kinds('number')).toEqual(['42']);
    expect(t.map((x) => x.text).join('')).toBe('const a = "x"; // hi\nlet n = 42;');
  });
  it('handles rust lifetimes and python triple strings', () => {
    const r = tokenize("fn f<'a>(x: &'a str) -> char { 'c' }", 'rust');
    expect(r.filter((x) => x.kind === 'string').map((x) => x.text)).toEqual(["'c'"]);
    const p = tokenize('"""doc\nmore""" # c', 'py');
    expect(p[0]).toEqual({ kind: 'string', text: '"""doc\nmore"""' });
  });
});

describe('tables', () => {
  const t = { header: ['a', 'b|c'], rows: [['1', 'x\ty']], align: [null, 'right' as const] };
  it('builds TSV, Markdown and RTF', () => {
    expect(toTsv(t)).toBe('a\tb|c\n1\tx y');
    expect(toMarkdown(t)).toBe('| a | b\\|c |\n| --- | ---: |\n| 1 | x\ty |');
    expect(toRtf({ header: ['é'], rows: [], align: [null] })).toContain('\\u233?');
  });
});

describe('paths', () => {
  it('finds real-looking paths and strips punctuation', () => {
    const m = findPaths('See /tmp/a.png, then ~/x/y and src/lib/a.ts:12. Not https://x.com/a/b or and/or.');
    expect(m.map((x) => [x.path, x.line])).toEqual([
      ['/tmp/a.png', undefined],
      ['~/x/y', undefined],
      ['src/lib/a.ts', 12],
    ]);
  });
});

describe('image sources', () => {
  it('keeps remote, raster data and local paths; drops scripts', () => {
    expect(imageSource('https://a/b.png')?.kind).toBe('remote');
    expect(imageSource('data:image/png;base64,AAAA')?.kind).toBe('data');
    expect(imageSource('data:image/svg+xml;base64,AAAA')).toBeNull();
    expect(imageSource('javascript:alert(1)')).toBeNull();
    expect(imageSource('/tmp/a%20b.png')).toEqual({ kind: 'local', value: '/tmp/a b.png' });
    expect(imageSource('file:///tmp/x.png')).toEqual({ kind: 'local', value: '/tmp/x.png' });
  });
});
