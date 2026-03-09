/// 256 Pokémon names for generating human-friendly share codes.
/// 4 unique Pokémon = 32 bits of entropy, enough for LAN transfer identification.
pub static WORDLIST: [&str; 256] = [
    "bulbasaur", "ivysaur", "venusaur", "charmander", "charmeleon", "charizard", "squirtle", "wartortle",
    "blastoise", "caterpie", "butterfree", "weedle", "beedrill", "pidgey", "pidgeot", "rattata",
    "raticate", "spearow", "fearow", "ekans", "arbok", "pikachu", "raichu", "sandshrew",
    "sandslash", "nidoran", "nidorina", "nidoqueen", "nidorino", "nidoking", "clefairy", "clefable",
    "vulpix", "ninetales", "jigglypuff", "wigglytuff", "zubat", "golbat", "oddish", "gloom",
    "vileplume", "paras", "parasect", "venonat", "venomoth", "diglett", "dugtrio", "meowth",
    "persian", "psyduck", "golduck", "mankey", "primeape", "growlithe", "arcanine", "poliwag",
    "poliwhirl", "poliwrath", "abra", "kadabra", "alakazam", "machop", "machoke", "machamp",
    "bellsprout", "weepinbell", "victreebel", "tentacool", "tentacruel", "geodude", "graveler", "golem",
    "ponyta", "rapidash", "slowpoke", "slowbro", "magnemite", "magneton", "farfetchd", "doduo",
    "dodrio", "seel", "dewgong", "grimer", "muk", "shellder", "cloyster", "gastly",
    "haunter", "gengar", "onix", "drowzee", "hypno", "krabby", "kingler", "voltorb",
    "electrode", "exeggcute", "exeggutor", "cubone", "marowak", "hitmonlee", "hitmonchan", "lickitung",
    "koffing", "weezing", "rhyhorn", "rhydon", "chansey", "tangela", "kangaskhan", "horsea",
    "seadra", "goldeen", "seaking", "staryu", "starmie", "scyther", "jynx", "electabuzz",
    "magmar", "pinsir", "tauros", "magikarp", "gyarados", "lapras", "ditto", "eevee",
    "vaporeon", "jolteon", "flareon", "porygon", "omanyte", "omastar", "kabuto", "kabutops",
    "aerodactyl", "snorlax", "articuno", "zapdos", "moltres", "dratini", "dragonair", "dragonite",
    "mewtwo", "mew", "chikorita", "bayleef", "meganium", "cyndaquil", "quilava", "typhlosion",
    "totodile", "croconaw", "feraligatr", "sentret", "furret", "hoothoot", "noctowl", "ledyba",
    "spinarak", "ariados", "crobat", "chinchou", "lanturn", "pichu", "cleffa", "igglybuff",
    "togepi", "togetic", "natu", "xatu", "mareep", "flaaffy", "ampharos", "bellossom",
    "marill", "azumarill", "sudowoodo", "politoed", "hoppip", "skiploom", "jumpluff", "aipom",
    "sunkern", "sunflora", "yanma", "wooper", "quagsire", "espeon", "umbreon", "murkrow",
    "slowking", "misdreavus", "unown", "wobbuffet", "girafarig", "pineco", "forretress", "dunsparce",
    "gligar", "steelix", "snubbull", "granbull", "qwilfish", "scizor", "shuckle", "heracross",
    "sneasel", "teddiursa", "ursaring", "slugma", "magcargo", "swinub", "piloswine", "corsola",
    "remoraid", "octillery", "delibird", "mantine", "skarmory", "houndour", "houndoom", "kingdra",
    "phanpy", "donphan", "porygon2", "stantler", "smeargle", "tyrogue", "hitmontop", "smoochum",
    "elekid", "magby", "miltank", "blissey", "raikou", "entei", "suicune", "larvitar",
    "pupitar", "tyranitar", "lugia", "celebi", "treecko", "grovyle", "sceptile", "torchic",
    "combusken", "blaziken", "mudkip", "marshtomp", "swampert", "ralts", "gardevoir", "absol",
];

/// Generate a 4-word share code from random bytes.
/// Always picks 4 **different** Pokémon (no repeats).
pub fn generate_code() -> String {
    use rand::RngExt;
    let mut rng = rand::rng();
    let mut indices = Vec::with_capacity(4);

    while indices.len() < 4 {
        let idx: usize = rng.random_range(0..256);
        if !indices.contains(&idx) {
            indices.push(idx);
        }
    }

    indices
        .iter()
        .map(|&i| WORDLIST[i])
        .collect::<Vec<_>>()
        .join("-")
}

/// Look up a word's index in the wordlist. Returns None if not found.
pub fn word_index(word: &str) -> Option<u8> {
    let lower = word.to_lowercase();
    WORDLIST
        .iter()
        .position(|w| *w == lower)
        .map(|i| i as u8)
}

pub struct ParsedCode {
    pub words: String,
    pub remote_endpoint: Option<std::net::SocketAddr>,
}

pub fn parse_code(s: &str) -> Option<ParsedCode> {
    let (word_part, endpoint) = if let Some((words, ep_str)) = s.split_once('@') {
        (words, ep_str.parse().ok())
    } else {
        (s, None)
    };

    if !validate_words(word_part) {
        return None;
    }

    Some(ParsedCode {
        words: word_part.to_string(),
        remote_endpoint: endpoint,
    })
}

pub fn is_valid_code(s: &str) -> bool {
    parse_code(s).is_some()
}

fn validate_words(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 4 {
        return false;
    }
    let mut seen = Vec::with_capacity(4);
    for word in &parts {
        match word_index(word) {
            Some(idx) => {
                if seen.contains(&idx) {
                    return false;
                }
                seen.push(idx);
            }
            None => return false,
        }
    }
    true
}
