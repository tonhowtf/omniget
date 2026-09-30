//! The kit contract: every key the mesher can emit, per style, and what each
//! module should look like (the text agent E models from).
//!
//! Key = `layer/case/style/variant`. Layers: `terreo`, `parede`, `telhado`,
//! `porta`, `bay`, `junta`, `torre`, `enfeite`, `obra`, `prop`, `arvore`.
//! `bay` is a facade overlay (janela, vitrine, sacada) placed on the tile face
//! over the plain wall modules, so walls need no bay slots and a bay never
//! multiplies the wall variants.
//!
//! Styled keys use one of [`STYLES`]; shared keys (`obra`, `prop`, `arvore`)
//! use the style `any` and live in their own kit (`kit_keys("any")`).

use serde_json::{json, Value};

pub const STYLES: &[&str] = &["vila-colonial", "vila-praiana", "vila-arida"];
pub const ANY: &str = "any";

/// Tree presets (`prop.kind == "arvore"`, `prop.preset`). Unknown presets
/// fall back to the first one.
pub const TREE_PRESETS: &[&str] = &[
    "oak-estilizada",
    "pinheiro-estilizado",
    "palmeira",
    "coqueiro",
    "ipe",
    "arbusto",
    "cacto",
];

/// Prop kinds. Unknown kinds fall back to `generico`.
pub const PROP_KINDS: &[&str] = &[
    "banco",
    "poste",
    "lixeira",
    "vaso",
    "chafariz",
    "placa",
    "barril",
    "caixote",
    "mesa",
    "carroca",
    "bebedouro",
    "fogueira",
    "estatua",
    "generico",
];

pub const WALL_CASES: &[&str] = &["convexo", "parede", "diagonal", "concavo"];
pub const DOOR_KINDS: &[&str] = &["simples", "dupla", "loja"];
pub const BAY_KINDS: &[&str] = &["janela", "vitrine", "sacada"];

pub const ROOF_PIECES: &[&str] = &[
    "plano-convexo",
    "plano-parede",
    "plano-diagonal",
    "plano-concavo",
    "plano-cheio",
    "duas-aguas-beiral",
    "duas-aguas-rampa",
    "duas-aguas-cumeeira",
    "duas-aguas-oitao-leste",
    "duas-aguas-oitao-oeste",
    "duas-aguas-oitao-topo",
    "duas-aguas-canto-beiral-n",
    "duas-aguas-canto-beiral-o",
    "duas-aguas-concavo-beiral-n",
    "duas-aguas-concavo-beiral-o",
    "duas-aguas-diagonal",
    "quatro-aguas-beiral",
    "quatro-aguas-canto",
    "quatro-aguas-concavo",
    "quatro-aguas-diagonal",
    "quatro-aguas-rampa",
    "quatro-aguas-espigao",
    "quatro-aguas-cumeeira",
    "quatro-aguas-topo",
    "quatro-aguas-pico",
    "conico-canto",
    "conico-beiral",
    "conico-pico",
];

pub const ENFEITES: &[&str] = &[
    "meio-fio",
    "meio-fio-rampa",
    "meio-fio-bueiro",
    "meio-fio-poste",
    "calcada",
    "cerca",
    "muro",
    "sebe",
    "portao",
    "canteiro",
    "vaso",
    "chamine",
    "telha-capa",
    "cata-vento",
    "ponte",
    "ponte-guarda",
];

pub const OBRA: &[&str] = &["andaime", "fundacao"];

/// Variants per layer (and per enfeite piece).
pub fn variants(layer: &str, case: &str) -> u32 {
    match (layer, case) {
        ("terreo", _) | ("parede", _) => 3,
        ("telhado", _) => 1,
        ("porta", _) => 2,
        ("bay", _) => 2,
        ("junta", _) => 1,
        ("torre", _) => 2,
        ("enfeite", "calcada") => 3,
        ("enfeite", _) => 1,
        ("obra", _) => 1,
        ("prop", _) => 2,
        ("arvore", _) => 4,
        _ => 1,
    }
}

/// A known style, or `vila-colonial`.
pub fn norm_style(s: &str) -> &'static str {
    STYLES
        .iter()
        .find(|x| **x == s)
        .copied()
        .unwrap_or(STYLES[0])
}

pub fn norm_tree(p: &str) -> &'static str {
    TREE_PRESETS
        .iter()
        .find(|x| **x == p)
        .copied()
        .unwrap_or(TREE_PRESETS[0])
}

pub fn norm_prop(k: &str) -> &'static str {
    PROP_KINDS
        .iter()
        .find(|x| **x == k)
        .copied()
        .unwrap_or("generico")
}

/// Where a module's origin sits.
/// - `dual`: centre of the dual cell (corner shared by 4 tiles);
/// - `edge`: middle of a tile edge, on the edge line, piece along x, outside
///   toward −z (north) in the canonical orientation;
/// - `tile`: centre of a tile.
fn anchor(layer: &str, case: &str) -> &'static str {
    match layer {
        "terreo" | "parede" | "telhado" | "junta" | "torre" => "dual",
        "porta" | "bay" => "edge",
        "obra" if case == "andaime" => "dual",
        "enfeite" => match case {
            "calcada" | "canteiro" | "vaso" | "ponte" => "tile",
            "chamine" | "telha-capa" | "cata-vento" => "dual",
            _ => "edge",
        },
        _ => "tile",
    }
}

fn slots(layer: &str, case: &str) -> &'static [&'static str] {
    match layer {
        "terreo" | "parede" => &["wall", "trim", "stone"],
        "torre" => &["wall", "stone", "trim"],
        "telhado" => &["roof", "trim", "wall"],
        "porta" => &["door", "trim", "wood", "glass"],
        "bay" => &["glass", "trim", "wood", "cloth", "metal"],
        "junta" => &["trim", "stone", "metal"],
        "obra" => &["wood", "metal", "stone"],
        "arvore" => &["leaf", "trunk"],
        "prop" => &["wood", "metal", "stone", "cloth", "water", "leaf"],
        _ => match case {
            "cerca" | "portao" => &["wood", "trim"],
            "muro" => &["wall", "trim", "stone"],
            "sebe" | "canteiro" | "vaso" => &["leaf", "stone", "cloth"],
            "chamine" | "telha-capa" | "cata-vento" => &["roof", "stone", "metal", "trim"],
            "ponte" | "ponte-guarda" => &["stone", "wood", "metal"],
            _ => &["stone", "street", "plaza", "metal"],
        },
    }
}

fn geometry(layer: &str, case: &str) -> String {
    let wall = |what: &str| {
        match case {
        "convexo" => format!("canto convexo {what}: parede em L ocupando o quadrante SE do cubo 2x2x3 (x 0..1, z 0..1); faces externas nos planos z=0 (para N) e x=0 (para W); 3 m de altura"),
        "parede" => format!("parede reta {what} no plano z=0, de x=-1 a 1, face externa para N (-z), massa do prédio para S; 3 m de altura"),
        "diagonal" => format!("diagonal {what}: quadrantes NW e SE ocupados (dois cantos convexos que se tocam na origem); bloco NW com faces x=0 (z -1..0, para E) e z=0 (x -1..0, para S); bloco SE com faces z=0 (x 0..1, para N) e x=0 (z 0..1, para W)"),
        "concavo" => format!("canto côncavo (interno) {what}: quadrante NW vazio, os outros três ocupados; paredes em z=0 (x -1..0, face para N) e x=0 (z -1..0, face para W)"),
        _ => String::new(),
    }
    };
    match layer {
        "terreo" => format!("{}; com rodapé/embasamento de pedra de 0,4 m (slot stone) e sem aberturas: portas e janelas são peças `porta`/`bay` sobrepostas", wall("do térreo")),
        "parede" => format!("{}; com cimalha/friso no topo (slot trim) e sem aberturas", wall("de andar superior")),
        "torre" => format!("{}; parede de torre com cunhais de pedra e uma seteira estreita no meio da face", wall("da torre")),
        "junta" => format!("costura entre dois prédios vizinhos: pilastra vertical de 0,3x0,3x3 m centrada na origem (a costura passa sempre pela origem) com calha/tubo de descida; o caso diz em que quadrantes está o vizinho ({case})"),
        "porta" => match case {
            "simples" => "porta de uma folha 1,0 x 2,2 m com moldura, no plano z=0, saliente até z=-0,2 (para fora, N); degrau de 0,15 m".into(),
            "dupla" => "portada de duas folhas 1,6 x 2,8 m com verga/frontão, no plano z=0, saliente até z=-0,3; degrau".into(),
            _ => "porta de loja 1,6 x 2,4 m com bandeira de vidro e toldo de lona curto, no plano z=0, saliente até z=-0,6".into(),
        },
        "bay" => match case {
            "janela" => "janela 0,9 x 1,2 m, peitoril a 0,9 m do piso do andar, com moldura e venezianas abertas, saliente até z=-0,15".into(),
            "vitrine" => "vitrine 1,6 x 2,2 m a partir de 0,3 m do piso, caixilho fino e toldo de lona até z=-0,8".into(),
            _ => "sacada: porta-janela 1,0 x 2,2 m e balcão em balanço até z=-0,8 com guarda-corpo de 1,0 m (slot metal ou wood)".into(),
        },
        "telhado" => roof_geometry(case),
        "enfeite" => match case {
            "meio-fio" => "meio-fio de pedra ao longo de x (-1..1), 0,15 m de altura por 0,2 m de largura, face da rua em z=0 (a rua fica para N)".into(),
            "meio-fio-rampa" => "meio-fio rebaixado (rampa de 1,2 m) ao longo de x, rua para N".into(),
            "meio-fio-bueiro" => "meio-fio com boca de lobo/grelha de ferro, rua para N".into(),
            "meio-fio-poste" => "meio-fio com poste de luz de 3,5 m em x=0, z=+0,3 (luminária pendendo para N)".into(),
            "calcada" => "piso de calçada 2x2 m, 0,05 m de espessura, centrado na origem; variante 0 lisa, 1 ondas (pedra portuguesa), 2 xadrez".into(),
            "cerca" => "cerca de madeira de 0,9 m ao longo de x (-1..1) em z=0; o lado de fora do lote fica para N".into(),
            "muro" => "muro baixo caiado de 1,2 m com capa, ao longo de x (-1..1) em z=0".into(),
            "sebe" => "sebe podada de 1,0 m de altura e 0,5 m de largura ao longo de x (-1..1) em z=0".into(),
            "portao" => "portão de 1,4 m de vão com dois pilares, ao longo de x (-1..1) em z=0".into(),
            "canteiro" => "canteiro de flores 1,4 x 1,4 m com borda de pedra de 0,2 m, centrado na origem".into(),
            "vaso" => "vaso grande de 0,6 m com arbusto, centrado na origem".into(),
            "chamine" => "chaminé 0,5 x 0,5 m que sai da cumeeira até 1,2 m acima da origem (a origem é o pico da cumeeira, que corre ao longo de x)".into(),
            "telha-capa" => "capa de cumeeira ornamental (telhas capa sobrepostas, 2 m ao longo de x) assentada no pico, na origem".into(),
            "cata-vento" => "haste de 1,2 m com cata-vento (galo) no pico da cumeeira, na origem".into(),
            "ponte" => "tabuleiro de ponte 2x2 m, 0,3 m de espessura, topo em y=0; rot 0 = travessia N-S; pilar até y=-3".into(),
            _ => "guarda-corpo de ponte de 1,0 m ao longo de x (-1..1) em z=0; a água fica para N".into(),
        },
        "obra" => match case {
            "andaime" => "andaime tubular de 2 m de largura e 3 m de altura ao longo de x, a 0,6 m para fora (z=-0,6) do plano da parede, com tábuas a cada 1,5 m; mesma rotação do caso de parede do nível em obra".into(),
            _ => "laje de fundação 2x2 m, 0,3 m de altura, com esperas de ferro, centrada na origem".into(),
        },
        "arvore" => format!("árvore estilizada `{case}` com copa sólida (~500 triângulos), tronco na origem, 4 a 9 m de altura"),
        _ => format!("objeto `{case}` de rua ou praça, até 1x1 m de base, centrado na origem"),
    }
}

fn roof_geometry(case: &str) -> String {
    let base = "Telhado: origem no topo da parede do último andar (mais `d` passos de 1 m); a superfície é a altura y(x, z) em metros no quadro canônico";
    let body = match case {
        "plano-convexo" => "laje plana no quadrante SE (y=0) com platibanda de 0,4 m nas bordas z=0 (x 0..1) e x=0 (z 0..1)".to_string(),
        "plano-parede" => "laje plana na metade S (z 0..1), platibanda de 0,4 m ao longo de z=0".into(),
        "plano-diagonal" => "laje plana nos quadrantes NW e SE com platibanda nas bordas que dão para NE e SW".into(),
        "plano-concavo" => "laje plana em três quadrantes (NW vazio), platibanda em z=0 (x -1..0) e x=0 (z -1..0)".into(),
        "plano-cheio" => "laje plana 2x2 inteira, sem borda".into(),
        "duas-aguas-beiral" => "beiral: y = 0,5·z em z 0..1 (x -1..1), com aba saliente até z=-0,3".into(),
        "duas-aguas-rampa" => "água inteira: y = 0,5·z no 2x2 (de -0,5 em z=-1 a +0,5 em z=+1), a descida fica para N".into(),
        "duas-aguas-cumeeira" => "cumeeira ao longo de x: y = -0,5·|z|, pico em z=0 com telha capa".into(),
        "duas-aguas-oitao-leste" => "oitão (empena) no plano z=0, face para N, topo da parede y = 0,5·x (sobe para +x); a telha cobre z 0..1 com o mesmo perfil e aba de 0,2 m para N".into(),
        "duas-aguas-oitao-oeste" => "oitão no plano z=0, face para N, topo y = -0,5·x (sobe para -x); telha em z 0..1".into(),
        "duas-aguas-oitao-topo" => "oitão no plano z=0, face para N, topo y = -0,5·|x| (pico em x=0, a cumeeira corre ao longo de z)".into(),
        "duas-aguas-canto-beiral-n" => "quadrante SE: água y = 0,5·z com beiral na borda N (z=0); oitão em x=0 (face W) com topo y = 0,5·z".into(),
        "duas-aguas-canto-beiral-o" => "quadrante SE: água y = 0,5·x com beiral na borda W (x=0); oitão em z=0 (face N) com topo y = 0,5·x".into(),
        "duas-aguas-concavo-beiral-n" => "três quadrantes (NW vazio): água y = 0,5·z; beiral em z=0 (x -1..0); oitão em x=0 (z -1..0, face W) com topo y = 0,5·z".into(),
        "duas-aguas-concavo-beiral-o" => "três quadrantes (NW vazio): água y = 0,5·x; beiral em x=0 (z -1..0); oitão em z=0 (x -1..0, face N) com topo y = 0,5·x".into(),
        "duas-aguas-diagonal" => "quadrantes NW e SE, cada um com sua água: NW desce para S e E, SE desce para N e W (raro)".into(),
        "quatro-aguas-beiral" => "beiral: y = 0,5·z em z 0..1, aba até z=-0,3".into(),
        "quatro-aguas-canto" => "quadrante SE: y = 0,5·min(x, z), espigão na diagonal, beirais em z=0 e x=0".into(),
        "quatro-aguas-concavo" => "três quadrantes (NW vazio): y = 0,5·max(x, z), rincão na diagonal, beirais em z=0 (x<0) e x=0 (z<0)".into(),
        "quatro-aguas-diagonal" => "quadrantes NW e SE com cantos de telhado de quatro águas (raro)".into(),
        "quatro-aguas-rampa" => "água inteira: y = 0,5·z no 2x2, descida para N".into(),
        "quatro-aguas-espigao" => "2x2: y = 0,5·min(x, z), espigão na diagonal (baixo para N e W)".into(),
        "quatro-aguas-cumeeira" => "cumeeira ao longo de x: y = -0,5·|z|".into(),
        "quatro-aguas-topo" => "fim de cumeeira: y = min(0,5·x, -0,5·|z|) (a cumeeira segue para +x/E)".into(),
        "quatro-aguas-pico" => "pico de pirâmide: y = -0,5·max(|x|, |z|)".into(),
        "conico-canto" => "capa de torre, quadrante SE: y = 2·min(x, z) (íngreme), beirais em z=0 e x=0".into(),
        "conico-beiral" => "capa de torre, metade S: y = 2·z, beiral em z=0".into(),
        _ => "ponta de capa de torre: y = -2·max(|x|, |z|) com florão/cata-vento no topo".into(),
    };
    format!("{base}: {body}")
}

struct Spec {
    layer: &'static str,
    case: String,
}

fn styled_specs() -> Vec<Spec> {
    let mut v = Vec::new();
    let mut add = |layer: &'static str, cases: &[&str]| {
        for c in cases {
            v.push(Spec {
                layer,
                case: c.to_string(),
            });
        }
    };
    add("terreo", WALL_CASES);
    add("parede", WALL_CASES);
    add("telhado", ROOF_PIECES);
    add("porta", DOOR_KINDS);
    add("bay", BAY_KINDS);
    add("junta", WALL_CASES);
    add("torre", WALL_CASES);
    add("enfeite", ENFEITES);
    v
}

fn any_specs() -> Vec<Spec> {
    let mut v = Vec::new();
    for c in OBRA {
        v.push(Spec {
            layer: "obra",
            case: c.to_string(),
        });
    }
    for c in PROP_KINDS {
        v.push(Spec {
            layer: "prop",
            case: c.to_string(),
        });
    }
    for c in TREE_PRESETS {
        v.push(Spec {
            layer: "arvore",
            case: c.to_string(),
        });
    }
    v
}

fn keys_of(specs: &[Spec], style: &str) -> Vec<String> {
    let mut out = Vec::new();
    for s in specs {
        for v in 0..variants(s.layer, &s.case) {
            out.push(format!("{}/{}/{}/{}", s.layer, s.case, style, v));
        }
    }
    out
}

/// Every key the mesher can emit for a style: what the kit must provide.
/// `any` lists the shared keys (scaffolding, props, trees). Unknown styles
/// are empty (the mesher maps them to `vila-colonial`).
pub fn kit_keys(style: &str) -> Vec<String> {
    if style == ANY {
        keys_of(&any_specs(), ANY)
    } else if STYLES.contains(&style) {
        keys_of(&styled_specs(), style)
    } else {
        Vec::new()
    }
}

/// All keys of all styles plus `any`.
pub fn kit_keys_all() -> Vec<String> {
    let mut v: Vec<String> = STYLES.iter().flat_map(|s| kit_keys(s)).collect();
    v.extend(kit_keys(ANY));
    v
}

/// The kit catalog as JSON: the module frame, the dual-grid cases and, per
/// key pattern, variants, anchor, palette slots and the expected geometry.
pub fn kit_catalog() -> Value {
    let entry = |s: &Spec, style: &str| {
        json!({
            "pattern": format!("{}/{}/{}/<v>", s.layer, s.case, style),
            "layer": s.layer,
            "case": s.case,
            "variants": variants(s.layer, &s.case),
            "anchor": anchor(s.layer, &s.case),
            "slots": slots(s.layer, &s.case),
            "geometry": geometry(s.layer, &s.case),
        })
    };
    let styled: Vec<Value> = styled_specs()
        .iter()
        .map(|s| entry(s, "<estilo>"))
        .collect();
    let shared: Vec<Value> = any_specs().iter().map(|s| entry(s, ANY)).collect();
    let counts: serde_json::Map<String, Value> = STYLES
        .iter()
        .map(|s| (s.to_string(), json!(kit_keys(s).len())))
        .chain(std::iter::once((
            ANY.to_string(),
            json!(kit_keys(ANY).len()),
        )))
        .collect();
    json!({
        "schema": "omni.city.kit-catalog/v1",
        "key": "camada/caso/estilo/variante",
        "styles": STYLES,
        "shared_style": ANY,
        "module": {
            "cube_m": [2, 3, 2],
            "axes": "x = leste, y = cima, z = sul (metros; three.js)",
            "origin": "anchor `dual`: centro da célula dual (canto comum de 4 tiles), piso em y=0; `edge`: meio da aresta de um tile, peça ao longo de x, lado de fora para N (-z); `tile`: centro do tile",
            "canonical": "a 'fora' de todo caso aponta para N (-z)",
            "rot": "quartos de volta no sentido horário vistos de cima; no three.js, rotation.y = -rot·π/2",
            "materials": "cada malha usa um material `slot:<slot>` com <slot> de PALETTE_SLOTS; sem textura de cor",
            "node_name": "o nó do GLB se chama exatamente a chave",
        },
        "cases": {
            "mask": "NW | NE<<1 | SE<<2 | SW<<3 (tiles em volta do vértice dual)",
            "vazio": {"mask": 0, "occupied": []},
            "convexo": {"mask": 4, "occupied": ["SE"], "outside": ["N", "W"]},
            "parede": {"mask": 12, "occupied": ["SE", "SW"], "outside": ["N"]},
            "diagonal": {"mask": 5, "occupied": ["NW", "SE"], "outside": ["NE", "SW"]},
            "concavo": {"mask": 14, "occupied": ["NE", "SE", "SW"], "outside": ["N", "W"]},
            "cheio": {"mask": 15, "occupied": ["NW", "NE", "SE", "SW"]},
            "note": "paredes `cheio` e `vazio` nunca são emitidas (interior ou nada); telhado usa a máscara 'ocupado no nível k e vazio no k+1'",
        },
        "placement": {
            "wall_y": "base do prédio (menor altura de canto sob o térreo) + k·3000 mm",
            "roof_y": "base + (k+1)·3000 + d·1000 mm (d = passos até o beiral; cônico: d·4000)",
            "tile_y": "média inteira das 4 alturas de canto do tile",
            "edge_y": "média inteira das 2 alturas de canto da aresta",
        },
        "styled": styled,
        "shared": shared,
        "counts": counts,
    })
}
