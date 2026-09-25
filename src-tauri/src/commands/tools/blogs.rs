//! Comandos da categoria Blogs: arquivo pessoal das newsletters do Substack e
//! exportação das histórias do próprio usuário no Medium.
//!
//! As duas tools leem a sessão do gerenciador de cookies antes de chamar o
//! core — é ela que faz o Substack listar as assinaturas da conta e o Medium
//! entregar os rascunhos. Sem cookie, as duas ainda funcionam no que é
//! público (arquivo de uma publicação, feed do perfil).
