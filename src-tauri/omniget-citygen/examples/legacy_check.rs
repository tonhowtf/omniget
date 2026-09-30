//! Convert a workshop package (the 2D city) and run the building code on
//! every region: `cargo run -p omniget-citygen --example legacy_check -- <world.json>`.
use omniget_citygen::{checker, legacy, solver};

fn main() {
    let path = std::env::args().nth(1).expect("path to world.json");
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let c = legacy::convert(&v).expect("convert");
    let charter = legacy::from_legacy(&v).unwrap().0;
    for p in &c.programs {
        match solver::solve(&charter, p) {
            Ok(bp) => {
                for v in checker::check(&bp, p) {
                    if v.severity == omniget_citygen::Severity::Error {
                        println!(
                            "{:?} {} {} {} | {}",
                            p.region, v.code, v.id, v.measure, v.message
                        );
                    }
                }
            }
            Err(vs) => {
                for v in vs {
                    println!("{:?} SOLVE {} {} {}", p.region, v.code, v.id, v.message);
                }
            }
        }
    }
}
