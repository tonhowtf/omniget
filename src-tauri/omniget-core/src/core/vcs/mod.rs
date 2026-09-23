//! Central de agentes: git para as threads. Worktree por thread, checkpoint
//! por turno em refs ocultas, diff do turno e total, restauração e as ações
//! do botão dividido (commit, push, PR). Tudo pelo binário `git` do sistema
//! (e `gh`/`glab`), nunca interativo, com timeout; nada move a branch, o
//! índice ou o stash do usuário.

pub mod actions;
pub mod checkpoint;
pub mod diff;
pub mod repo;
pub mod runner;
pub mod worktree;

pub use runner::{VcsError, VcsResult};

#[cfg(test)]
mod integration_tests;
