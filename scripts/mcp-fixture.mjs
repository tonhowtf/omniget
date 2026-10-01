#!/usr/bin/env node

import http from 'node:http';
import process from 'node:process';

const argv = process.argv.slice(2);
const has = (flag) => argv.includes(flag);
const val = (flag, fallback) => {
  const i = argv.indexOf(flag);
  return i >= 0 && i + 1 < argv.length ? argv[i + 1] : fallback;
};

const opts = {
  http: has('--http'),
  port: Number(val('--port', '0')),
  sse: has('--sse'),
  partialSse: has('--partial-sse'),
  noSession: has('--no-session'),
  noGet: has('--no-get'),
  expireAfter: Number(val('--expire-after', '0')),
  status: Number(val('--status', '0')),
  protocol: val('--protocol', '2025-06-18'),
  crashAfter: Number(val('--crash-after', '0')),
  garbage: has('--garbage'),
  slowMs: Number(val('--slow-ms', '50')),
  hugeLine: Number(val('--huge-line', '0')),
  stringIds: has('--string-ids'),
};

const SERVER_INFO = { name: 'mcp-fixture', version: '1.0.0' };

const TOOLS = [
  {
    name: 'echo',
    description: 'Return the text it was given.',
    inputSchema: {
      type: 'object',
      properties: { text: { type: 'string' } },
      required: ['text'],
    },
  },
  {
    name: 'add',
    description: 'Add two numbers.',
    inputSchema: {
      type: 'object',
      properties: { a: { type: 'number' }, b: { type: 'number' } },
      required: ['a', 'b'],
    },
  },
  {
    name: 'slow',
    description: 'Answer after a delay, to exercise timeouts and cancellation.',
    inputSchema: {
      type: 'object',
      properties: { ms: { type: 'integer' } },
      required: [],
    },
  },
  {
    name: 'fail',
    description: 'Answer with isError: true and no structured content.',
    inputSchema: { type: 'object', properties: {}, required: [] },
  },
  {
    name: 'big',
    description: 'Return kb kilobytes of text, to exercise long lines.',
    inputSchema: {
      type: 'object',
      properties: { kb: { type: 'integer' } },
      required: [],
    },
  },
  {
    name: 'env',
    description: 'Return one environment variable of the server process.',
    inputSchema: {
      type: 'object',
      properties: { name: { type: 'string' } },
      required: ['name'],
    },
  },
  {
    name: 'headers',
    description: 'Return the headers of the last HTTP request the server saw.',
    inputSchema: { type: 'object', properties: {}, required: [] },
  },
];

let lastHeaders = {};

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

let handled = 0;

function textResult(text, structured) {
  const out = { content: [{ type: 'text', text }], isError: false };
  if (structured !== undefined) out.structuredContent = structured;
  return out;
}

async function callTool(name, args) {
  const a = args && typeof args === 'object' ? args : {};
  switch (name) {
    case 'echo':
      return textResult(String(a.text ?? ''), { text: String(a.text ?? '') });
    case 'add': {
      const sum = Number(a.a ?? 0) + Number(a.b ?? 0);
      return textResult(String(sum), { sum });
    }
    case 'slow':
      await sleep(Number(a.ms ?? opts.slowMs));
      return textResult('done', { done: true });
    case 'fail':
      return { content: [{ type: 'text', text: 'tool failed on purpose' }], isError: true };
    case 'big': {
      const kb = Math.max(1, Math.min(4096, Number(a.kb ?? 1)));
      return textResult('x'.repeat(kb * 1024));
    }
    case 'env': {
      const value = process.env[String(a.name ?? '')] ?? '';
      return textResult(value, { name: a.name, value });
    }
    case 'headers':
      return textResult(JSON.stringify(lastHeaders), { headers: lastHeaders });
    default:
      return null;
  }
}

async function handle(msg) {
  const method = typeof msg?.method === 'string' ? msg.method : '';
  const params = msg?.params ?? {};
  if (method.startsWith('notifications/')) return null;
  if (msg?.id === undefined || msg?.id === null) return null;
  const id = opts.stringIds ? String(msg.id) : msg.id;
  handled += 1;
  const ok = (result) => ({ jsonrpc: '2.0', id, result });
  const err = (code, message) => ({ jsonrpc: '2.0', id, error: { code, message } });
  switch (method) {
    case 'initialize':
      return ok({
        protocolVersion:
          typeof params.protocolVersion === 'string' ? params.protocolVersion : opts.protocol,
        capabilities: { tools: { listChanged: false }, resources: {}, prompts: {} },
        serverInfo: SERVER_INFO,
        instructions: 'Fixture server. Not a real tool.',
      });
    case 'ping':
      return ok({});
    case 'tools/list': {
      const result = { tools: TOOLS };
      if (opts.hugeLine > 0) {
        result.filler = 'x'.repeat(opts.hugeLine * 1024 * 1024);
      }
      return ok(result);
    }
    case 'tools/call': {
      const name = typeof params.name === 'string' ? params.name : '';
      const result = await callTool(name, params.arguments);
      return result ? ok(result) : err(-32602, `unknown tool: ${name}`);
    }
    case 'resources/list':
      return ok({
        resources: [
          { uri: 'file:///fixture/readme.txt', name: 'readme.txt', mimeType: 'text/plain' },
        ],
      });
    case 'prompts/list':
      return ok({ prompts: [{ name: 'greet', description: 'Say hello.' }] });
    default:
      return err(-32601, `method not found: ${method}`);
  }
}

function maybeCrash() {
  if (opts.crashAfter > 0 && handled >= opts.crashAfter) {
    process.exit(1);
  }
}

function runStdio() {
  let buf = '';
  let wroteGarbage = false;
  process.stdin.setEncoding('utf8');
  process.stdin.on('data', async (chunk) => {
    buf += chunk;
    let nl;
    while ((nl = buf.indexOf('\n')) >= 0) {
      const line = buf.slice(0, nl).trim();
      buf = buf.slice(nl + 1);
      if (!line) continue;
      let msg;
      try {
        msg = JSON.parse(line);
      } catch {
        process.stderr.write(`fixture: bad JSON line\n`);
        continue;
      }
      const resp = await handle(msg);
      if (opts.garbage && !wroteGarbage) {
        wroteGarbage = true;
        process.stdout.write('not json at all\n');
      }
      if (resp) process.stdout.write(`${JSON.stringify(resp)}\n`);
      maybeCrash();
    }
  });
  process.stdin.on('end', () => {
    process.stdout.write('', () => process.exit(0));
  });
  process.stderr.write('fixture: stdio ready\n');
}

const sessions = new Map();

function newSessionId() {
  return `fixture-${Math.random().toString(36).slice(2)}-${Date.now().toString(36)}`;
}

function sseFrame(data, event, id) {
  let out = '';
  if (event) out += `event: ${event}\n`;
  if (id) out += `id: ${id}\n`;
  out += `data: ${JSON.stringify(data)}\n\n`;
  return out;
}

function readBody(req) {
  return new Promise((resolve) => {
    let body = '';
    req.on('data', (c) => {
      body += c;
    });
    req.on('end', () => resolve(body));
  });
}

async function onPost(req, res, body) {
  lastHeaders = { ...req.headers };
  let msg;
  try {
    msg = JSON.parse(body);
  } catch {
    res.writeHead(400, { 'content-type': 'application/json' });
    res.end(
      JSON.stringify({ jsonrpc: '2.0', id: null, error: { code: -32700, message: 'parse error' } }),
    );
    return;
  }
  const isInitialize = msg?.method === 'initialize';
  const sid = req.headers['mcp-session-id'];

  if (!opts.noSession && !isInitialize) {
    if (!sid) {
      res.writeHead(400).end('missing Mcp-Session-Id');
      return;
    }
    const s = sessions.get(sid);
    if (!s) {
      res.writeHead(404).end('session not found');
      return;
    }
    s.requests += 1;
    if (opts.expireAfter > 0 && s.requests > opts.expireAfter) {
      sessions.delete(sid);
      res.writeHead(404).end('session expired');
      return;
    }
  }

  const resp = await handle(msg);
  if (!resp) {
    res.writeHead(202).end();
    maybeCrash();
    return;
  }

  const headers = { 'cache-control': 'no-store' };
  if (isInitialize && !opts.noSession) {
    const id = newSessionId();
    sessions.set(id, { requests: 0 });
    headers['mcp-session-id'] = id;
  }

  if (opts.status > 0 && !isInitialize) {
    res.writeHead(opts.status, { ...headers, 'content-type': 'application/json' });
    res.end(JSON.stringify({ error: 'forced status' }));
    return;
  }

  if (opts.sse || opts.partialSse) {
    res.writeHead(200, { ...headers, 'content-type': 'text/event-stream' });
    res.write(
      sseFrame(
        { jsonrpc: '2.0', method: 'notifications/message', params: { level: 'info', data: 'hi' } },
        'message',
        '1',
      ),
    );
    if (opts.partialSse) {
      await sleep(20);
      res.end();
      return;
    }
    await sleep(5);
    res.write(sseFrame(resp, 'message', '2'));
    res.end();
    maybeCrash();
    return;
  }

  res.writeHead(200, { ...headers, 'content-type': 'application/json' });
  res.end(JSON.stringify(resp));
  maybeCrash();
}

function onGet(req, res) {
  if (opts.noGet) {
    res.writeHead(405).end();
    return;
  }
  res.writeHead(200, { 'content-type': 'text/event-stream', 'cache-control': 'no-store' });
  res.write(
    sseFrame(
      { jsonrpc: '2.0', method: 'notifications/message', params: { level: 'info', data: 'open' } },
      'message',
      'g1',
    ),
  );
  const timer = setInterval(() => res.write(': keep-alive\n\n'), 250);
  req.on('close', () => clearInterval(timer));
}

function runHttp() {
  const server = http.createServer(async (req, res) => {
    const path = (req.url || '/').split('?')[0];
    if (path !== '/mcp') {
      res.writeHead(404).end();
      return;
    }
    if (req.method === 'POST') {
      const body = await readBody(req);
      await onPost(req, res, body);
      return;
    }
    if (req.method === 'GET') {
      onGet(req, res);
      return;
    }
    if (req.method === 'DELETE') {
      const sid = req.headers['mcp-session-id'];
      if (sid) sessions.delete(sid);
      res.writeHead(204).end();
      return;
    }
    res.writeHead(405).end();
  });
  server.listen(opts.port, '127.0.0.1', () => {
    process.stdout.write(`MCP_FIXTURE_PORT ${server.address().port}\n`);
  });
  process.on('SIGTERM', () => process.exit(0));
}

if (opts.http) runHttp();
else runStdio();
