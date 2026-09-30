//! A hand-made blueprint that exercises every mesher path (row houses with
//! seams, L plan, U plan with tower, hip, gable on both axes, flat roof,
//! works at 0 % and 50 %, fences, plaza, river with bridges, forest, beach).
//! Used by tests, by `examples/dump_region.rs` when the solver is not ready,
//! and as a fixture for the renderer.

use crate::hash::canonical_hash;
use crate::types::*;

struct B<'a> {
    id: &'a str,
    lot: [i32; 4],
    front: Dir,
    style: &'a str,
    palette: &'a str,
    use_: &'a str,
    /// Ground plan cells.
    plan: Vec<[i32; 2]>,
    floors: u8,
    roof: &'a str,
    axis: Axis,
    door: [i32; 2],
    tower: Option<(Vec<[i32; 2]>, u8)>,
    progress: u8,
    balcony: bool,
}

fn rect(x: i32, y: i32, w: i32, h: i32) -> Vec<[i32; 2]> {
    let mut v = Vec::new();
    for yy in y..y + h {
        for xx in x..x + w {
            v.push([xx, yy]);
        }
    }
    v
}

pub fn sample_blueprint() -> Blueprint {
    let n = REGION_TILES;
    let mut surf = vec![Surface::Grass; (n * n) as usize];
    let set = |s: &mut Vec<Surface>, x: i32, y: i32, v: Surface| {
        if x >= 0 && y >= 0 && x < n && y < n {
            s[tile_index(x, y)] = v;
        }
    };
    for y in 0..10 {
        for x in 50..n {
            set(&mut surf, x, y, Surface::Field);
        }
    }
    for y in 60..n {
        for x in 0..n {
            set(
                &mut surf,
                x,
                y,
                if x <= 40 {
                    Surface::Forest
                } else {
                    Surface::Sand
                },
            );
        }
    }
    for y in 56..60 {
        for x in 0..n {
            set(&mut surf, x, y, Surface::Water);
        }
    }
    let street = |s: &mut Vec<Surface>, x: i32, y: i32| {
        let v = if s[tile_index(x, y)] == Surface::Water {
            Surface::Bridge
        } else {
            Surface::Street
        };
        s[tile_index(x, y)] = v;
    };
    for x in 0..n {
        street(&mut surf, x, 30);
        street(&mut surf, x, 31);
    }
    for y in 0..n {
        for x in [20, 21, 44, 45] {
            street(&mut surf, x, y);
        }
    }
    for x in 0..20 {
        street(&mut surf, x, 12);
    }
    let side = |s: &mut Vec<Surface>, x: i32, y: i32| {
        if s[tile_index(x, y)] == Surface::Grass {
            s[tile_index(x, y)] = Surface::Sidewalk;
        }
    };
    for x in 0..n {
        side(&mut surf, x, 29);
        side(&mut surf, x, 32);
    }
    for y in 0..56 {
        for x in [19, 22, 43, 46] {
            side(&mut surf, x, y);
        }
    }
    for y in 15..28 {
        for x in 24..38 {
            if surf[tile_index(x, y)] == Surface::Grass {
                surf[tile_index(x, y)] = Surface::Plaza;
            }
        }
    }

    let specs = vec![
        B {
            id: "sobrado-1",
            lot: [0, 24, 4, 5],
            front: Dir::S,
            style: "vila-colonial",
            palette: "cal-e-terracota",
            use_: "loja-sobrado",
            plan: rect(0, 25, 4, 4),
            floors: 2,
            roof: "duas-aguas",
            axis: Axis::X,
            door: [1, 28],
            tower: None,
            progress: 100,
            balcony: true,
        },
        B {
            id: "sobrado-2",
            lot: [4, 24, 4, 5],
            front: Dir::S,
            style: "vila-colonial",
            palette: "azul-anil",
            use_: "loja-sobrado",
            plan: rect(4, 25, 4, 4),
            floors: 3,
            roof: "duas-aguas",
            axis: Axis::X,
            door: [5, 28],
            tower: None,
            progress: 100,
            balcony: true,
        },
        B {
            id: "sobrado-3",
            lot: [8, 24, 4, 5],
            front: Dir::S,
            style: "vila-colonial",
            palette: "cal-e-terracota",
            use_: "loja-sobrado",
            plan: rect(8, 25, 4, 4),
            floors: 2,
            roof: "duas-aguas",
            axis: Axis::X,
            door: [9, 28],
            tower: None,
            progress: 100,
            balcony: false,
        },
        B {
            id: "sobrado-4",
            lot: [12, 24, 4, 5],
            front: Dir::S,
            style: "vila-colonial",
            palette: "musgo",
            use_: "loja-sobrado",
            plan: rect(12, 25, 4, 4),
            floors: 1,
            roof: "duas-aguas",
            axis: Axis::X,
            door: [13, 28],
            tower: None,
            progress: 100,
            balcony: false,
        },
        B {
            id: "prefeitura",
            lot: [24, 4, 12, 11],
            front: Dir::S,
            style: "vila-colonial",
            palette: "cal-e-terracota",
            use_: "civico",
            plan: rect(25, 6, 10, 8)
                .into_iter()
                .filter(|c| !(c[1] <= 9 && c[0] >= 28 && c[0] <= 31))
                .collect(),
            floors: 3,
            roof: "quatro-aguas",
            axis: Axis::X,
            door: [29, 13],
            tower: Some((vec![[25, 12], [26, 12], [25, 13], [26, 13]], 3)),
            progress: 100,
            balcony: true,
        },
        B {
            id: "casa-do-tonho",
            lot: [0, 33, 8, 9],
            front: Dir::N,
            style: "vila-praiana",
            palette: "azul-anil",
            use_: "casa",
            plan: rect(1, 34, 6, 7)
                .into_iter()
                .filter(|c| !(c[1] >= 37 && c[0] >= 4))
                .collect(),
            floors: 2,
            roof: "duas-aguas",
            axis: Axis::Y,
            door: [3, 34],
            tower: None,
            progress: 100,
            balcony: true,
        },
        B {
            id: "obra-meio",
            lot: [8, 33, 8, 9],
            front: Dir::N,
            style: "vila-praiana",
            palette: "cal-e-terracota",
            use_: "casa",
            plan: rect(9, 34, 6, 6),
            floors: 2,
            roof: "quatro-aguas",
            axis: Axis::X,
            door: [11, 34],
            tower: None,
            progress: 50,
            balcony: false,
        },
        B {
            id: "obra-inicio",
            lot: [23, 33, 8, 8],
            front: Dir::N,
            style: "vila-praiana",
            palette: "vila-praiana",
            use_: "casa",
            plan: rect(24, 34, 6, 5),
            floors: 1,
            roof: "duas-aguas",
            axis: Axis::X,
            door: [26, 34],
            tower: None,
            progress: 0,
            balcony: false,
        },
        B {
            id: "galpao",
            lot: [32, 33, 10, 9],
            front: Dir::N,
            style: "vila-arida",
            palette: "adobe-ocre",
            use_: "galpao",
            plan: rect(33, 34, 8, 7),
            floors: 1,
            roof: "plano",
            axis: Axis::X,
            door: [36, 34],
            tower: None,
            progress: 100,
            balcony: false,
        },
        B {
            id: "templo",
            lot: [47, 34, 8, 8],
            front: Dir::W,
            style: "vila-colonial",
            palette: "cal-e-terracota",
            use_: "templo",
            plan: rect(48, 35, 7, 6),
            floors: 2,
            roof: "duas-aguas",
            axis: Axis::X,
            door: [48, 37],
            tower: Some((vec![[48, 35]], 2)),
            progress: 100,
            balcony: false,
        },
        B {
            id: "loja-esquina",
            lot: [55, 33, 9, 8],
            front: Dir::N,
            style: "vila-arida",
            palette: "adobe-ocre",
            use_: "loja-sobrado",
            plan: rect(56, 34, 7, 6),
            floors: 3,
            roof: "quatro-aguas",
            axis: Axis::X,
            door: [59, 34],
            tower: None,
            progress: 100,
            balcony: true,
        },
        B {
            id: "casa-leste",
            lot: [47, 12, 8, 10],
            front: Dir::W,
            style: "vila-praiana",
            palette: "vila-praiana",
            use_: "casa",
            plan: rect(48, 13, 6, 8),
            floors: 2,
            roof: "duas-aguas",
            axis: Axis::Y,
            door: [48, 16],
            tower: None,
            progress: 100,
            balcony: false,
        },
        B {
            id: "chacara",
            lot: [55, 12, 8, 10],
            front: Dir::N,
            style: "vila-arida",
            palette: "musgo",
            use_: "casa",
            plan: rect(56, 13, 6, 7),
            floors: 1,
            roof: "quatro-aguas",
            axis: Axis::X,
            door: [58, 13],
            tower: None,
            progress: 100,
            balcony: false,
        },
        B {
            id: "casa-beco",
            lot: [2, 4, 8, 8],
            front: Dir::S,
            style: "vila-colonial",
            palette: "musgo",
            use_: "casa",
            plan: rect(3, 5, 6, 6),
            floors: 2,
            roof: "duas-aguas",
            axis: Axis::X,
            door: [5, 10],
            tower: None,
            progress: 100,
            balcony: false,
        },
    ];

    // Heights: a gentle slope, a river valley, flat lots.
    let mut h = vec![0i32; ((n + 1) * (n + 1)) as usize];
    for vy in 0..=n {
        for vx in 0..=n {
            let dip = if (56..=60).contains(&vy) { 1500 } else { 0 };
            h[corner_index(vx, vy)] = 12_000 + vx * 30 + vy * 20 - dip;
        }
    }
    for s in &specs {
        let [x, y, w, hh] = s.lot;
        let mut m = i32::MAX;
        for vy in y..=y + hh {
            for vx in x..=x + w {
                m = m.min(h[corner_index(vx, vy)]);
            }
        }
        for vy in y..=y + hh {
            for vx in x..=x + w {
                h[corner_index(vx, vy)] = m;
            }
        }
    }

    for s in &specs {
        for c in s.lot_cells() {
            if surf[tile_index(c[0], c[1])] == Surface::Grass {
                surf[tile_index(c[0], c[1])] = Surface::Lot;
            }
        }
    }
    let public = |x: i32, y: i32| {
        x >= 0
            && y >= 0
            && x < n
            && y < n
            && matches!(
                surf[tile_index(x, y)],
                Surface::Street | Surface::Sidewalk | Surface::Plaza | Surface::Bridge
            )
    };
    let mut lots = Vec::new();
    let mut buildings = Vec::new();
    for s in &specs {
        let lot_id = format!("lote-{}", s.id);
        lots.push(Lot {
            id: lot_id.clone(),
            rect: s.lot,
            zone: "centro".into(),
            front: s.front,
            building: Some(s.id.into()),
            owner_ref: None,
        });
        let mut levels: Vec<Vec<[i32; 2]>> = Vec::new();
        let back_row = match s.front {
            Dir::S => s.plan.iter().map(|c| c[1]).min().unwrap(),
            Dir::N => s.plan.iter().map(|c| c[1]).max().unwrap(),
            Dir::W => s.plan.iter().map(|c| c[0]).max().unwrap(),
            Dir::E => s.plan.iter().map(|c| c[0]).min().unwrap(),
        };
        let is_back = |c: &[i32; 2]| match s.front {
            Dir::N | Dir::S => c[1] == back_row,
            _ => c[0] == back_row,
        };
        for k in 0..s.floors {
            levels.push(
                s.plan
                    .iter()
                    .copied()
                    .filter(|c| k < 2 || !is_back(c))
                    .collect(),
            );
        }
        let tower = s.tower.as_ref().map(|(cells, from)| {
            levels.push(cells.clone());
            Tower {
                cells: cells.clone(),
                from_level: *from,
                levels: 1,
            }
        });
        let (dx, dy) = s.front.delta();
        let door = Door {
            cell: s.door,
            outside: [s.door[0] + dx, s.door[1] + dy],
            face: s.front,
            level: 0,
        };
        let mut bays = Vec::new();
        for (k, cells) in levels.iter().enumerate() {
            let tower_level = k >= s.floors as usize;
            for c in cells {
                for face in [Dir::N, Dir::E, Dir::S, Dir::W] {
                    let (fx, fy) = face.delta();
                    if cells.contains(&[c[0] + fx, c[1] + fy]) {
                        continue;
                    }
                    let street_face = face == s.front || public(c[0] + fx, c[1] + fy);
                    let kind = if tower_level {
                        "janela"
                    } else if k == 0 && *c == s.door && face == s.front {
                        "porta"
                    } else if street_face && k == 0 && s.use_ == "loja-sobrado" {
                        "vitrine"
                    } else if street_face && k == 1 && s.balcony {
                        "sacada"
                    } else if (c[0] + c[1] + k as i32) % 5 == 0 {
                        "cega"
                    } else {
                        "janela"
                    };
                    bays.push(Bay {
                        level: k as u8,
                        cell: *c,
                        face,
                        kind: kind.into(),
                    });
                }
            }
        }
        let stairs = if s.floors > 1 {
            s.plan
                .iter()
                .filter(|c| is_back(c))
                .take(1)
                .copied()
                .collect()
        } else {
            Vec::new()
        };
        buildings.push(Building {
            id: s.id.into(),
            lot: lot_id,
            style: s.style.into(),
            palette: s.palette.into(),
            use_: s.use_.into(),
            floors: s.floors,
            levels,
            door,
            stairs,
            roof: Roof {
                kind: s.roof.into(),
                ridge_axis: s.axis,
            },
            bays,
            tower,
            typology: None,
            progress: s.progress,
            owner_ref: None,
        });
    }

    let prop = |kind: &str, preset: &str, x: i32, y: i32, seed: u32| Prop {
        kind: kind.into(),
        preset: preset.into(),
        cell: [x, y],
        seed,
        anchors: Vec::new(),
    };
    let props = vec![
        prop("banco", "", 26, 17, 11),
        prop("banco", "", 35, 17, 12),
        prop("banco", "", 26, 26, 13),
        prop("banco", "", 35, 26, 14),
        prop("arvore", "oak-estilizada", 25, 16, 21),
        prop("arvore", "oak-estilizada", 36, 16, 22),
        prop("arvore", "ipe", 25, 27, 23),
        prop("arvore", "ipe", 36, 27, 24),
        prop("chafariz", "", 30, 21, 31),
        prop("poste", "", 24, 21, 41),
        prop("poste", "", 37, 21, 42),
        prop("arvore", "palmeira", 47, 61, 51),
        prop("arvore", "coqueiro", 52, 62, 52),
        prop("arvore", "palmeira", 58, 61, 53),
    ];

    let mut bp = Blueprint {
        schema: "omni.city.blueprint/v1".into(),
        city_id: "amostra".into(),
        region: [0, 0],
        generator_version: GENERATOR_VERSION.into(),
        program_hash: "sha256:amostra-mesher-c".into(),
        surface: surf,
        corner_height_mm: h,
        streets: vec![
            StreetRef {
                id: "av-central".into(),
                class: "avenida".into(),
                width: 2,
                tiles: (0..n).flat_map(|x| [[x, 30], [x, 31]]).collect(),
            },
            StreetRef {
                id: "rua-do-porto".into(),
                class: "rua".into(),
                width: 2,
                tiles: (0..n).flat_map(|y| [[20, y], [21, y]]).collect(),
            },
            StreetRef {
                id: "rua-das-flores".into(),
                class: "rua".into(),
                width: 2,
                tiles: (0..n).flat_map(|y| [[44, y], [45, y]]).collect(),
            },
            StreetRef {
                id: "beco-norte".into(),
                class: "beco".into(),
                width: 1,
                tiles: (0..20).map(|x| [x, 12]).collect(),
            },
        ],
        lots,
        buildings,
        props,
        places: vec![Place {
            id: "praca-matriz".into(),
            kind: "praca".into(),
            rect: [24, 15, 14, 13],
        }],
        hash: String::new(),
    };
    bp.hash = canonical_hash(&bp);
    bp
}

impl B<'_> {
    fn lot_cells(&self) -> Vec<[i32; 2]> {
        let [x, y, w, h] = self.lot;
        rect(x, y, w, h)
    }
}
