//! Utilitários de CTF e análise de arquivo: hash, tipo de arquivo, cifras
//! clássicas, XOR, codificações e análise de frequência. Tudo local.

use omniget_core::core::tools::ctf;

use super::err;

#[tauri::command]
pub async fn tool_ctf_hash(input: String, is_file: bool) -> Result<ctf::hashes::HashSet, String> {
    tokio::task::spawn_blocking(move || {
        let data = if is_file {
            std::fs::read(&input).map_err(|e| e.to_string())?
        } else {
            input.into_bytes()
        };
        Ok::<_, String>(ctf::hashes::hash_all(&data))
    })
    .await
    .map_err(err)?
}

#[tauri::command]
pub fn tool_ctf_hmac(opts: ctf::hashes::HmacOptions) -> String {
    ctf::hashes::hmac(opts.data.as_bytes(), opts.key.as_bytes(), &opts.algorithm)
}

#[tauri::command]
pub fn tool_ctf_hash_id(input: String) -> Vec<ctf::hashes::HashGuess> {
    ctf::hashes::identify(&input)
}

#[tauri::command]
pub async fn tool_ctf_magic(
    path: String,
    min_len: Option<usize>,
    limit: Option<usize>,
) -> Result<ctf::magic::MagicReport, String> {
    tokio::task::spawn_blocking(move || {
        ctf::magic::report(&path, min_len.unwrap_or(6), limit.unwrap_or(200))
    })
    .await
    .map_err(err)?
    .map_err(err)
}

#[tauri::command]
pub fn tool_ctf_cipher(opts: ctf::ciphers::CipherOptions) -> String {
    ctf::ciphers::apply(&opts)
}

#[tauri::command]
pub fn tool_ctf_caesar_brute(text: String) -> Vec<ctf::ciphers::CaesarCandidate> {
    ctf::ciphers::caesar_bruteforce(&text)
}

#[tauri::command]
pub async fn tool_ctf_xor(opts: ctf::xor::XorOptions) -> Result<ctf::xor::XorResult, String> {
    tokio::task::spawn_blocking(move || ctf::xor::run(&opts))
        .await
        .map_err(err)?
        .map_err(err)
}

#[tauri::command]
pub fn tool_ctf_encode(opts: ctf::encoding::EncodeOptions) -> ctf::encoding::EncodeResult {
    ctf::encoding::convert(&opts)
}

#[tauri::command]
pub fn tool_ctf_detect(input: String) -> Vec<String> {
    ctf::encoding::detect(&input)
}

#[tauri::command]
pub async fn tool_ctf_freq(
    input: String,
    is_file: bool,
) -> Result<ctf::analysis::FreqReport, String> {
    tokio::task::spawn_blocking(move || {
        let data = if is_file {
            std::fs::read(&input).map_err(|e| e.to_string())?
        } else {
            input.into_bytes()
        };
        Ok::<_, String>(ctf::analysis::analyze(&data, 24))
    })
    .await
    .map_err(err)?
}
