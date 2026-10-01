pub mod group_a;
pub mod group_b;

use super::model::SessionSource;

pub fn all_sources() -> Vec<Box<dyn SessionSource>> {
    let mut v = group_a::sources();
    v.extend(group_b::sources());
    v
}
