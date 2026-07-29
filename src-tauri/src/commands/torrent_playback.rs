//! Quando da para comecar a assistir um torrent que ainda esta baixando.
//!
//! B46. O modulo `core::torrent_stream` foi escrito supondo que o OmniGet
//! escolheria a ordem das pecas. Ao ligar, apareceu que o `librqbit` **ja faz
//! isso**: `ManagedTorrent::stream(file_id)` devolve um `FileStream` que
//! prioriza as pecas sob o cursor de leitura sozinho.
//!
//! Entao a ordem das pecas fica com o motor — dois escalonadores disputando as
//! mesmas pecas seria pior que um so. O que sobra, e que o motor nao responde, e
//! a pergunta da interface: **ja da para apertar play?** E o `can_start_playback`
//! que decide, e e isso que este modulo expoe.

use crate::core::torrent_stream::{can_start_playback, request_order, StreamPlan};

#[derive(Debug, Clone, serde::Serialize)]
pub struct PlaybackReadiness {
    pub ready: bool,
    /// Pecas que faltam a partir do playhead, em ordem de urgencia.
    pub missing_head: Vec<u32>,
    pub have_count: usize,
    pub total_pieces: usize,
}

/// Responde se ha disco suficiente a partir do playhead para tocar.
///
/// Serve tanto para o play inicial quanto para o seek: o playhead e a peca que
/// a posicao atual exige, e pular para o meio de um torrent incompleto e o caso
/// em que "50% baixado" mais engana.
///
/// Recebe o bitfield em vez de ler do motor porque quem tem a sessao viva e o
/// downloader; passar o estado para ca mantem a decisao testavel e sem depender
/// de um torrent rodando.
#[tauri::command]
pub fn torrent_playback_readiness(
    total_pieces: u32,
    playhead_piece: u32,
    have: Vec<bool>,
) -> PlaybackReadiness {
    let plan = StreamPlan {
        total_pieces,
        playhead_piece,
    };
    let ready = can_start_playback(&plan, &have);
    let missing_head = request_order(&plan, &have);
    PlaybackReadiness {
        ready,
        have_count: have.iter().filter(|h| **h).count(),
        total_pieces: total_pieces as usize,
        missing_head,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn torrent_vazio_nao_esta_pronto() {
        // O caso que importa: apertar play com nada baixado abre um arquivo
        // truncado e o player mostra erro, que e pior do que o botao desativado.
        let r = torrent_playback_readiness(100, 0, vec![false; 100]);
        assert!(!r.ready);
        assert_eq!(r.have_count, 0);
        assert!(!r.missing_head.is_empty(), "tem que dizer o que falta");
    }

    #[test]
    fn com_inicio_e_ultima_peca_da_para_tocar() {
        // A ultima peca entra junto porque em muitos MP4 o `moov` (o indice do
        // container) fica no fim: sem ele o player nao sabe nem a duracao.
        let mut have = vec![false; 100];
        for h in have.iter_mut().take(20) {
            *h = true;
        }
        have[99] = true;
        let r = torrent_playback_readiness(100, 0, have);
        assert!(r.ready);
        assert_eq!(r.have_count, 21);
    }

    #[test]
    fn so_o_inicio_nao_basta_sem_a_ultima_peca() {
        // Escrevi este teste esperando que passasse com so o inicio. Passou o
        // contrario, e o modulo estava certo: sem o fim do container o player
        // abre e falha.
        let mut have = vec![false; 100];
        for h in have.iter_mut().take(20) {
            *h = true;
        }
        let r = torrent_playback_readiness(100, 0, have);
        assert!(!r.ready, "falta a ultima peca, com o indice do container");
    }

    #[test]
    fn baixar_o_fim_primeiro_nao_ajuda_a_comecar() {
        // Torrent comum baixa as pecas mais raras primeiro, que costuma ser
        // qualquer lugar menos o inicio. E exatamente por isso que "50% baixado"
        // nao significa "da para assistir".
        let mut have = vec![false; 100];
        for h in have.iter_mut().skip(50) {
            *h = true;
        }
        let r = torrent_playback_readiness(100, 0, have);
        assert!(!r.ready, "metade baixada, mas a metade errada");
        assert_eq!(r.have_count, 50);
    }
}
