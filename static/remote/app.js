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
        jobApprovals: "Tarefas esperando você", stopTask: "Recusar e parar",
        answer: "Responder", older: "Carregar anteriores", diff: "Diff", diffAll: "Diff da thread inteira", noDiff: "Sem mudanças.",
        readOnly: "Somente leitura", drive: "Pode pilotar", unpair: "Desparear este aparelho",
        search: "Buscar", noSessions: "Nenhuma sessão.", pending: "pendente(s)", revoked: "Este aparelho foi revogado.",
        turn: "Turno", scope: "Escopo", name: "Nome", paired: "Pareado em", host: "Máquina", version: "Versão",
        status: { approval: "aprovação", input: "pergunta", running: "rodando", error: "erro", idle: "parada" },
        confirmUnpair: "Desparear este aparelho?", custom: "Outra resposta", sent: "Enviado.",
        notes: "Notas", noNotes: "Nenhuma nota.", edit: "Editar", save: "Salvar", cancel: "Cancelar", reload: "Recarregar",
        addPhoto: "Adicionar foto", uploading: "Enviando imagem…", saved: "Salvo.", locked: "Esta nota está travada no computador.",
        conflict: "A nota mudou no computador depois que você abriu. Escolha o que fazer com o seu texto.",
        loadLatest: "Carregar a versão do computador", overwrite: "Salvar a minha por cima",
        discard: "Descartar as mudanças desta nota?", imageMissing: "Imagem indisponível", emptyNote: "Nota vazia.",
      }
    : {
        threads: "Threads", sessions: "Sessions", device: "Device", send: "Send", stop: "Stop",
        message: "Message", notPaired: "This device is not paired.",
        notPairedHint: "In OmniGet, open Central → Remote, turn access on and scan the QR code.",
        pairing: "Pairing…", pairFailed: "Pairing failed: the code expired or was already used. Make a new QR code.",
        offline: "Offline: showing the last saved state.", noThreads: "No threads yet.",
        approvals: "Approvals", approve: "Approve", approveSession: "Approve for session", decline: "Decline",
        jobApprovals: "Tasks waiting for you", stopTask: "Decline and stop",
        answer: "Answer", older: "Load older", diff: "Diff", diffAll: "Whole thread diff", noDiff: "No changes.",
        readOnly: "Read only", drive: "Can drive", unpair: "Unpair this device",
        search: "Search", noSessions: "No sessions.", pending: "pending", revoked: "This device was revoked.",
        turn: "Turn", scope: "Scope", name: "Name", paired: "Paired", host: "Machine", version: "Version",
        status: { approval: "approval", input: "question", running: "running", error: "error", idle: "idle" },
        confirmUnpair: "Unpair this device?", custom: "Other answer", sent: "Sent.",
        notes: "Notes", noNotes: "No notes.", edit: "Edit", save: "Save", cancel: "Cancel", reload: "Reload",
        addPhoto: "Add photo", uploading: "Uploading image…", saved: "Saved.", locked: "This note is locked on the computer.",
        conflict: "The note changed on the computer after you opened it. Choose what to do with your text.",
        loadLatest: "Load the computer's version", overwrite: "Save mine over it",
        discard: "Discard your changes to this note?", imageMissing: "Image unavailable", emptyNote: "Empty note.",
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
    notes: null,
    noteQuery: "",
    jobAsks: [],
  };

  var $ = function (id) { return document.getElementById(id); };

  function lsGet(k) { try { return localStorage.getItem(k); } catch (e) { return null; } }
  function lsSet(k, v) { try { if (v == null) localStorage.removeItem(k); else localStorage.setItem(k, v); } catch (e) { } }
  function jsonGet(k) { try { return JSON.parse(lsGet(k) || "null"); } catch (e) { return null; } }
  function saveThreadPage(id, page) {
    S.threadPages[id] = page;
    var ids = Object.keys(S.threadPages);
    while (ids.length > 5) { delete S.threadPages[ids.shift()]; }
    lsSet(LS_THREADS, JSON.stringify(S.threadPages));
  }

  function h(tag, attrs, kids) {
    var el = document.createElement(tag);
    if (attrs) for (var k in attrs) {
      if (k === "class") el.className = attrs[k];
      else if (k === "text") el.textContent = attrs[k];
      else if (k.slice(0, 2) === "on") el.addEventListener(k.slice(2), attrs[k]);
      else if (attrs[k] != null && attrs[k] !== false) el.setAttribute(k, attrs[k]);
    }
    (kids || []).forEach(function (c) { if (c != null) el.appendChild(typeof c === "string" ? document.createTextNode(c) : c); });
    if (tag === "div" && attrs && attrs.onclick && !el.hasAttribute("role")) {
      el.setAttribute("role", "button");
      if (!el.hasAttribute("tabindex")) el.setAttribute("tabindex", "0");
      el.addEventListener("keydown", function (e) {
        if (e.target === el && (e.key === "Enter" || e.key === " ")) { e.preventDefault(); attrs.onclick.call(el, e); }
      });
    }
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

  function api(path, opts) {
    opts = opts || {};
    var headers = { "Accept": "application/json" };
    var raw = opts.raw !== undefined;
    var sends = raw || opts.body !== undefined;
    if (S.token) headers["Authorization"] = "Bearer " + S.token;
    if (raw) headers["Content-Type"] = opts.type || "application/octet-stream";
    else if (opts.body !== undefined) headers["Content-Type"] = "application/json";
    return fetch("/remote/api/" + path, {
      method: opts.method || (sends ? "POST" : "GET"),
      headers: headers,
      body: raw ? opts.raw : opts.body !== undefined ? JSON.stringify(opts.body) : undefined,
      cache: "no-store",
    }).then(function (res) {
      if (res.status === 401) { forget(T.revoked); throw new Error("unauthorized"); }
      return res.json().catch(function () { return {}; }).then(function (j) {
        if (!res.ok) {
          var err = new Error(j.message || j.error || ("HTTP " + res.status));
          err.code = j.error; err.status = res.status;
          throw err;
        }
        S.online = true;
        return j;
      });
    }, function (e) { S.online = false; throw e; });
  }

  function forget(msg) {
    S.token = null; S.device = null;
    lsSet(LS_TOKEN, null); lsSet(LS_DEVICE, null);
    if (S.ws) { try { S.ws.close(); } catch (e) { } }
    render();
    if (msg) banner(msg, true);
  }

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

  var snapTimer = null, threadTimer = null;
  function loadApprovals() {
    return api("approvals").then(function (list) {
      var before = JSON.stringify(S.jobAsks);
      S.jobAsks = Array.isArray(list) ? list : [];
      if (JSON.stringify(S.jobAsks) !== before && S.route.name === "threads") render();
    }, function () { });
  }
  setInterval(function () { if (S.token && !document.hidden) loadApprovals(); }, 5000);
  function answerJob(id, decision) {
    return api("approvals/" + encodeURIComponent(id), { body: { decision: decision } }).then(function () {
      banner(T.sent); setTimeout(function () { banner(null); }, 1500);
      return loadApprovals();
    }, function (e) { banner(e.message, true); return loadApprovals(); });
  }
  function renderJobAsks(view) {
    if (!S.jobAsks.length) return;
    view.appendChild(h("div", { class: "group-label", text: T.jobApprovals }));
    S.jobAsks.forEach(function (a) {
      var card = h("div", { class: "card" }, [
        h("h3", { text: T.approvals + ": " + (a.tool || "") }),
        h("div", { class: "muted small", text: [a.agent, a.job_id].filter(Boolean).join(" · ") }),
        a.preview ? h("pre", { class: "box", text: String(a.preview) }) : null,
      ]);
      if (canDrive()) {
        card.appendChild(h("div", { class: "btns" }, [
          h("button", { class: "btn primary", text: T.approve, onclick: function () { answerJob(a.id, "accept"); } }),
          h("button", { class: "btn", text: T.approveSession, onclick: function () { answerJob(a.id, "accept_session"); } }),
          h("button", { class: "btn danger", text: T.decline, onclick: function () { answerJob(a.id, "decline"); } }),
          h("button", { class: "btn ghost", text: T.stopTask, onclick: function () { answerJob(a.id, "cancel"); } }),
        ]));
      } else card.appendChild(h("div", { class: "muted small", text: T.readOnly }));
      view.appendChild(card);
    });
  }

  function loadSnapshot() {
    loadApprovals();
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
      ws.onerror = function () { try { ws.close(); } catch (e) { } };
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

  function cmdId() { return "rmt_" + Date.now().toString(36) + Math.random().toString(36).slice(2, 8); }
  function dispatch(cmd) {
    cmd.commandId = cmdId();
    return api("dispatch", { body: cmd }).then(function (r) { stale(null); return r; }, function (e) { banner(e.message, true); throw e; });
  }

  function go(route) {
    S.route = route;
    route.fresh = true;
    if (route.name === "thread") {
      loadThread(route.id);
      if (canDrive()) dispatch({ type: "thread.visit", threadId: route.id }).catch(function () {});
    }
    if (route.name === "sessions" && !S.sessions) loadSessions();
    if (route.name === "session") loadSession(route.tool, route.id, 0);
    if (route.name === "notes") loadNotes();
    if (route.name === "note") loadNote(route);
    render(true);
    $("view").scrollTop = 0;
  }
  $("back").addEventListener("click", function () {
    if (S.route.name === "thread") go({ name: "threads" });
    else if (S.route.name === "session") go({ name: "sessions" });
    else if (S.route.name === "diff") go({ name: "thread", id: S.route.id });
    else if (S.route.name === "note") { if (noteDirty(S.route) && !confirm(T.discard)) return; go({ name: "notes" }); }
  });
  Array.prototype.forEach.call(document.querySelectorAll(".tab"), function (b) {
    b.addEventListener("click", function () { go({ name: b.getAttribute("data-tab") }); });
  });

  function render(force) {
    var view = $("view");
    var r = S.route;
    if (!force && S.token && r.name === "note" && r.editing && r.editor && r.editor.isConnected) return;
    var nearBottom = view.scrollHeight - view.scrollTop - view.clientHeight < 80;
    var prevTop = view.scrollTop;
    view.textContent = "";
    var inner = r.name === "thread" || r.name === "session" || r.name === "diff" || r.name === "note";
    $("tabs").hidden = !S.token || inner;
    $("back").hidden = !S.token || !inner;
    Array.prototype.forEach.call(document.querySelectorAll(".tab"), function (b) {
      b.classList.toggle("active", b.getAttribute("data-tab") === r.name);
    });
    $("tab-threads").textContent = T.threads; $("tab-sessions").textContent = T.sessions; $("tab-device").textContent = T.device;
    $("tab-notes").textContent = T.notes;
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
    else if (r.name === "notes") renderNotes(view);
    else if (r.name === "note") renderNote(view);
    else if (r.name === "device") renderDevice(view);
  }

  function statusPill(status) {
    return h("span", { class: "pill " + (status || "idle"), text: T.status[status] || status || "" });
  }

  function renderThreads(view) {
    $("title").textContent = "OmniGet";
    var snap = S.snapshot;
    var pend = (snap ? (snap.pendingApprovals || []).length + (snap.pendingUserInputs || []).length : 0) + S.jobAsks.length;
    $("subtitle").textContent = pend ? pend + " " + T.pending : (S.device ? S.device.name : "");
    renderJobAsks(view);
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

  function notePath(r) {
    return [r.ws, r.floor, r.id].map(encodeURIComponent).join("/");
  }
  function noteDirty(r) {
    return !!(r && r.editing && r.data && r.draft !== r.data.text);
  }
  function loadNotes() {
    api("notes").then(function (j) {
      S.notes = j.workspaces || [];
      if (S.route.name === "notes") render();
    }, function (e) {
      if (S.route.name === "notes" && !S.notes) { S.notes = { error: e.message }; render(); }
    });
  }
  function renderNotes(view) {
    $("title").textContent = T.notes; $("subtitle").textContent = "";
    var inp = h("input", { class: "text search", placeholder: T.search, value: S.noteQuery, type: "search" });
    inp.addEventListener("input", function () {
      S.noteQuery = inp.value;
      var list = view.querySelector(".note-groups");
      if (list) list.replaceWith(noteGroups());
    });
    view.appendChild(inp);
    if (!S.notes) { view.appendChild(h("div", { class: "empty", text: "…" })); return; }
    if (S.notes.error) { view.appendChild(h("div", { class: "empty", text: S.notes.error })); return; }
    view.appendChild(noteGroups());
  }
  function noteGroups() {
    var q = S.noteQuery.trim().toLowerCase();
    var box = h("div", { class: "note-groups" });
    (S.notes || []).forEach(function (w) {
      var notes = (w.notes || []).filter(function (n) {
        return !q || String(n.title || "").toLowerCase().indexOf(q) >= 0 || String(n.preview || "").toLowerCase().indexOf(q) >= 0;
      });
      if (!notes.length) return;
      box.appendChild(h("div", { class: "group-label", text: w.name || w.id }));
      var list = h("div", { class: "list" });
      notes.forEach(function (n) {
        list.appendChild(h("div", { class: "row", onclick: function () { go({ name: "note", ws: n.ws, floor: n.floor, id: n.id, wsName: w.name, floorName: n.floorName, title: n.title }); } }, [
          h("div", { class: "row-main" }, [
            h("div", { class: "row-title", text: n.title || n.id }),
            h("div", { class: "row-sub", text: [n.floorName, ago(n.updated), n.preview].filter(Boolean).join(" · ") }),
          ]),
        ]));
      });
      box.appendChild(list);
    });
    if (!box.childNodes.length) box.appendChild(h("div", { class: "empty", text: T.noNotes }));
    return box;
  }

  function loadNote(r) {
    return api("notes/" + notePath(r)).then(function (d) {
      if (S.route !== r) return;
      r.data = d; r.conflict = null;
      if (!r.editing) r.draft = d.text;
      render(true);
    }, function (e) {
      if (S.route !== r) return;
      if (!r.data) r.data = { error: e.message };
      else banner(e.message, true);
      render(true);
    });
  }

  var imgCache = {}, imgKeys = [];
  function noteImageUrl(ws, path) {
    var key = ws + "\n" + path;
    if (!imgCache[key]) {
      imgCache[key] = fetch("/remote/api/note-image?ws=" + encodeURIComponent(ws) + "&path=" + encodeURIComponent(path), {
        headers: { "Authorization": "Bearer " + S.token },
      }).then(function (res) {
        if (!res.ok) throw new Error("HTTP " + res.status);
        return res.blob();
      }).then(function (b) { return URL.createObjectURL(b); });
      imgCache[key].catch(function () { delete imgCache[key]; });
      imgKeys.push(key);
      while (imgKeys.length > 60) {
        var old = imgKeys.shift(), p = imgCache[old];
        delete imgCache[old];
        if (p) p.then(function (u) { URL.revokeObjectURL(u); }, function () {});
      }
    }
    return imgCache[key];
  }
  function loadImg(img) {
    noteImageUrl(img.getAttribute("data-ws"), img.getAttribute("data-path")).then(function (u) {
      img.src = u;
    }, function () { missingImg(img); });
  }
  function missingImg(img) {
    if (!img.parentNode) return;
    img.replaceWith(h("span", { class: "note-img-missing", text: T.imageMissing + ": " + (img.getAttribute("data-path") || img.alt || "") }));
  }
  var lazyIO = "IntersectionObserver" in window ? new IntersectionObserver(function (entries) {
    entries.forEach(function (e) {
      if (!e.isIntersecting) return;
      lazyIO.unobserve(e.target);
      loadImg(e.target);
    });
  }, { rootMargin: "400px 0px" }) : null;

  function noteImg(ws, target, alt) {
    if (/^data:image\//i.test(target)) return h("img", { class: "note-img", src: target, alt: alt, decoding: "async" });
    if (/^https?:/i.test(target)) return noteLink(alt || target, target);
    var img = h("img", { class: "note-img", alt: alt, "data-ws": ws, "data-path": target, decoding: "async", loading: "lazy" });
    img.addEventListener("error", function () { missingImg(img); });
    if (lazyIO) lazyIO.observe(img); else loadImg(img);
    return img;
  }
  function noteLink(text, url) {
    if (/^(https?:|mailto:)/i.test(url)) return h("a", { href: url, target: "_blank", rel: "noopener noreferrer", text: text });
    return h("span", { class: "note-path", title: url, text: text });
  }

  var INLINE = /!\[([^\]\n]*)\]\((?:<([^>\n]+)>|([^)\s]+))(?:\s+"[^"\n]*")?\)|\[([^\]\n]+)\]\((?:<([^>\n]+)>|([^)\s]+))(?:\s+"[^"\n]*")?\)|`([^`\n]+)`|\*\*([^*\n]+)\*\*|\*([^*\s][^*\n]*)\*|(https?:\/\/[^\s<>()]+)/g;
  function inline(text, ws) {
    var out = [], re = new RegExp(INLINE.source, "g"), last = 0, m;
    while ((m = re.exec(text))) {
      if (m.index > last) out.push(text.slice(last, m.index));
      if (m[0].charAt(0) === "!") out.push(noteImg(ws, (m[2] || m[3]).trim(), m[1]));
      else if (m[4] != null) out.push(noteLink(m[4], m[5] || m[6]));
      else if (m[7] != null) out.push(h("code", { text: m[7] }));
      else if (m[8] != null) out.push(h("strong", { text: m[8] }));
      else if (m[9] != null) out.push(h("em", { text: m[9] }));
      else out.push(noteLink(m[10], m[10]));
      last = re.lastIndex;
    }
    if (last < text.length) out.push(text.slice(last));
    return out;
  }

  function renderMarkdown(text, ws) {
    var root = h("div", { class: "note-md" });
    var lines = String(text || "").replace(/\r\n?/g, "\n").split("\n");
    var para = [], list = null, quote = null, i = 0;
    function flush() {
      if (para.length) {
        var p = h("p");
        para.forEach(function (l, k) {
          if (k) p.appendChild(h("br"));
          inline(l, ws).forEach(function (c) { p.appendChild(typeof c === "string" ? document.createTextNode(c) : c); });
        });
        root.appendChild(p);
        para = [];
      }
      list = null; quote = null;
    }
    while (i < lines.length) {
      var line = lines[i];
      var fence = line.match(/^\s*(```|~~~)/);
      if (fence) {
        flush();
        var code = [];
        i++;
        while (i < lines.length && lines[i].trim().indexOf(fence[1]) !== 0) { code.push(lines[i]); i++; }
        i++;
        root.appendChild(h("pre", { class: "box", text: code.join("\n") }));
        continue;
      }
      if (!line.trim()) { flush(); i++; continue; }
      if (/^\s*\|/.test(line)) {
        flush();
        var rows = [];
        while (i < lines.length && /^\s*\|/.test(lines[i])) { rows.push(lines[i].trim()); i++; }
        root.appendChild(h("pre", { class: "box", text: rows.join("\n") }));
        continue;
      }
      var hm = line.match(/^(#{1,6})\s+(.*)$/);
      if (hm) {
        flush();
        root.appendChild(h("h" + Math.min(hm[1].length + 1, 6), { class: "note-h" }, inline(hm[2].replace(/\s+#+\s*$/, ""), ws)));
        i++; continue;
      }
      if (/^\s*([-*_])(\s*\1){2,}\s*$/.test(line)) { flush(); root.appendChild(h("hr")); i++; continue; }
      var bq = line.match(/^\s*>\s?(.*)$/);
      if (bq) {
        if (!quote) { flush(); quote = h("blockquote"); root.appendChild(quote); }
        else quote.appendChild(h("br"));
        inline(bq[1], ws).forEach(function (c) { quote.appendChild(typeof c === "string" ? document.createTextNode(c) : c); });
        i++; continue;
      }
      var lm = line.match(/^(\s*)([-*+]|\d+[.)])\s+(.*)$/);
      if (lm) {
        if (para.length || quote) flush();
        var ordered = /\d/.test(lm[2]);
        if (!list || list.ordered !== ordered) { list = { ordered: ordered, el: h(ordered ? "ol" : "ul") }; root.appendChild(list.el); }
        var task = lm[3].match(/^\[([ xX])\]\s+(.*)$/);
        var li = h("li", lm[1].length >= 2 ? { class: "nested" } : null, task ? [task[1] === " " ? "☐ " : "☑ "].concat(inline(task[2], ws)) : inline(lm[3], ws));
        list.el.appendChild(li);
        i++; continue;
      }
      if (list && /^\s{2,}\S/.test(line)) {
        var lastLi = list.el.lastChild;
        lastLi.appendChild(document.createTextNode(" "));
        inline(line.trim(), ws).forEach(function (c) { lastLi.appendChild(typeof c === "string" ? document.createTextNode(c) : c); });
        i++; continue;
      }
      if (list || quote) flush();
      para.push(line.trim());
      i++;
    }
    flush();
    return root;
  }

  function renderNote(view) {
    var r = S.route, d = r.data;
    $("title").textContent = (d && d.title) || r.title || r.id;
    $("subtitle").textContent = [r.wsName, r.floorName].filter(Boolean).join(" · ");
    if (!d) { view.appendChild(h("div", { class: "empty", text: "…" })); return; }
    if (d.error) { view.appendChild(h("div", { class: "empty", text: d.error })); return; }
    if (r.editing) { renderNoteEditor(view, r); return; }
    var bar = h("div", { class: "btns note-bar" }, [
      canDrive() && !d.locked ? h("button", { class: "btn primary", text: T.edit, onclick: function () {
        r.editing = true; r.draft = d.text; r.conflict = null; render(true);
        if (r.editor) r.editor.focus();
      } }) : null,
      h("button", { class: "btn", text: T.reload, onclick: function () { loadNote(r); } }),
    ]);
    view.appendChild(bar);
    if (d.locked) view.appendChild(h("div", { class: "muted small", text: T.locked }));
    else if (!canDrive()) view.appendChild(h("div", { class: "muted small", text: T.readOnly }));
    view.appendChild(d.text.trim() ? h("div", { class: "card" }, [renderMarkdown(d.text, r.ws)]) : h("div", { class: "empty", text: T.emptyNote }));
  }

  function insertAtCursor(r, snippet) {
    var ta = r.editor;
    var s = ta.selectionStart != null ? ta.selectionStart : ta.value.length;
    var e = ta.selectionEnd != null ? ta.selectionEnd : s;
    var before = ta.value.slice(0, s), after = ta.value.slice(e);
    var pre = before && before.charAt(before.length - 1) !== "\n" ? "\n" : "";
    var post = after.charAt(0) === "\n" ? "" : "\n";
    ta.value = before + pre + snippet + post + after;
    var pos = (before + pre + snippet + post).length;
    try { ta.setSelectionRange(pos, pos); } catch (err) { }
    r.draft = ta.value;
  }
  function uploadImages(r, files) {
    var imgs = Array.prototype.filter.call(files || [], function (f) { return !f.type || f.type.indexOf("image/") === 0; });
    if (!imgs.length) return Promise.resolve();
    r.uploads = (r.uploads || 0) + 1;
    syncNoteButtons(r);
    banner(T.uploading);
    var chain = Promise.resolve();
    imgs.forEach(function (f) {
      chain = chain.then(function () {
        var ext = (f.type.split("/")[1] || "png").replace("jpeg", "jpg").replace(/\+.*$/, "");
        var name = f.name && f.name !== "image.png" ? f.name : "photo-" + Date.now() + "." + ext;
        return api("notes/" + notePath(r) + "/image?filename=" + encodeURIComponent(name), { raw: f, type: f.type || "application/octet-stream" }).then(function (j) {
          if (S.route === r && r.editor) insertAtCursor(r, j.markdown);
        });
      });
    });
    return chain.then(function () { banner(null); }, function (e) { banner(e.message, true); }).then(function () {
      r.uploads--;
      syncNoteButtons(r);
    });
  }
  function syncNoteButtons(r) {
    if (r.saveBtn) r.saveBtn.disabled = !!r.uploads || !!r.saving;
  }
  function saveNote(r, base) {
    r.saving = true; syncNoteButtons(r);
    var text = r.editor ? r.editor.value : r.draft;
    r.draft = text;
    return api("notes/" + notePath(r), { method: "PUT", body: { text: text, revision: base } }).then(function (j) {
      r.saving = false;
      if (S.route !== r) return;
      r.data.text = text; r.data.revision = j.revision; r.data.title = j.title;
      r.editing = false; r.conflict = null; r.editor = null;
      banner(T.saved); setTimeout(function () { banner(null); }, 1500);
      render(true);
    }, function (e) {
      r.saving = false; syncNoteButtons(r);
      if (e.code === "CANVAS_NOTE_CONFLICT") {
        r.conflict = String(e.message).split(":").slice(1).join(":").trim();
        render(true);
      } else banner(e.message, true);
    });
  }
  function renderNoteEditor(view, r) {
    var ta = r.editor && r.editor.isConnected === false ? r.editor : h("textarea", { class: "note-editor", rows: "12", spellcheck: "true" });
    if (ta.value !== r.draft) ta.value = r.draft || "";
    r.editor = ta;
    ta.oninput = function () { r.draft = ta.value; };
    ta.onpaste = function (e) {
      var files = e.clipboardData && e.clipboardData.files;
      if (!files || !files.length) return;
      var imgs = Array.prototype.filter.call(files, function (f) { return f.type.indexOf("image/") === 0; });
      if (!imgs.length) return;
      e.preventDefault();
      uploadImages(r, imgs);
    };
    var file = h("input", { type: "file", accept: "image/*", multiple: "", hidden: "" });
    file.addEventListener("change", function () {
      var picked = Array.prototype.slice.call(file.files || []);
      file.value = "";
      uploadImages(r, picked);
    });
    r.saveBtn = h("button", { class: "btn primary", text: T.save, onclick: function () { saveNote(r, r.data.revision); } });
    syncNoteButtons(r);
    view.appendChild(h("div", { class: "btns note-bar" }, [
      r.saveBtn,
      h("label", { class: "btn" }, [T.addPhoto, file]),
      h("button", { class: "btn ghost", text: T.cancel, onclick: function () {
        if (noteDirty(r) && !confirm(T.discard)) return;
        r.editing = false; r.editor = null; r.conflict = null; r.draft = r.data.text; render(true);
      } }),
    ]));
    if (r.conflict != null) {
      view.appendChild(h("div", { class: "card note-conflict" }, [
        h("div", { text: T.conflict }),
        h("div", { class: "btns" }, [
          h("button", { class: "btn", text: T.loadLatest, onclick: function () {
            r.editing = false; r.editor = null; r.conflict = null; loadNote(r);
          } }),
          h("button", { class: "btn danger", text: T.overwrite, onclick: function () { saveNote(r, r.conflict); } }),
        ]),
      ]));
    }
    view.appendChild(ta);
  }

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
