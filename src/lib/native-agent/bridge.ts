import { waitForPaint } from './paint';
import { allowedActions } from './actions';
import 'virtual:omniget-dev-revision';
import { surfaceReadiness } from './readiness';
import { readSemantics } from './semantic';
import { goto } from '$app/navigation';
import { tick } from 'svelte';
import catalog from './catalog.json';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { get } from 'svelte/store';
import { page, navigating } from '$app/stores';

const SELECTOR = 'button,a[href],input,textarea,select,[role],h1,h2,h3,canvas,[data-agent-region],[data-agent-actions]';
const SENSITIVE_ROUTES = /^\/(settings|about\/debug)(\/|$)/;
const MAX_NODES = 10000;
let epoch = '';
let revision = 0;
let fingerprint = '';
let nextRef = 0;
let refs = new WeakMap<Element, string>();
let previousRoute = '';
let targets = new Map<string, Element>();

function nameOf(el: Element): string {
  const labels = el.getAttribute('aria-labelledby')?.split(/\s+/).map(id => document.getElementById(id)?.textContent ?? '').join(' ');
  const label = el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement || el instanceof HTMLSelectElement
    ? [...(el.labels ?? [])].map(l => l.textContent ?? '').join(' ') : '';
  return (el.getAttribute('aria-label') || labels || label || el.getAttribute('placeholder') || el.textContent || '').trim().slice(0, 240);
}

function inspect(args: { cursor?: number; limit?: number; unguarded?: unknown }) {
  const unguarded = args.unguarded === true;
  const route = get(page).url.pathname;
  const blocker = document.querySelector('[data-agent-sensitive], input[type=password], [role=dialog], dialog[open]');
  const sensitive = !unguarded && (SENSITIVE_ROUTES.test(route) || !!blocker);
  const sensitiveReason = !sensitive ? null
    : SENSITIVE_ROUTES.test(route) ? `route ${route} holds credentials`
    : blocker?.matches('input[type=password]') ? 'a password field is on screen'
    : blocker?.matches('[role=dialog], dialog[open]') ? `a dialog is open (${(blocker.getAttribute('aria-label') || blocker.querySelector('h1,h2,h3')?.textContent || 'untitled').trim().slice(0, 80)}); close it (Escape or its close button) and retry`
    : 'the screen marks itself as sensitive';
  if (route !== previousRoute) {
    refs = new WeakMap();
    fingerprint = '';
    previousRoute = route;
  }
  targets.clear();
  const candidates = sensitive ? [] : Array.from(document.querySelectorAll(SELECTOR)).slice(0, MAX_NODES);
  const nodes = candidates.filter(el => {
    const r = el.getBoundingClientRect();
    const style = getComputedStyle(el);
    return r.width > 0 && r.height > 0 && style.display !== 'none' && style.visibility !== 'hidden' && !el.closest('[aria-hidden="true"],[inert]');
  }).map(el => {
    let ref = refs.get(el);
    if (!ref) { ref = `${epoch}:${++nextRef}`; refs.set(el, ref); }
    targets.set(ref, el);
    const r = el.getBoundingClientRect();
    const input = el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement || el instanceof HTMLSelectElement;
    const secret = !unguarded && el instanceof HTMLInputElement && (el.type === 'password' || /password|token|secret|api.?key/i.test(`${el.name} ${el.id} ${el.autocomplete}`));
    const tag = el.tagName.toLowerCase();
    const role = el.getAttribute('role') || ({button:'button',a:'link',input:'textbox',textarea:'textbox',select:'combobox',canvas:'canvas',h1:'heading',h2:'heading',h3:'heading'} as Record<string,string>)[tag] || 'region';
    return {
      ref, role, region: el.getAttribute("data-agent-region"), name: secret ? '[sensitive input]' : nameOf(el), tag,
      value: input && !secret ? el.value.slice(0, 1000) : undefined,
      redacted: secret, focused: el === document.activeElement,
      disabled: el.matches(':disabled,[aria-disabled="true"]'),
      checked: el instanceof HTMLInputElement && ['checkbox','radio'].includes(el.type) ? el.checked : undefined,
      expanded: el.getAttribute('aria-expanded'),
      bounds: {x: Math.round(r.x), y: Math.round(r.y), width: Math.round(r.width), height: Math.round(r.height)},
      scroll: {top: el.scrollTop, left: el.scrollLeft, height: el.scrollHeight, viewportHeight: el.clientHeight},
      inViewport: r.bottom > 0 && r.right > 0 && r.top < innerHeight && r.left < innerWidth,
      actions: secret ? [] : allowedActions(el, unguarded),
    };
  });
  const next = JSON.stringify([route, sensitive, nodes]);
  if (next !== fingerprint) { revision++; fingerprint = next; }
  const cursor = Math.min(MAX_NODES, Math.max(0, args.cursor ?? 0));
  const limit = Math.min(200, Math.max(1, args.limit ?? 100));
  const semantics = sensitive ? {} : readSemantics();
  return {
    schemaVersion: 1, route, uiEpoch: epoch, stateRevision: `${epoch}:${revision}`, observedAt: new Date().toISOString(),
    frontendRevision: (globalThis as typeof globalThis & {__omnigetDev?: {frontendRevision: string}}).__omnigetDev?.frontendRevision ?? null,
    ...surfaceReadiness(sensitive, !!get(navigating), semantics),
    source: 'html-aria', sensitive, sensitiveReason, semantics,
    viewport: {width:innerWidth,height:innerHeight,devicePixelRatio,coordinateSpace:'css-pixels'},
    nodes: nodes.slice(cursor, cursor + limit), nextCursor: cursor + limit < nodes.length ? cursor + limit : null,
    warnings: [
      ...(sensitive ? [`Sensitive surface: inspection and capture blocked (${sensitiveReason}).`] : []),
      ...(candidates.length === MAX_NODES ? ['Inspection limited to 10000 candidates.'] : []),
      ...(nodes.some(n => n.role === 'canvas') && Object.keys(semantics).length === 0 ? ['Canvas pixels are native; semantic canvas adapter not yet available.'] : []),
      'HTML/ARIA projection; not the full operating-system accessibility tree.',
    ],
  };
}

const MAX_REPLY = 7 * 1024 * 1024;
function base64(bytes: Uint8Array): string {
  let text = '';
  for (let i = 0; i < bytes.length; i += 32768) text += String.fromCharCode(...bytes.subarray(i, i + 32768));
  return btoa(text);
}
async function callCommand(command: string, args: Record<string, unknown>) {
  try {
    const out = await invoke(command, args);
    const result = out instanceof ArrayBuffer ? {base64: base64(new Uint8Array(out))}
      : out instanceof Uint8Array ? {base64: base64(out)} : out ?? null;
    if (JSON.stringify(result).length > MAX_REPLY) return {invokeError: 'RESULT_TOO_LARGE: narrow the request'};
    return {result};
  } catch (error) {
    return {invokeError: typeof error === 'string' ? error : error instanceof Error ? error.message : JSON.stringify(error)};
  }
}

type Request = { id: string; epoch: string; operation: string; arguments: Record<string, unknown> };
async function dispatch(operation: string, args: Record<string, unknown>) {
  if (operation === 'inspect') return inspect(args);
  if (operation === 'invoke') return callCommand(String(args.command), (args.args ?? {}) as Record<string, unknown>);
  if (operation === 'navigate') {
    const screen = catalog.find(s => s.screenId === args.screenId);
    if (!screen || screen.windowId !== 'main' || screen.kind === 'development' || screen.navigation !== 'main-route') throw new Error('TARGET_NOT_ACTIONABLE');
    await goto(screen.route);
    await tick();
    await waitForPaint();
    return inspect({unguarded: args.unguarded});
  }
  if (operation !== 'interact') throw new Error('CAPABILITY_UNAVAILABLE');
  const before = inspect({unguarded: args.unguarded});
  if (before.sensitive) throw new Error('PERMISSION_REQUIRED');
  if (before.stateRevision !== args.stateRevision) throw new Error('STALE_TARGET');
  const target = targets.get(String(args.ref));
  if (!(target instanceof HTMLElement) || !target.isConnected) throw new Error('STALE_TARGET');
  const action = String(args.action);
  const allowed = allowedActions(target, args.unguarded === true);
  if (!allowed.includes(action) || target.matches(':disabled,[aria-disabled="true"]')) throw new Error('TARGET_NOT_ACTIONABLE');
  const value = args.value;
  if (action === 'scroll') {
    const delta = Number(value);
    if (!Number.isFinite(delta) || Math.abs(delta) > 4000) throw new Error('INVALID_ARGUMENT');
    target.scrollBy({top:delta,behavior:'instant'});
  } else if (action === 'fill' && (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement)) {
    if (typeof value !== 'string' || value.length > 10000 || (target.type === 'password' && args.unguarded !== true)) throw new Error('INVALID_ARGUMENT');
    target.focus(); target.value = value;
    target.dispatchEvent(new Event('input', {bubbles:true}));
  } else if (action === 'fill' && target instanceof HTMLSelectElement) {
    if (typeof value !== 'string') throw new Error('INVALID_ARGUMENT');
    target.value = value;
    target.dispatchEvent(new Event('change', {bubbles:true}));
  } else if (action === 'click') target.click();
  else throw new Error('CAPABILITY_UNAVAILABLE');
  await tick();
  await waitForPaint();
  return inspect({unguarded: args.unguarded});
}

let activeStop: (() => void) | undefined;
if (import.meta.hot) import.meta.hot.dispose(() => activeStop?.());
export async function startNativeAgentBridge(): Promise<() => void> {
  activeStop?.();
  const ownEpoch = crypto.randomUUID();
  epoch = ownEpoch;
  const stop = await listen<Request>('native-agent-request', async ({payload}) => {
    if (payload.epoch !== ownEpoch) return;
    let result;
    try { result = await dispatch(payload.operation, payload.arguments); }
    catch (error) { result = {error:{code:error instanceof Error ? error.message : 'FRONTEND_NOT_READY'}}; }
    await invoke('native_agent_reply', {id:payload.id,epoch:ownEpoch,payload:result}).catch(() => {});
  });
  try { await invoke('native_agent_register', {epoch:ownEpoch}); }
  catch (error) { stop(); throw error; }
  activeStop = stop;
  return () => {stop(); if (activeStop === stop) activeStop = undefined;};
}
