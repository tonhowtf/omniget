/* OmniGet Remote — a dependency-free PWA served by the desktop app.
 *
 * Auth: the pairing link carries `#pair=<secret>&scope=…`; the secret is
 * traded once at /remote/api/pair for a per-device bearer (localStorage).
 * Live updates: a WebSocket opened with a short ticket from
 * /remote/api/ws-ticket and `after=<sequence>`; any event marks the
 * snapshot (and the open thread) stale and they are re-read, debounced.
 * Offline: the last snapshot and the last opened threads are kept in
 * localStorage and shown with an "offline" banner.
 */
(function () {
  "use strict";

  var LS_TOKEN = "omniget.remote.token";
  var LS_SNAPSHOT = "omniget.remote.snapshot";
  var LS_THREADS = "omniget.remote.threads";
  var LS_DEVICE = "omniget.remote.device";

  var pt = (navigator.language || "").toLowerCase().indexOf("pt") === 0;
  var T = pt
    ? {
        threads: "Threads", sessions: "Sessões", device: "Aparelho", send: "Enviar", stop: "Parar",
        message: "Mensagem", notPaired: "Este aparelho não está pareado.",
        notPairedHint: "No OmniGet, abra Central → Remoto, ligue o acesso e escaneie o QR code.",
        pairing: "Pareando…", pairFailed: "Não deu para parear: o código expirou ou já foi usado. Gere outro QR code.",
        offline: "Sem conexão: mostrando o último estado salvo.", noThreads: "Nenhuma thread ainda.",
        approvals: "Aprovações", approve: "Aprovar", approveSession: "Aprovar na sessão", decline: "Recusar",
        answer: "Responder", older: "Carregar anteriores", diff: "Diff", diffAll: "Diff da thread inteira", noDiff: "Sem mudanças.",
        readOnly: "Somente leitura", drive: "Pode pilotar", unpair: "Desparear este aparelho",
        search: "Buscar", noSessions: "Nenhuma sessão.", pending: "pendente(s)", revoked: "Este aparelho foi revogado.",
        turn: "Turno", scope: "Escopo", name: "Nome", paired: "Pareado em", host: "Máquina", version: "Versão",
        status: { approval: "aprovação", input: "pergunta", running: "rodando", error: "erro", idle: "parada" },
        confirmUnpair: "Desparear este aparelho?", custom: "Outra resposta", sent: "Enviado.",
      }
    : {
        threads: "Threads", sessions: "Sessions", device: "Device", send: "Send", stop: "Stop",
        message: "Message", notPaired: "This device is not paired.",
        notPairedHint: "In OmniGet, open Central → Remote, turn access on and scan the QR code.",
        pairing: "Pairing…", pairFailed: "Pairing failed: the code expired or was already used. Make a new QR code.",
        offline: "Offline: showing the last saved state.", noThreads: "No threads yet.",
        approvals: "Approvals", approve: "Approve", approveSession: "Approve for session", decline: "Decline",
        answer: "Answer", older: "Load older", diff: "Diff", diffAll: "Whole thread diff", noDiff: "No changes.",
        readOnly: "Read only", drive: "Can drive", unpair: "Unpair this device",
        search: "Search", noSessions: "No sessions.", pending: "pending", revoked: "This device was revoked.",
        turn: "Turn", scope: "Scope", name: "Name", paired: "Paired", host: "Machine", version: "Version",
        status: { approval: "approval", input: "question", running: "running", error: "error", idle: "idle" },
        confirmUnpair: "Unpair this device?", custom: "Other answer", sent: "Sent.",
      };

  var S = {
    token: lsGet(LS_TOKEN),
    device: jsonGet(LS_DEVICE),
    snapshot: jsonGet(LS_SNAPSHOT),
    threadPages: jsonGet(LS_THREADS) || {},
    route: { name: "threads" },
    ws: null,
    wsState: "idle",
    retry: 0,
    online: true,
    sessions: null,
    sessionQuery: "",
  };

  var $ = function (id) { return document.getElementById(id); };

  // ── storage ──
  function lsGet(k) { try { return localStorage.getItem(k); } catch (e) { return null; } }
  function lsSet(k, v) { try { if (v == null) localStorage.removeItem(k); else localStorage.setItem(k, v); } catch (e) { /* full */ } }
  function jsonGet(k) { try { return JSON.parse(lsGet(k) || "null"); } catch (e) { return null; } }
  function saveThreadPage(id, page) {
    S.threadPages[id] = page;
    var ids = Object.keys(S.threadPages);
    while (ids.length > 5) { delete S.threadPages[ids.shift()]; }
    lsSet(LS_THREADS, JSON.stringify(S.threadPages));
  }

  // ── dom helpers ──
  function h(tag, attrs, kids) {
    var el = document.createElement(tag);
    if (attrs) for (var k in attrs) {
      if (k === "class") el.className = attrs[k];
      else if (k === "text") el.textContent = attrs[k];
      else if (k.slice(0, 2) === "on") el.addEventListener(k.slice(2), attrs[k]);
      else if (attrs[k] != null && attrs[k] !== false) el.setAttribute(k, attrs[k]);
    }
    (kids || []).forEach(function (c) { if (c != null) el.appendChild(typeof c === "string" ? document.createTextNode(c) : c); });
    return el;
  }
  function banner(text, error) {
    var b = $("banner");
    if (!text) { b.hidden = true; return; }
    b.hidden = false; b.textContent = text; b.className = "banner" + (error ? " error" : "");
  }
  function ago(iso) {
    if (!iso) return "";
    var s = (Date.now() - new Date(iso).getTime()) / 1000;
    if (!isFinite(s)) return "";
    if (s < 60) return Math.max(0, Math.round(s)) + "s";
    if (s < 3600) return Math.round(s / 60) + "m";
    if (s < 86400) return Math.round(s / 3600) + "h";
    return Math.round(s / 86400) + "d";
  }
  function canDrive() { return S.device && S.device.scope === "drive"; }

  // ── api ──
  function api(path, opts) {
    opts = opts || {};
    var headers = { "Accept": "application/json" };
    if (S.token) headers["Authorization"] = "Bearer " + S.token;
    if (opts.body !== undefined) headers["Content-Type"] = "application/json";
    return fetch("/remote/api/" + path, {
      method: opts.method || (opts.body !== undefined ? "POST" : "GET"),
      headers: headers,
      body: opts.body !== undefined ? JSON.stringify(opts.body) : undefined,
      cache: "no-store",
    }).then(function (res) {
      if (res.status === 401) { forget(T.revoked); throw new Error("unauthorized"); }
      return res.json().catch(function () { return {}; }).then(function (j) {
        if (!res.ok) throw new Error(j.message || j.error || ("HTTP " + res.status));
        S.online = true;
        return j;
      });
    }, function (e) { S.online = false; throw e; });
  }

  function forget(msg) {
    S.token = null; S.device = null;
    lsSet(LS_TOKEN, null); lsSet(LS_DEVICE, null);
    if (S.ws) { try { S.ws.close(); } catch (e) { /* closed */ } }
    render();
    if (msg) banner(msg, true);
  }

  // ── pairing ──
  function readPairing() {
    var hash = location.hash.replace(/^#/, "");
    if (hash.indexOf("pair=") < 0) return null;
    var p = new URLSearchParams(hash);
    return p.get("pair");
  }

  function pair(secret) {
    history.replaceState(null, "", "/remote/");
    banner(T.pairing);
    return api("pair", { body: { secret: secret } }).then(function (j) {
      S.token = j.token; S.device = j.device;
      lsSet(LS_TOKEN, j.token); lsSet(LS_DEVICE, JSON.stringify(j.device));
      banner(null);
    }, function () { banner(T.pairFailed, true); });
  }

  // ── data ──
  var snapTimer = null, threadTimer = null;
  function loadSnapshot() {
    return api("snapshot").then(function (snap) {
      S.snapshot = snap;
      lsSet(LS_SNAPSHOT, JSON.stringify(snap));
      if (S.online) banner(null);
      render();
      return snap;
    }, function (e) {
      if (!S.online) { banner(T.offline, true); render(); }
      throw e;
    });
  }
  function loadThread(id, before) {
    var q = "threads/" + encodeURIComponent(id) + "/turns?limit=" + (before ? 20 : 10) + (before ? "&before=" + before : "");
    return api(q).then(function (page) {
      if (before && S.threadPages[id]) {
        var old = S.threadPages[id];
        page.turns = page.turns.concat(old.turns);
      }
      saveThreadPage(id, page);
      if (S.route.name === "thread" && S.route.id === id) render();
    }, function () { if (!S.online) { banner(T.offline, true); render(); } });
  }
  function stale(ev) {
    clearTimeout(snapTimer);
    snapTimer = setTimeout(function () { loadSnapshot().catch(function () {}); }, 350);
    if (S.route.name === "thread" && (!ev || ev.streamId === S.route.id)) {
      clearTimeout(threadTimer);
      threadTimer = setTimeout(function () { loadThread(S.route.id); }, 350);
    }
  }

  // ── live socket ──
  function setConn(state) {
    S.wsState = state;
    var d = $("conn");
    d.className = "dot " + (state === "live" ? "live" : state === "connecting" ? "connecting" : state === "offline" ? "offline" : "");
    d.title = state;
  }
  function connect() {
    if (!S.token || (S.ws && S.ws.readyState <= 1)) return;
    setConn("connecting");
    api("ws-ticket", { body: {} }).then(function (j) {
      var proto = location.protocol === "https:" ? "wss:" : "ws:";
      var after = S.snapshot ? S.snapshot.sequence : "";
      var ws = new WebSocket(proto + "//" + location.host + "/remote/ws?ticket=" + encodeURIComponent(j.ticket) + (after !== "" ? "&after=" + after : ""));
      S.ws = ws;
      ws.onopen = function () { S.retry = 0; setConn("live"); };
      ws.onmessage = function (m) {
        var msg; try { msg = JSON.parse(m.data); } catch (e) { return; }
        if (msg.type === "event") {
          if (S.snapshot) S.snapshot.sequence = Math.max(S.snapshot.sequence || 0, msg.sequence);
          stale(msg.event);
        } else if (msg.type === "reset") { stale(null); }
        else if (msg.type === "revoked") { forget(T.revoked); }
      };
      ws.onclose = function () { S.ws = null; setConn(S.online ? "connecting" : "offline"); schedule(); };
      ws.onerror = function () { try { ws.close(); } catch (e) { /* closed */ } };
    }, function () { setConn("offline"); schedule(); });
  }
  function schedule() {
    if (!S.token || document.hidden) return;
    S.retry = Math.min(S.retry + 1, 6);
    setTimeout(connect, Math.min(15000, 500 * Math.pow(2, S.retry)));
  }
  document.addEventListener("visibilitychange", function () {
    if (!document.hidden && S.token) { loadSnapshot().catch(function () {}); connect(); }
  });

  // ── dispatch ──
  function cmdId() { return "rmt_" + Date.now().toString(36) + Math.random().toString(36).slice(2, 8); }
  function dispatch(cmd) {
    cmd.commandId = cmdId();
    return api("dispatch", { body: cmd }).then(function (r) { stale(null); return r; }, function (e) { banner(e.message, true); throw e; });
  }

  // ── routing ──
  function go(route) {
    S.route = route;
    route.fresh = true;
    if (route.name === "thread") {
      loadThread(route.id);
      if (canDrive()) dispatch({ type: "thread.visit", threadId: route.id }).catch(function () {});
    }
    if (route.name === "sessions" && !S.sessions) loadSessions();
    if (route.name === "session") loadSession(route.tool, route.id, 0);
    render();
    $("view").scrollTop = 0;
  }
  $("back").addEventListener("click", function () {
    if (S.route.name === "thread") go({ name: "threads" });
    else if (S.route.name === "session") go({ name: "sessions" });
    else if (S.route.name === "diff") go({ name: "thread", id: S.route.id });
  });
  Array.prototype.forEach.call(document.querySelectorAll(".tab"), function (b) {
    b.addEventListener("click", function () { go({ name: b.getAttribute("data-tab") }); });
  });

  // ── render ──
  function render() {
    var view = $("view");
    var r = S.route;
    var nearBottom = view.scrollHeight - view.scrollTop - view.clientHeight < 80;
    var prevTop = view.scrollTop;
    view.textContent = "";
    $("tabs").hidden = !S.token || r.name === "thread" || r.name === "session" || r.name === "diff";
    $("back").hidden = !S.token || !(r.name === "thread" || r.name === "session" || r.name === "diff");
    Array.prototype.forEach.call(document.querySelectorAll(".tab"), function (b) {
      b.classList.toggle("active", b.getAttribute("data-tab") === r.name);
    });
    $("tab-threads").textContent = T.threads; $("tab-sessions").textContent = T.sessions; $("tab-device").textContent = T.device;
    $("composer").hidden = true;
    $("drawer").hidden = true;
    if (!S.token) {
      $("title").textContent = "OmniGet"; $("subtitle").textContent = "";
      view.appendChild(h("div", { class: "empty" }, [
        h("img", { src: "/remote/icon.svg", class: "big-icon", alt: "" }),
        h("p", { text: T.notPaired }), h("p", { class: "muted", text: T.notPairedHint }),
      ]));
      return;
    }
    if (r.name === "threads") renderThreads(view);
    else if (r.name === "thread") {
      renderThread(view, r.id);
      view.scrollTop = r.fresh || nearBottom ? view.scrollHeight : prevTop;
      if (S.threadPages[r.id]) r.fresh = false;
    }
    else if (r.name === "diff") renderDiff(view);
    else if (r.name === "sessions") renderSessions(view);
    else if (r.name === "session") {
      renderSession(view);
      view.scrollTop = r.fresh || nearBottom ? view.scrollHeight : prevTop;
      if (r.data) r.fresh = false;
    }
    else if (r.name === "device") renderDevice(view);
  }

  function statusPill(status) {
    return h("span", { class: "pill " + (status || "idle"), text: T.status[status] || status || "" });
  }

  function renderThreads(view) {
    $("title").textContent = "OmniGet";
    var snap = S.snapshot;
    var pend = snap ? (snap.pendingApprovals || []).length + (snap.pendingUserInputs || []).length : 0;
    $("subtitle").textContent = pend ? pend + " " + T.pending : (S.device ? S.device.name : "");
    if (!snap) { view.appendChild(h("div", { class: "empty", text: "…" })); return; }
    var threads = (snap.threads || []).filter(function (t) { return !t.archivedAt; });
    if (!threads.length) { view.appendChild(h("div", { class: "empty", text: T.noThreads })); return; }
    var byProject = {};
    threads.forEach(function (t) { (byProject[t.projectId] = byProject[t.projectId] || []).push(t); });
    var rank = { approval: 0, input: 1, running: 2, error: 3, idle: 4 };
    (snap.projects || []).concat([{ projectId: "__none", title: "" }]).forEach(function (p) {
      var list = byProject[p.projectId];
      if (!list || !list.length) return;
      delete byProject[p.projectId];
      list.sort(function (a, b) {
        if (!!b.pinnedAt !== !!a.pinnedAt) return b.pinnedAt ? 1 : -1;
        var ra = rank[a.status] == null ? 5 : rank[a.status], rb = rank[b.status] == null ? 5 : rank[b.status];
        if (ra !== rb) return ra - rb;
        return String(b.updatedAt || "").localeCompare(String(a.updatedAt || ""));
      });
      view.appendChild(h("div", { class: "group-label", text: p.title || p.projectId }));
      var box = h("div", { class: "list" });
      list.forEach(function (t) {
        box.appendChild(h("div", { class: "row", onclick: function () { go({ name: "thread", id: t.threadId }); } }, [
          h("div", { class: "row-main" }, [
            h("div", { class: "row-title", text: (t.pinnedAt ? "• " : "") + (t.title || t.threadId) }),
            h("div", { class: "row-sub", text: [t.driver, t.model, ago(t.updatedAt)].filter(Boolean).join(" · ") }),
          ]),
          statusPill(t.status),
        ]));
      });
      view.appendChild(box);
    });
    Object.keys(byProject).forEach(function (pid) {
      view.appendChild(h("div", { class: "group-label", text: pid }));
      var box = h("div", { class: "list" });
      byProject[pid].forEach(function (t) {
        box.appendChild(h("div", { class: "row", onclick: function () { go({ name: "thread", id: t.threadId }); } }, [
          h("div", { class: "row-main" }, [h("div", { class: "row-title", text: t.title || t.threadId })]), statusPill(t.status),
        ]));
      });
      view.appendChild(box);
    });
  }

  function threadRow(id) {
    return S.snapshot && (S.snapshot.threads || []).filter(function (t) { return t.threadId === id; })[0];
  }

  function renderMessage(m) {
    var role = m.role || "assistant";
    if (!m.text) return null;
    var el = h("div", { class: "msg " + role });
    el.textContent = m.text;
    if (role === "reasoning" || role === "plan") {
      var d = h("details", {}, [h("summary", { class: "muted small", text: role }), el]);
      return d;
    }
    return el;
  }

  function renderActivity(a) {
    var p = a.payload || {};
    var name = p.toolName || p.itemType || a.kind;
    return h("div", { class: "act" + (a.tone === "error" ? " error" : "") }, [
      h("span", { class: "k", text: String(name || "tool") }),
      h("span", { class: "s", text: a.summary || p.title || "" }),
    ]);
  }

  function renderThread(view, id) {
    var t = threadRow(id);
    var page = S.threadPages[id];
    $("title").textContent = t ? (t.title || id) : id;
    $("subtitle").textContent = t ? [t.driver, t.model, T.status[t.status] || t.status].filter(Boolean).join(" · ") : "";
    if (page && page.hasMore) {
      view.appendChild(h("div", { class: "center" }, [h("button", { class: "btn ghost", text: T.older, onclick: function () {
        var first = page.turns[0]; loadThread(id, first ? first.turn.ordinal : undefined);
      } })]));
    }
    if (!page) { view.appendChild(h("div", { class: "empty", text: "…" })); }
    else {
      (page.turns || []).forEach(function (td) {
        var box = h("div", { class: "turn" });
        box.appendChild(h("div", { class: "turn-head" }, [
          h("span", { text: T.turn + " " + td.turn.ordinal }),
          h("span", { text: td.turn.state }),
          h("button", { class: "btn ghost small", text: T.diff, onclick: function () { openDiff(id, td.turn.ordinal); } }),
        ]));
        var items = [];
        (td.messages || []).forEach(function (m) { items.push({ at: m.createdAt, el: renderMessage(m) }); });
        (td.activities || []).forEach(function (a) { items.push({ at: a.createdAt, el: renderActivity(a) }); });
        items.sort(function (a, b) { return String(a.at).localeCompare(String(b.at)); });
        items.forEach(function (i) { if (i.el) box.appendChild(i.el); });
        if (td.turn.errorMessage) box.appendChild(h("div", { class: "act error", text: td.turn.errorMessage }));
        view.appendChild(box);
      });
      (page.looseMessages || []).forEach(function (m) { var e = renderMessage(m); if (e) view.appendChild(e); });
      view.appendChild(h("div", { class: "center" }, [h("button", { class: "btn ghost", text: T.diffAll, onclick: function () { openDiff(id, null); } })]));
    }
    renderPending(id);
    renderComposer(t);
  }

  function pendingFor(id) {
    var snap = S.snapshot || {};
    return {
      approvals: (snap.pendingApprovals || []).filter(function (a) { return a.threadId === id; }),
      inputs: (snap.pendingUserInputs || []).filter(function (u) { return u.threadId === id; }),
    };
  }

  function renderPending(id) {
    var p = pendingFor(id);
    if (!p.approvals.length && !p.inputs.length) return;
    var body = $("drawer-body");
    body.textContent = "";
    p.approvals.forEach(function (a) {
      var d = a.detail || {};
      var card = h("div", { class: "card" }, [
        h("h3", { text: T.approvals + ": " + (d.toolName || a.requestType) }),
        d.reason ? h("div", { class: "muted small", text: d.reason }) : null,
        d.command ? h("pre", { class: "box", text: d.command }) : null,
        d.paths && d.paths.length ? h("div", { class: "small", text: d.paths.join("\n") }) : null,
        d.diff ? diffBlock(d.diff) : (d.preview ? h("pre", { class: "box", text: String(d.preview) }) : null),
      ]);
      if (canDrive()) {
        var respond = function (decision) {
          return function () { dispatch({ type: "thread.approval.respond", threadId: id, requestId: a.requestId, decision: decision }).catch(function () {}); };
        };
        card.appendChild(h("div", { class: "btns" }, [
          h("button", { class: "btn primary", text: T.approve, onclick: respond("accept") }),
          h("button", { class: "btn", text: T.approveSession, onclick: respond("acceptForSession") }),
          h("button", { class: "btn danger", text: T.decline, onclick: respond("decline") }),
        ]));
      } else card.appendChild(h("div", { class: "muted small", text: T.readOnly }));
      body.appendChild(card);
    });
    p.inputs.forEach(function (u) {
      var picks = {};
      var card = h("div", { class: "card" });
      (u.questions || []).forEach(function (q) {
        card.appendChild(h("h3", { text: q.header || "" }));
        card.appendChild(h("div", { text: q.question || "" }));
        picks[q.id] = [];
        (q.options || []).forEach(function (o) {
          var v = o.value != null ? o.value : o.label;
          var b = h("button", { class: "btn q-opt", type: "button" }, [h("div", { text: o.label }), o.description ? h("div", { class: "muted small", text: o.description }) : null]);
          b.addEventListener("click", function () {
            var cur = picks[q.id];
            if (q.multiSelect) { var i = cur.indexOf(v); if (i >= 0) cur.splice(i, 1); else cur.push(v); }
            else { picks[q.id] = cur = [v]; Array.prototype.forEach.call(b.parentNode.querySelectorAll(".q-opt"), function (x) { x.classList.remove("on"); }); }
            b.classList.toggle("on", cur.indexOf(v) >= 0);
          });
          card.appendChild(b);
        });
        if (q.allowCustomAnswer) {
          var inp = h("input", { class: "text", placeholder: T.custom });
          inp.addEventListener("input", function () { picks[q.id + "::custom"] = inp.value; });
          card.appendChild(inp);
        }
      });
      if (canDrive()) {
        card.appendChild(h("div", { class: "btns" }, [h("button", { class: "btn primary", text: T.answer, onclick: function () {
          var answers = {};
          (u.questions || []).forEach(function (q) {
            var chosen = picks[q.id] || [];
            var custom = (picks[q.id + "::custom"] || "").trim();
            if (custom) chosen = q.multiSelect ? chosen.concat([custom]) : [custom];
            answers[q.id] = q.multiSelect ? chosen : (chosen[0] || "");
          });
          dispatch({ type: "thread.user-input.respond", threadId: id, requestId: u.requestId, answers: answers }).catch(function () {});
        } })]));
      }
      body.appendChild(card);
    });
    $("drawer").hidden = false;
  }

  function renderComposer(t) {
    if (!canDrive() || !t) return;
    var c = $("composer");
    c.hidden = false;
    $("composer-text").placeholder = T.message;
    $("send").textContent = T.send;
    var stop = $("stop");
    stop.textContent = T.stop;
    stop.hidden = !(t.status === "running" || t.activeTurnId);
  }
  $("composer").addEventListener("submit", function (e) {
    e.preventDefault();
    var ta = $("composer-text");
    var text = ta.value.trim();
    if (!text || S.route.name !== "thread") return;
    $("send").disabled = true;
    dispatch({ type: "thread.turn.start", threadId: S.route.id, text: text }).then(function () {
      ta.value = ""; autosize();
    }, function () {}).then(function () { $("send").disabled = false; });
  });
  $("stop").addEventListener("click", function () {
    if (S.route.name !== "thread") return;
    var t = threadRow(S.route.id);
    dispatch({ type: "thread.turn.interrupt", threadId: S.route.id, turnId: t && t.activeTurnId || undefined }).catch(function () {});
  });
  function autosize() { var ta = $("composer-text"); ta.style.height = "auto"; ta.style.height = Math.min(ta.scrollHeight, window.innerHeight * 0.4) + "px"; }
  $("composer-text").addEventListener("input", autosize);

  // ── diff ──
  function diffBlock(patch) {
    var box = h("div", { class: "diff" });
    String(patch).split("\n").slice(0, 4000).forEach(function (line) {
      var c = line.indexOf("+++") === 0 || line.indexOf("---") === 0 || line.indexOf("@@") === 0 || line.indexOf("diff --git") === 0 ? "h"
        : line[0] === "+" ? "a" : line[0] === "-" ? "d" : "";
      box.appendChild(h("div", { class: c, text: line || " " }));
    });
    return box;
  }
  function openDiff(id, turn) {
    S.route = { name: "diff", id: id, turn: turn, data: null };
    render();
    api("threads/" + encodeURIComponent(id) + "/diff" + (turn != null ? "?turn=" + turn : "")).then(function (d) {
      if (S.route.name === "diff" && S.route.id === id) { S.route.data = d; render(); }
    }, function (e) {
      if (S.route.name === "diff") { S.route.data = { error: e.message }; render(); }
    });
  }
  function renderDiff(view) {
    var r = S.route;
    $("title").textContent = T.diff + (r.turn != null ? " · " + T.turn + " " + r.turn : "");
    $("subtitle").textContent = "";
    if (!r.data) { view.appendChild(h("div", { class: "empty", text: "…" })); return; }
    if (r.data.error) { view.appendChild(h("div", { class: "empty", text: r.data.error })); return; }
    var files = r.data.files || [];
    if (!files.length) { view.appendChild(h("div", { class: "empty", text: T.noDiff })); return; }
    view.appendChild(h("div", { class: "muted small" }, [
      h("span", { class: "plus", text: "+" + (r.data.additions || 0) }), " ", h("span", { class: "minus", text: "−" + (r.data.deletions || 0) }),
    ]));
    files.forEach(function (f) {
      view.appendChild(h("details", { class: "diff-file", open: files.length <= 3 ? "" : null }, [
        h("summary", {}, [f.path + " ", h("span", { class: "plus", text: "+" + f.additions }), " ", h("span", { class: "minus", text: "−" + f.deletions })]),
        f.patch ? diffBlock(f.patch) : h("div", { class: "muted small", text: f.binary ? "binary" : "…" }),
      ]));
    });
  }

  // ── sessions of outside CLIs ──
  function loadSessions() {
    var q = "sessions?limit=60" + (S.sessionQuery ? "&query=" + encodeURIComponent(S.sessionQuery) : "");
    api(q).then(function (p) { S.sessions = p.sessions || []; if (S.route.name === "sessions") render(); }, function () {});
  }
  function renderSessions(view) {
    $("title").textContent = T.sessions; $("subtitle").textContent = "";
    var inp = h("input", { class: "text search", placeholder: T.search, value: S.sessionQuery });
    inp.addEventListener("keydown", function (e) { if (e.key === "Enter") { S.sessionQuery = inp.value.trim(); S.sessions = null; loadSessions(); } });
    view.appendChild(inp);
    if (!S.sessions) { view.appendChild(h("div", { class: "empty", text: "…" })); return; }
    if (!S.sessions.length) { view.appendChild(h("div", { class: "empty", text: T.noSessions })); return; }
    var box = h("div", { class: "list" });
    S.sessions.forEach(function (s) {
      var proj = (s.project_path || "").split(/[\\/]/).filter(Boolean).pop() || "";
      box.appendChild(h("div", { class: "row", onclick: function () { go({ name: "session", tool: s.tool, id: s.id, page: 0 }); } }, [
        h("div", { class: "row-main" }, [
          h("div", { class: "row-title", text: s.title || s.id }),
          h("div", { class: "row-sub", text: [s.tool, proj, ago(s.ended || s.started)].filter(Boolean).join(" · ") }),
        ]),
      ]));
    });
    view.appendChild(box);
  }
  function loadSession(tool, id, page) {
    api("sessions/" + encodeURIComponent(tool) + "/" + encodeURIComponent(id) + "?page=" + page + "&pageSize=40").then(function (p) {
      if (S.route.name !== "session" || S.route.id !== id) return;
      if (page > 0 && S.route.data) p.turns = p.turns.concat(S.route.data.turns);
      S.route.data = p; S.route.page = page; render();
    }, function (e) { if (S.route.name === "session") { S.route.data = { error: e.message }; render(); } });
  }
  function renderSession(view) {
    var r = S.route, p = r.data;
    $("title").textContent = p && p.meta ? (p.meta.title || r.id) : r.id;
    $("subtitle").textContent = r.tool;
    if (!p) { view.appendChild(h("div", { class: "empty", text: "…" })); return; }
    if (p.error) { view.appendChild(h("div", { class: "empty", text: p.error })); return; }
    if (p.has_older) view.appendChild(h("div", { class: "center" }, [h("button", { class: "btn ghost", text: T.older, onclick: function () { loadSession(r.tool, r.id, (r.page || 0) + 1); } })]));
    (p.turns || []).forEach(function (t) {
      var role = String(t.role || "").toLowerCase();
      var el = renderMessage({ role: role === "user" ? "user" : role === "assistant" ? "assistant" : "reasoning", text: t.text });
      if (el) view.appendChild(el);
      (t.tool_calls || []).forEach(function (c) {
        view.appendChild(h("div", { class: "act" + (c.status === "error" ? " error" : "") }, [
          h("span", { class: "k", text: c.name_canonical || c.name_raw }), h("span", { class: "s", text: summarizeInput(c.input) }),
        ]));
      });
    });
    if (p.resume_command) view.appendChild(h("pre", { class: "box", text: p.resume_command }));
  }
  function summarizeInput(input) {
    if (!input || typeof input !== "object") return String(input || "");
    var k = input.command || input.file_path || input.path || input.pattern || input.url || input.description || input.prompt;
    return k ? String(k).slice(0, 160) : JSON.stringify(input).slice(0, 160);
  }

  // ── device ──
  function renderDevice(view) {
    $("title").textContent = T.device; $("subtitle").textContent = "";
    var d = S.device || {};
    var kv = h("div", { class: "kv card" }, [
      h("span", { class: "k", text: T.name }), h("span", { text: d.name || "" }),
      h("span", { class: "k", text: T.scope }), h("span", { text: d.scope === "drive" ? T.drive : T.readOnly }),
      h("span", { class: "k", text: T.paired }), h("span", { text: d.createdAt ? new Date(d.createdAt).toLocaleString() : "" }),
    ]);
    view.appendChild(kv);
    api("session").then(function (s) {
      S.device = s.device; lsSet(LS_DEVICE, JSON.stringify(s.device));
      kv.appendChild(h("span", { class: "k", text: T.host })); kv.appendChild(h("span", { text: s.host || location.host }));
      kv.appendChild(h("span", { class: "k", text: T.version })); kv.appendChild(h("span", { text: s.version }));
    }, function () {});
    view.appendChild(h("button", { class: "btn danger", text: T.unpair, onclick: function () {
      if (!confirm(T.confirmUnpair)) return;
      api("logout", { body: {} }).catch(function () {}).then(function () { forget(null); });
    } }));
  }

  // ── boot ──
  if ("serviceWorker" in navigator && window.isSecureContext) {
    navigator.serviceWorker.register("/remote/sw.js", { scope: "/remote/" }).catch(function () {});
  }
  var secret = readPairing();
  var ready = secret ? pair(secret) : Promise.resolve();
  ready.then(function () {
    render();
    if (!S.token) return;
    if (!S.snapshot) S.snapshot = null;
    loadSnapshot().then(connect, function () { setConn("offline"); schedule(); });
  });
})();
