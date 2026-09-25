//! `llm_*` commands: the Local tab — what is running on this machine, the
//! managed `llama-server`, the GGUF catalogue and the OpenAI-compatible bridge
//! switch. Owned by f2-llm-commands.
//!
//! Every one of these touches the network or spawns a process only when the
//! user clicks; nothing here runs on its own.
