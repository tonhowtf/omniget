//! Ordem de peças para assistir um torrent enquanto ele baixa.
//!
//! Um torrent comum baixa as peças mais raras primeiro, que é ótimo para a
//! saúde do enxame e péssimo para assistir: o arquivo só abre quando termina.
//! Para streaming a ordem é outra, e a troca é explícita — pior para o enxame,
//! utilizável para quem está esperando.
//!
//! A decisão de **qual peça pedir agora** é pura e testável. Falar com o
//! `librqbit` e escrever no disco é I/O.
//!
//! Origem: rqbit, mais o primeira-e-última-peça do qBittorrent.

use serde::Serialize;

/// Quantas peças à frente da posição de leitura mantemos garantidas.
///
/// Oito: o suficiente para o player não parar em cada seek, sem travar o
/// download inteiro esperando uma janela grande demais.
pub const READAHEAD_PIECES: u32 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    /// Sem isto o player nem abre o arquivo.
    Critical,
    /// Dentro da janela de leitura.
    High,
    /// O resto, em ordem.
    Normal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamPlan {
    pub total_pieces: u32,
    /// Peça que a posição atual de reprodução exige.
    pub playhead_piece: u32,
}

/// Prioridade de uma peça, dado o plano.
///
/// Primeira e última peça são críticas porque o container guarda o cabeçalho no
/// início e, em MP4 não otimizado, o índice (`moov`) no fim. Sem as duas o
/// player não consegue nem descobrir a duração, quanto mais fazer seek.
pub fn priority_of(plan: &StreamPlan, piece: u32) -> Priority {
    if plan.total_pieces == 0 {
        return Priority::Normal;
    }
    let last = plan.total_pieces - 1;

    if piece == 0 || piece == last {
        return Priority::Critical;
    }
    if piece >= plan.playhead_piece && piece < plan.playhead_piece + READAHEAD_PIECES {
        return Priority::High;
    }
    Priority::Normal
}

/// Ordem em que pedir as peças que ainda faltam.
///
/// Sequencial a partir do playhead, com as críticas na frente. O que ficou para
/// trás vai para o fim: reproduzir já passou por ali, e só interessa para quem
/// voltar.
pub fn request_order(plan: &StreamPlan, have: &[bool]) -> Vec<u32> {
    if plan.total_pieces == 0 {
        return Vec::new();
    }
    let missing = |p: &u32| have.get(*p as usize).copied() != Some(true);

    let last = plan.total_pieces - 1;
    let mut order: Vec<u32> = Vec::new();
    let mut queued = std::collections::HashSet::new();
    let push = |order: &mut Vec<u32>, queued: &mut std::collections::HashSet<u32>, p: u32| {
        if missing(&p) && queued.insert(p) {
            order.push(p);
        }
    };

    for critica in [0, last] {
        push(&mut order, &mut queued, critica);
    }
    for p in plan.playhead_piece..plan.total_pieces {
        push(&mut order, &mut queued, p);
    }
    for p in 0..plan.playhead_piece {
        push(&mut order, &mut queued, p);
    }
    order
}

/// Dá para começar a assistir?
///
/// Exige as duas peças críticas mais a janela de leitura. Liberar antes faz o
/// player abrir e travar em seguida, que é pior do que esperar — o usuário
/// interpreta como aplicativo quebrado, não como download em andamento.
pub fn can_start_playback(plan: &StreamPlan, have: &[bool]) -> bool {
    if plan.total_pieces == 0 {
        return false;
    }
    let last = plan.total_pieces - 1;
    let tem = |p: u32| have.get(p as usize).copied() == Some(true);

    if !tem(0) || !tem(last) {
        return false;
    }
    let fim_janela = (plan.playhead_piece + READAHEAD_PIECES).min(plan.total_pieces);
    (plan.playhead_piece..fim_janela).all(tem)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(total: u32, playhead: u32) -> StreamPlan {
        StreamPlan {
            total_pieces: total,
            playhead_piece: playhead,
        }
    }

    #[test]
    fn primeira_e_ultima_peca_sao_criticas() {
        // Cabeçalho no início, índice moov no fim. Sem as duas o player não
        // descobre nem a duração.
        let p = plan(100, 50);
        assert_eq!(priority_of(&p, 0), Priority::Critical);
        assert_eq!(priority_of(&p, 99), Priority::Critical);
    }

    #[test]
    fn a_janela_a_frente_do_playhead_e_alta() {
        let p = plan(100, 50);
        assert_eq!(priority_of(&p, 50), Priority::High);
        assert_eq!(priority_of(&p, 57), Priority::High);
        assert_eq!(priority_of(&p, 58), Priority::Normal, "fora da janela");
        assert_eq!(priority_of(&p, 49), Priority::Normal, "ja passou");
    }

    #[test]
    fn pede_as_criticas_antes_de_qualquer_coisa() {
        let p = plan(10, 5);
        let have = vec![false; 10];
        let ordem = request_order(&p, &have);
        assert_eq!(&ordem[..2], &[0, 9], "criticas primeiro: {ordem:?}");
        assert_eq!(ordem[2], 5, "depois sequencial do playhead");
    }

    #[test]
    fn nao_pede_de_novo_o_que_ja_tem() {
        let p = plan(6, 2);
        let mut have = vec![false; 6];
        have[0] = true;
        have[5] = true;
        have[3] = true;
        let ordem = request_order(&p, &have);
        assert_eq!(ordem, vec![2, 4, 1], "{ordem:?}");
    }

    #[test]
    fn o_que_ficou_para_tras_vai_para_o_fim() {
        // Reproduzir já passou por ali; só interessa para quem voltar.
        let p = plan(8, 5);
        let have = vec![false; 8];
        let ordem = request_order(&p, &have);
        let pos_de = |x: u32| ordem.iter().position(|&y| y == x).unwrap();
        assert!(pos_de(6) < pos_de(1), "adiante antes de atras: {ordem:?}");
        assert!(pos_de(6) < pos_de(4));
    }

    #[test]
    fn nao_libera_a_reproducao_cedo_demais() {
        // Abrir e travar em seguida é pior que esperar: o usuário lê como
        // aplicativo quebrado, não como download em andamento.
        let p = plan(100, 0);
        let mut have = vec![false; 100];
        have[0] = true;
        assert!(
            !can_start_playback(&p, &have),
            "so a primeira peca nao basta"
        );

        have[99] = true;
        assert!(!can_start_playback(&p, &have), "falta a janela de leitura");

        for i in 0..READAHEAD_PIECES {
            have[i as usize] = true;
        }
        assert!(can_start_playback(&p, &have));
    }

    #[test]
    fn seek_para_o_meio_exige_a_janela_de_la() {
        // O ponto do seek em arquivo parcial: ter o começo não ajuda em nada
        // se o usuário pulou para o meio.
        let mut have = vec![false; 100];
        have[0] = true;
        have[99] = true;
        for h in have.iter_mut().take(20) {
            *h = true;
        }
        assert!(can_start_playback(&plan(100, 0), &have));
        assert!(
            !can_start_playback(&plan(100, 60), &have),
            "sem a janela do meio"
        );

        for h in have.iter_mut().take(68).skip(60) {
            *h = true;
        }
        assert!(can_start_playback(&plan(100, 60), &have));
    }

    #[test]
    fn torrent_de_uma_peca_so_nao_quebra() {
        let p = plan(1, 0);
        assert_eq!(priority_of(&p, 0), Priority::Critical);
        assert_eq!(request_order(&p, &[false]), vec![0]);
        assert!(can_start_playback(&p, &[true]));
    }

    #[test]
    fn plano_vazio_nao_libera_nem_estoura() {
        let p = plan(0, 0);
        assert!(request_order(&p, &[]).is_empty());
        assert!(!can_start_playback(&p, &[]));
        assert_eq!(priority_of(&p, 0), Priority::Normal);
    }

    #[test]
    fn have_menor_que_o_total_nao_estoura_indice() {
        // Estado parcial vindo do librqbit não pode derrubar o cálculo.
        let p = plan(10, 3);
        let have = vec![true; 2];
        assert!(!can_start_playback(&p, &have));
        assert!(!request_order(&p, &have).is_empty());
    }
}
