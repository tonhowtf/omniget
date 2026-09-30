//! The prototype's loop (estudo 09 §6): v1 is refused, v2 passes.

use omniget_citygen::checker::{check, errors};
use omniget_citygen::program::{apply, CityProgram, PatchOp};
use omniget_citygen::solver::{ascii, default_program, free_lots, options, solve};
use omniget_citygen::*;

fn load(name: &str) -> CityProgram {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// Flat land, no rivers or forest: the prototype had no terrain.
fn flat_charter() -> Charter {
    let mut c = charter::example("omni", "omni-2026");
    c.relief = Relief {
        sea_level_mm: -100_000,
        hill_mm: 0,
        mountain_mm: 0,
        erosion: 0,
        rivers: 0,
        forest: 0,
        city_flatten_slope: 80,
    };
    c
}

fn codes(v: &[&Violation]) -> Vec<String> {
    let mut c: Vec<String> = v.iter().map(|x| x.code.clone()).collect();
    c.sort();
    c
}

#[test]
fn v1_is_refused_with_the_prototype_violations() {
    let c = flat_charter();
    let p = load("program-v1.json");
    let bp = solve(&c, &p).expect("v1 solves");
    let v = check(&bp, &p);
    let e = errors(&v);
    eprintln!("{}", ascii(&bp));
    for x in &e {
        eprintln!("{} {} {} | {}", x.code, x.id, x.measure, x.message);
    }
    eprintln!(
        "v1 hash {} lots {} buildings {}",
        bp.hash,
        bp.lots.len(),
        bp.buildings.len()
    );
    // Prototype R3 x3, R5 (network) and R8 (height) are R3, R6 and R9 in the
    // study's numbering. The sub-seeds are hash.rs's, not the prototype's
    // `h32`, so the lot draws differ: here `casa-do-tonho` also lands on a
    // shallow lot (one more R3) while the prefeitura's lot is deep enough.
    let mut distinct = codes(&e);
    distinct.dedup();
    assert_eq!(distinct, vec!["R3", "R6", "R9"]);
    let net = e.iter().find(|x| x.code == "R6").unwrap();
    assert_eq!(net.measure, "140 tiles disconnected");
    assert!(net.message.contains("praca-matriz"));
    let h = e.iter().find(|x| x.code == "R9").unwrap();
    assert_eq!(
        (h.id.as_str(), h.measure.as_str()),
        ("prefeitura", "3 floors not in 1..2")
    );
    assert!(e.iter().filter(|x| x.code == "R3").count() >= 3);
    assert_eq!(e.len(), 6);
}

#[test]
fn v2_passes_and_is_deterministic() {
    let c = flat_charter();
    let p = load("program-v2.json");
    let bp = solve(&c, &p).expect("v2 solves");
    let v = check(&bp, &p);
    eprintln!("{}", ascii(&bp));
    for x in &v {
        eprintln!(
            "{:?} {} {} {} | {}",
            x.severity, x.code, x.id, x.measure, x.message
        );
    }
    eprintln!(
        "v2 hash {} lots {} buildings {} props {}",
        bp.hash,
        bp.lots.len(),
        bp.buildings.len(),
        bp.props.len()
    );
    assert!(errors(&v).is_empty());
    let again = solve(&c, &p).unwrap();
    assert_eq!(bp.hash, again.hash);
    let pre = bp.buildings.iter().find(|b| b.id == "prefeitura").unwrap();
    assert_eq!(
        bp.lots.iter().find(|l| l.id == pre.lot).unwrap().zone,
        "centro"
    );
    assert_eq!(pre.floors, 3);
    assert!(pre.tower.is_some());
    let casa = bp
        .buildings
        .iter()
        .find(|b| b.id == "casa-do-tonho")
        .unwrap();
    assert_eq!(casa.owner_ref.as_deref(), Some("home:1"));
    assert!(casa.bays.iter().any(|b| b.kind == "sacada"));
}

#[test]
fn default_program_passes() {
    let c = flat_charter();
    let p = default_program("omni", [0, 0]);
    let bp = solve(&c, &p).expect("default solves");
    let v = check(&bp, &p);
    for x in &v {
        eprintln!(
            "{:?} {} {} {} | {}",
            x.severity, x.code, x.id, x.measure, x.message
        );
    }
    eprintln!("{}", ascii(&bp));
    eprintln!(
        "default hash {} buildings {} props {}",
        bp.hash,
        bp.buildings.len(),
        bp.props.len()
    );
    assert!(errors(&v).is_empty());
    let o = options(&c, &p);
    assert!(o["zones"][0]["free_lots"].as_array().is_some());
    assert!(!free_lots(&c, &p, Some("centro")).is_empty());
}

#[test]
fn patch_ops_and_p_codes() {
    let c = flat_charter();
    let p = load("program-v2.json");
    let ops: Vec<PatchOp> = serde_json::from_value(serde_json::json!([
        { "op": "add_building", "id": "padaria", "lot": "auto:zone:centro",
          "intent": { "use": "loja-sobrado", "floors": 2, "shape": "rect", "roof": "duas-aguas", "awning": true } },
        { "op": "set_intent", "id": "prefeitura", "patch": { "tower": false } },
        { "op": "add_street", "id": "rua-nova", "class": "rua", "width": 2, "path": [[20, 40], [44, 40]] },
        { "op": "remove", "id": "beco-norte" }
    ]))
    .unwrap();
    let next = apply(&p, &ops, false).expect("ops apply");
    assert!(
        !next
            .buildings
            .iter()
            .find(|b| b.id == "prefeitura")
            .unwrap()
            .intent
            .tower
    );
    assert!(next.streets.iter().all(|s| s.id != "beco-norte"));
    let bp = solve(&c, &next).unwrap();
    let v = check(&bp, &next);
    for x in errors(&v) {
        eprintln!("{} {} {} | {}", x.code, x.id, x.measure, x.message);
    }

    let bad: Vec<PatchOp> = serde_json::from_value(serde_json::json!([
        { "op": "add_zone", "id": "centro", "kind": "civico", "rect": [0, 0, 4, 4], "style": "vila-colonial", "floors": [1, 2] },
        { "op": "remove", "id": "nada" },
        { "op": "add_landmark", "id": "fora", "kind": "praca", "rect": [60, 60, 10, 10] },
        { "op": "add_street", "id": "vazia", "class": "rua", "width": 2, "path": [] },
        { "op": "add_building", "id": "x", "lot": "auto:zone:nenhuma", "intent": { "use": "casa" } },
        { "op": "set_intent", "id": "casa-do-tonho", "patch": { "floors": 1 } },
        { "op": "set_intent", "id": "prefeitura", "patch": { "cor": "azul" } }
    ]))
    .unwrap();
    let e = apply(&p, &bad, false).unwrap_err();
    for x in &e {
        eprintln!("{} {} {} | {}", x.code, x.id, x.measure, x.message);
    }
    let got: Vec<&str> = e.iter().map(|x| x.code.as_str()).collect();
    assert_eq!(got, vec!["P1", "P2", "P3", "P4", "P5", "P7", "P8"]);
    assert!(apply(&p, &bad[5..6], true).is_ok());
}

#[test]
fn explicit_lots_are_verbatim() {
    let c = flat_charter();
    let p = load("golden-explicit.json");
    let bp = solve(&c, &p).expect("golden solves");
    let v = check(&bp, &p);
    for x in &v {
        eprintln!(
            "{:?} {} {} {} | {}",
            x.severity, x.code, x.id, x.measure, x.message
        );
    }
    eprintln!("{}", ascii(&bp));
    for e in &p.lots.explicit {
        let l = bp
            .lots
            .iter()
            .find(|l| l.id == e.id)
            .expect("explicit lot kept");
        assert_eq!(l.rect, e.rect);
        assert_eq!(l.owner_ref, e.owner_ref);
        if let Some(b) = l
            .building
            .as_ref()
            .and_then(|id| bp.buildings.iter().find(|b| &b.id == id))
        {
            assert_eq!(Some(b.door.outside), e.door);
        }
    }
    assert!(!bp
        .lots
        .iter()
        .any(|l| l.id.starts_with('q')
            && p.lots.explicit.iter().any(|e| rects_touch(e.rect, l.rect))));
    assert!(errors(&v).iter().all(|x| x.code != "R5" && x.code != "R17"));
}

fn rects_touch(a: [i32; 4], b: [i32; 4]) -> bool {
    a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3]
}
