//! Utilitários de CTF e análise de arquivo. Tudo Rust puro e offline: nada
//! aqui dispara ataque contra ninguém — é calculadora de hash, identificador
//! de formato, cifra clássica e análise de frequência, o que se usa para
//! *entender* um arquivo ou um texto que já está na sua mão.

pub mod analysis;
pub mod ciphers;
pub mod encoding;
pub mod hashes;
pub mod magic;
pub mod xor;
