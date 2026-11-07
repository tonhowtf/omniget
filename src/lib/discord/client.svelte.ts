import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import * as dc from "./api";
import type {
  DcChannel,
  DcConnection,
  DcGuild,
  DcMessage,
  DcReadState,
  DcSnapshot,
  DcStatus,
  MessageCreateEvent,
  MessageDeleteBulkEvent,
  MessageDeleteEvent,
  MessageUpdateEvent,
  QrEvent,
  ReadEvent,
} from "./api";
import { mentionsMe, mergeOlder, snowflakeCmp, trimWindow, upsertMessage } from "./format";

export const PAGE = 50;
export const WINDOW = 250;

export type Feed = {
  messages: DcMessage[];
  loading: boolean;
  hasOlder: boolean;
  detached: boolean;
  error: string | null;
  loaded: boolean;
};

const emptyFeed = (): Feed => ({ messages: [], loading: false, hasOlder: true, detached: false, error: null, loaded: false });

export const dcState = $state<{
  status: DcStatus | null;
  booted: boolean;
  bootError: string | null;
  qr: QrEvent | null;
}>({ status: null, booted: false, bootError: null, qr: null });

let snap = $state.raw<DcSnapshot | null>(null);
let feeds = $state.raw<Record<string, Feed>>({});
let viewing: { id: string; atBottom: boolean } | null = null;
let unlisten: UnlistenFn[] = [];
let attached = false;

export const getSnapshot = () => snap;
export const feedOf = (id: string | null | undefined): Feed => (id && feeds[id]) || emptyFeed();
export const connection = (): DcConnection => dcState.status?.connection ?? { state: "offline" };

function setFeed(id: string, patch: Partial<Feed>) {
  const cur = feeds[id] ?? emptyFeed();
  feeds = { ...feeds, [id]: { ...cur, ...patch } };
}

function setConnection(c: DcConnection) {
  if (!dcState.status) return;
  dcState.status = { ...dcState.status, connection: c };
}

function setReadState(channelId: string, rs: DcReadState) {
  if (!snap) return;
  snap = { ...snap, read_states: { ...snap.read_states, [channelId]: rs } };
}

function patchChannel(channelId: string, patch: Partial<DcChannel>) {
  if (!snap) return;
  const di = snap.dms.findIndex((c) => c.id === channelId);
  if (di >= 0) {
    const moved = { ...snap.dms[di], ...patch };
    const rest = snap.dms.filter((_, i) => i !== di);
    snap = { ...snap, dms: patch.last_message_id ? [moved, ...rest] : snap.dms.map((c, i) => (i === di ? moved : c)) };
    return;
  }
  const gi = snap.guilds.findIndex((g) => g.channels.some((c) => c.id === channelId));
  if (gi < 0) return;
  const g = snap.guilds[gi];
  const ng: DcGuild = { ...g, channels: g.channels.map((c) => (c.id === channelId ? { ...c, ...patch } : c)) };
  snap = { ...snap, guilds: snap.guilds.map((x, i) => (i === gi ? ng : x)) };
}

export function findChannel(id: string | null | undefined): DcChannel | null {
  if (!id || !snap) return null;
  return snap.dms.find((c) => c.id === id) ?? snap.guilds.flatMap((g) => g.channels).find((c) => c.id === id) ?? null;
}

function onCreate(p: MessageCreateEvent) {
  const m = p.message;
  const me = snap?.me.id ?? "";
  patchChannel(m.channel_id, { last_message_id: m.id });
  if (!findChannel(m.channel_id) && !m.guild_id && snap) void refreshSnapshot();
  const f = feeds[m.channel_id];
  if (f?.loaded && !f.detached) {
    setFeed(m.channel_id, { messages: trimWindow(upsertMessage(f.messages, m), WINDOW, true), hasOlder: f.hasOlder || f.messages.length >= WINDOW });
  }
  const rs = snap?.read_states[m.channel_id];
  if (m.author.id === me) {
    setReadState(m.channel_id, { last_message_id: m.id, mention_count: 0 });
  } else if (viewing?.id === m.channel_id && viewing.atBottom && document.visibilityState === "visible") {
    markRead(m.channel_id, m.id);
  } else if (mentionsMe(m, me)) {
    setReadState(m.channel_id, { last_message_id: rs?.last_message_id ?? null, mention_count: (rs?.mention_count ?? 0) + 1 });
  }
}

function onUpdate(p: MessageUpdateEvent) {
  const m = p.message;
  const f = feeds[m.channel_id];
  if (!f?.loaded) return;
  const i = f.messages.findIndex((x) => x.id === m.id);
  if (i < 0) return;
  const old = f.messages[i];
  const next: DcMessage = p.partial
    ? { ...old, embeds: m.embeds.length ? m.embeds : old.embeds, edited_timestamp: m.edited_timestamp ?? old.edited_timestamp }
    : { ...m, referenced_message: m.referenced_message ?? old.referenced_message };
  const list = f.messages.slice();
  list[i] = next;
  setFeed(m.channel_id, { messages: list });
}

function dropIds(channelId: string, ids: string[]) {
  const f = feeds[channelId];
  if (!f?.loaded) return;
  const gone = new Set(ids);
  if (!f.messages.some((m) => gone.has(m.id))) return;
  setFeed(channelId, { messages: f.messages.filter((m) => !gone.has(m.id)) });
}

async function refreshSnapshot() {
  try {
    const s = await dc.snapshot();
    if (s) snap = s;
  } catch {
  }
}

export async function attachEvents() {
  if (attached) return;
  attached = true;
  const subs = await Promise.all([
    listen<DcSnapshot>("discord-ready", (e) => {
      snap = e.payload;
      feeds = {};
    }),
    listen<DcConnection>("discord-status", (e) => setConnection(e.payload)),
    listen<QrEvent>("discord-qr", (e) => {
      dcState.qr = e.payload;
      if (e.payload.stage === "done") void refreshStatus();
    }),
    listen<MessageCreateEvent>("discord-message-create", (e) => onCreate(e.payload)),
    listen<MessageUpdateEvent>("discord-message-update", (e) => onUpdate(e.payload)),
    listen<MessageDeleteEvent>("discord-message-delete", (e) => dropIds(e.payload.channel_id, [e.payload.id])),
    listen<MessageDeleteBulkEvent>("discord-message-delete-bulk", (e) => dropIds(e.payload.channel_id, e.payload.ids ?? [])),
    listen<ReadEvent>("discord-read", (e) => setReadState(e.payload.channel_id, { last_message_id: e.payload.message_id, mention_count: e.payload.mention_count ?? 0 })),
    listen<DcGuild>("discord-guild", (e) => {
      if (!snap) return;
      const g = e.payload;
      const has = snap.guilds.some((x) => x.id === g.id);
      snap = { ...snap, guilds: has ? snap.guilds.map((x) => (x.id === g.id ? g : x)) : [...snap.guilds, g] };
    }),
    listen<{ id: string }>("discord-guild-remove", (e) => {
      if (snap) snap = { ...snap, guilds: snap.guilds.filter((g) => g.id !== e.payload.id) };
    }),
    listen<DcChannel>("discord-channel", (e) => {
      if (snap && !snap.dms.some((c) => c.id === e.payload.id)) snap = { ...snap, dms: [e.payload, ...snap.dms] };
    }),
  ]);
  unlisten = subs;
}

export function detachEvents() {
  for (const u of unlisten) u();
  unlisten = [];
  attached = false;
}

export async function refreshStatus(): Promise<DcStatus | null> {
  try {
    dcState.status = await dc.status();
    if (dcState.status.connected_account) await refreshSnapshot();
    else snap = null;
    return dcState.status;
  } catch (e) {
    dcState.bootError = dc.errKey(e);
    return null;
  }
}

export async function boot() {
  dcState.bootError = null;
  const s = await refreshStatus();
  if (s?.active && !s.connected_account) {
    try {
      dcState.status = await dc.connect();
    } catch (e) {
      dcState.bootError = dc.errKey(e);
    }
  }
  await refreshSnapshot();
  dcState.booted = true;
}

export function resetData() {
  snap = null;
  feeds = {};
}

export async function switchTo(accountId: string) {
  snap = null;
  feeds = {};
  dcState.status = await dc.switchAccount(accountId);
  await refreshSnapshot();
}

export async function logoutAccount(accountId: string) {
  const wasLive = dcState.status?.connected_account === accountId;
  dcState.status = await dc.logout(accountId);
  if (wasLive) {
    snap = null;
    feeds = {};
    if (dcState.status.active) {
      dcState.status = await dc.connect();
      await refreshSnapshot();
    }
  }
}

export async function reconnect() {
  try {
    dcState.status = await dc.connect();
  } catch (e) {
    dcState.bootError = dc.errKey(e);
  }
}

export async function loadLatest(channelId: string) {
  const f = feeds[channelId];
  if (f?.loading) return;
  setFeed(channelId, { loading: true, error: null });
  try {
    const page = await dc.messages(channelId, null, PAGE);
    const list = page.slice().sort((a, b) => snowflakeCmp(a.id, b.id));
    setFeed(channelId, { messages: list, loading: false, loaded: true, hasOlder: page.length >= PAGE, detached: false });
  } catch (e) {
    setFeed(channelId, { loading: false, error: dc.errKey(e), loaded: true });
  }
}

export async function loadOlder(channelId: string): Promise<number> {
  const f = feeds[channelId];
  if (!f || f.loading || !f.hasOlder || !f.messages.length) return 0;
  setFeed(channelId, { loading: true, error: null });
  try {
    const page = await dc.messages(channelId, f.messages[0].id, PAGE);
    const cur = feeds[channelId] ?? f;
    const merged = mergeOlder(cur.messages, page);
    const trimmed = merged.length > WINDOW;
    setFeed(channelId, {
      messages: trimWindow(merged, WINDOW, false),
      loading: false,
      hasOlder: page.length >= PAGE,
      detached: cur.detached || trimmed,
    });
    return page.length;
  } catch (e) {
    setFeed(channelId, { loading: false, error: dc.errKey(e) });
    return 0;
  }
}

export function setViewing(id: string | null, atBottom = true) {
  viewing = id ? { id, atBottom } : null;
}

const pendingAck = new Map<string, string>();

export function markRead(channelId: string, messageId: string) {
  const rs = snap?.read_states[channelId];
  if (rs && rs.mention_count === 0 && rs.last_message_id && snowflakeCmp(rs.last_message_id, messageId) >= 0) return;
  setReadState(channelId, { last_message_id: messageId, mention_count: 0 });
  if (pendingAck.get(channelId) === messageId) return;
  pendingAck.set(channelId, messageId);
  void dc.ack(channelId, messageId).catch(() => {}).finally(() => {
    if (pendingAck.get(channelId) === messageId) pendingAck.delete(channelId);
  });
}

export function applySent(m: DcMessage) {
  const f = feeds[m.channel_id];
  if (f?.loaded && !f.detached) setFeed(m.channel_id, { messages: trimWindow(upsertMessage(f.messages, m), WINDOW, true) });
  patchChannel(m.channel_id, { last_message_id: m.id });
  setReadState(m.channel_id, { last_message_id: m.id, mention_count: 0 });
}

export function applyEdited(m: DcMessage) {
  onUpdate({ message: m, partial: false });
}

export function applyDeleted(channelId: string, id: string) {
  dropIds(channelId, [id]);
}
