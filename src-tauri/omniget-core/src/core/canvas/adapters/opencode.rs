//! OpenCode has an HTTP server of its own (https://opencode.ai/docs/server/):
//! `POST /session/:id/prompt_async` takes a prompt without a terminal, and the
//! SSE stream `GET /event` says `session.idle` when a turn ends. Delivery and
//! state without the PTY. Spike (C2): the helpers are here; delivery still
//! goes through the PTY until a node knows its server port and session.
