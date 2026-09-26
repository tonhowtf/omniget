//! Entities as arrays per component.
//!
//! One `Vec` per field instead of one `Vec` of structs: the tick walks
//! position, then path, then activity, and each pass touches a contiguous
//! run of memory. `BTreeMap` — never `HashMap` — maps an id to a slot, because
//! iteration order is part of the snapshot and a hashed order is not
//! reproducible across processes.

use std::collections::BTreeMap;

use crate::ents::agent::{speed_for, Activity, ANIM_IDLE, DIR_S, ENERGY_FULL};
use crate::ents::id::{EntId, ObjectId};
use crate::ents::object::Object;
use crate::error::{Result, WorldError};
use crate::fixed::Fixed;
use crate::map::Tile;
use crate::rng::Rng;
use crate::sim::routine::Routine;

/// How much faster than a stroll an agent heads for a workstation.
pub const WORK_HURRY: i32 = 2;

/// The route an agent is walking, as tiles left to visit. The `Vec` is reused
/// across paths so a walking agent allocates nothing per tick.
#[derive(Clone, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Path {
    steps: Vec<Tile>,
    cursor: usize,
}

impl Path {
    pub fn set(&mut self, steps: &[Tile]) {
        self.steps.clear();
        self.steps.extend_from_slice(steps);
        self.cursor = 0;
    }

    pub fn clear(&mut self) {
        self.steps.clear();
        self.cursor = 0;
    }

    pub fn next(&self) -> Option<Tile> {
        self.steps.get(self.cursor).copied()
    }

    pub fn advance(&mut self) {
        self.cursor += 1;
        if self.cursor >= self.steps.len() {
            self.clear();
        }
    }

    pub fn is_empty(&self) -> bool {
        self.cursor >= self.steps.len()
    }

    pub fn remaining(&self) -> usize {
        self.steps.len().saturating_sub(self.cursor)
    }

    pub fn goal(&self) -> Option<Tile> {
        self.steps.last().copied()
    }

    /// Tiles still to walk, the next one first.
    pub fn remaining_tiles(&self) -> &[Tile] {
        &self.steps[self.cursor.min(self.steps.len())..]
    }
}

/// What an agent means to do when it gets where it is going. Walking is a
/// journey, not a decision, so the decision is parked here until arrival.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Intent {
    #[default]
    None,
    Sit(ObjectId),
    Work(ObjectId),
    Sleep,
}

/// Agents, one array per component. Slots are dense and stable inside a tick;
/// removal swaps the last agent in, so nothing but the id may be used to
/// address an agent between ticks.
#[derive(Clone, Debug, Default)]
pub struct Agents {
    pub id: Vec<EntId>,
    pub name: Vec<String>,
    pub x: Vec<Fixed>,
    pub y: Vec<Fixed>,
    pub z: Vec<Fixed>,
    pub dir: Vec<u8>,
    pub anim: Vec<u8>,
    pub energy: Vec<u8>,
    pub activity: Vec<Activity>,
    // --- not observable, never in a snapshot ---
    pub path: Vec<Path>,
    pub intent: Vec<Intent>,
    /// Ticks left in the current activity; 0 means "until something changes".
    pub timer: Vec<u16>,
    pub routine: Vec<Routine>,
    pub rng: Vec<Rng>,
    /// Last routine minute fired, so a restarted day fires again and a
    /// re-entered minute does not.
    pub last_routine_min: Vec<u16>,
    index: BTreeMap<EntId, usize>,
}

impl Agents {
    pub fn len(&self) -> usize {
        self.id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.id.is_empty()
    }

    pub fn slot(&self, id: EntId) -> Option<usize> {
        self.index.get(&id).copied()
    }

    pub fn contains(&self, id: EntId) -> bool {
        self.index.contains_key(&id)
    }

    /// Ids in ascending order, which is the order everything on the wire uses.
    pub fn ids_sorted(&self) -> impl Iterator<Item = (EntId, usize)> + '_ {
        self.index.iter().map(|(id, slot)| (*id, *slot))
    }

    pub fn spawn(&mut self, id: EntId, name: &str, at: Tile, z: Fixed, seed: Rng) -> Result<usize> {
        if id.is_none() {
            return Err(WorldError::UnknownEnt(0));
        }
        if self.index.contains_key(&id) {
            return Err(WorldError::DuplicateEnt(id.0));
        }
        let slot = self.id.len();
        self.id.push(id);
        self.name.push(name.to_string());
        self.x.push(Fixed::from_tiles(at.x) + Fixed::HALF);
        self.y.push(Fixed::from_tiles(at.y) + Fixed::HALF);
        self.z.push(z);
        self.dir.push(DIR_S);
        self.anim.push(ANIM_IDLE);
        self.energy.push(ENERGY_FULL);
        self.activity.push(Activity::Idle);
        self.path.push(Path::default());
        self.intent.push(Intent::None);
        self.timer.push(0);
        self.routine.push(Routine::default());
        self.rng.push(seed.stream(id.0 as u64));
        self.last_routine_min.push(u16::MAX);
        self.index.insert(id, slot);
        Ok(slot)
    }

    pub fn despawn(&mut self, id: EntId) -> Result<()> {
        let slot = self.index.remove(&id).ok_or(WorldError::UnknownEnt(id.0))?;
        let last = self.id.len() - 1;
        self.id.swap_remove(slot);
        self.name.swap_remove(slot);
        self.x.swap_remove(slot);
        self.y.swap_remove(slot);
        self.z.swap_remove(slot);
        self.dir.swap_remove(slot);
        self.anim.swap_remove(slot);
        self.energy.swap_remove(slot);
        self.activity.swap_remove(slot);
        self.path.swap_remove(slot);
        self.intent.swap_remove(slot);
        self.timer.swap_remove(slot);
        self.routine.swap_remove(slot);
        self.rng.swap_remove(slot);
        self.last_routine_min.swap_remove(slot);
        if slot != last {
            let moved = self.id[slot];
            self.index.insert(moved, slot);
        }
        Ok(())
    }

    /// Whole tile an agent stands on.
    #[inline]
    pub fn tile_of(&self, slot: usize) -> Tile {
        Tile::new(self.x[slot].floor_tile(), self.y[slot].floor_tile())
    }

    #[inline]
    pub fn speed(&self, slot: usize) -> Fixed {
        let base = speed_for(self.energy[slot]);
        // Called to a workstation, an agent hurries: a tool call lasts
        // seconds, and an agent that strolls arrives after the work is over.
        match self.intent[slot] {
            Intent::Work(_) => Fixed(base * WORK_HURRY),
            _ => Fixed(base),
        }
    }

    /// Set the activity and the clip that goes with it in one place, so the
    /// two can never disagree.
    #[inline]
    pub fn set_activity(&mut self, slot: usize, a: Activity) {
        self.activity[slot] = a;
        self.anim[slot] = a.anim();
    }
}

/// Placed objects, kept sorted by id so the wire order is stable.
#[derive(Clone, Debug, Default)]
pub struct Objects {
    items: Vec<Object>,
    index: BTreeMap<ObjectId, usize>,
}

impl Objects {
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn all(&self) -> &[Object] {
        &self.items
    }

    pub fn get(&self, id: ObjectId) -> Option<&Object> {
        self.index.get(&id).map(|i| &self.items[*i])
    }

    pub fn get_mut(&mut self, id: ObjectId) -> Option<&mut Object> {
        let i = *self.index.get(&id)?;
        self.items.get_mut(i)
    }

    /// Insert or replace. Objects stay sorted by id.
    pub fn put(&mut self, o: Object) {
        match self.index.get(&o.id) {
            Some(i) => self.items[*i] = o,
            None => {
                let pos = self.items.partition_point(|x| x.id < o.id);
                self.items.insert(pos, o);
                self.reindex();
            }
        }
    }

    pub fn remove(&mut self, id: ObjectId) -> Result<Object> {
        let i = *self.index.get(&id).ok_or(WorldError::UnknownObject(id.0))?;
        let o = self.items.remove(i);
        self.reindex();
        Ok(o)
    }

    fn reindex(&mut self) {
        self.index.clear();
        for (i, o) in self.items.iter().enumerate() {
            self.index.insert(o.id, i);
        }
    }

    /// First object covering a tile, in id order.
    pub fn at(&self, t: Tile) -> Option<&Object> {
        self.items.iter().find(|o| o.covers(t))
    }

    /// Next free id, so `PlaceObject` never has to invent one.
    pub fn next_id(&self) -> ObjectId {
        ObjectId(self.items.last().map(|o| o.id.0 + 1).unwrap_or(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(id: u32, x: i32, y: i32) -> Object {
        Object {
            id: ObjectId(id),
            kind: "object/chair".into(),
            tile: Tile::new(x, y),
            dir: 0,
            slot: None,
            footprint: [1, 1],
            walkable: false,
            height: 16,
        }
    }

    #[test]
    fn spawn_and_despawn_keep_the_index_honest() {
        let mut a = Agents::default();
        let r = Rng::new(1);
        a.spawn(EntId(1), "omni", Tile::new(2, 3), Fixed::ZERO, r)
            .unwrap();
        a.spawn(EntId(2), "ada", Tile::new(4, 5), Fixed::ZERO, r)
            .unwrap();
        a.spawn(EntId(3), "bo", Tile::new(6, 7), Fixed::ZERO, r)
            .unwrap();
        assert_eq!(a.len(), 3);
        a.despawn(EntId(1)).unwrap();
        assert_eq!(a.len(), 2);
        assert!(!a.contains(EntId(1)));
        for id in [EntId(2), EntId(3)] {
            let s = a.slot(id).unwrap();
            assert_eq!(a.id[s], id);
        }
        assert_eq!(a.tile_of(a.slot(EntId(3)).unwrap()), Tile::new(6, 7));
    }

    #[test]
    fn duplicate_and_missing_ids_are_errors() {
        let mut a = Agents::default();
        let r = Rng::new(1);
        a.spawn(EntId(1), "omni", Tile::new(0, 0), Fixed::ZERO, r)
            .unwrap();
        assert_eq!(
            a.spawn(EntId(1), "omni", Tile::new(0, 0), Fixed::ZERO, r),
            Err(WorldError::DuplicateEnt(1))
        );
        assert_eq!(a.despawn(EntId(9)), Err(WorldError::UnknownEnt(9)));
        assert_eq!(
            a.spawn(EntId(0), "nil", Tile::new(0, 0), Fixed::ZERO, r),
            Err(WorldError::UnknownEnt(0))
        );
    }

    #[test]
    fn agents_are_centred_on_their_tile() {
        let mut a = Agents::default();
        a.spawn(EntId(1), "omni", Tile::new(2, 3), Fixed::ONE, Rng::new(1))
            .unwrap();
        assert_eq!(a.x[0], Fixed::from_tiles(2) + Fixed::HALF);
        assert_eq!(a.z[0], Fixed::ONE);
        assert_eq!(a.tile_of(0), Tile::new(2, 3));
    }

    #[test]
    fn ids_sorted_is_ascending_after_a_swap_remove() {
        let mut a = Agents::default();
        for i in 1..=5u32 {
            a.spawn(EntId(i), "x", Tile::new(0, 0), Fixed::ZERO, Rng::new(1))
                .unwrap();
        }
        a.despawn(EntId(2)).unwrap();
        let ids: Vec<u32> = a.ids_sorted().map(|(id, _)| id.0).collect();
        assert_eq!(ids, vec![1, 3, 4, 5]);
    }

    #[test]
    fn path_reuses_its_buffer() {
        let mut p = Path::default();
        p.set(&[Tile::new(1, 0), Tile::new(2, 0)]);
        assert_eq!(p.remaining(), 2);
        assert_eq!(p.next(), Some(Tile::new(1, 0)));
        assert_eq!(p.goal(), Some(Tile::new(2, 0)));
        p.advance();
        assert_eq!(p.next(), Some(Tile::new(2, 0)));
        p.advance();
        assert!(p.is_empty());
        assert_eq!(p.next(), None);
    }

    #[test]
    fn objects_stay_sorted_and_addressable() {
        let mut o = Objects::default();
        o.put(obj(3, 1, 1));
        o.put(obj(1, 2, 2));
        o.put(obj(2, 3, 3));
        let ids: Vec<u32> = o.all().iter().map(|x| x.id.0).collect();
        assert_eq!(ids, vec![1, 2, 3]);
        assert_eq!(o.next_id(), ObjectId(4));
        assert_eq!(o.at(Tile::new(2, 2)).map(|x| x.id), Some(ObjectId(1)));
        o.remove(ObjectId(2)).unwrap();
        assert_eq!(o.get(ObjectId(3)).map(|x| x.tile), Some(Tile::new(1, 1)));
        assert_eq!(o.remove(ObjectId(2)), Err(WorldError::UnknownObject(2)));
    }

    #[test]
    fn putting_the_same_id_replaces_it() {
        let mut o = Objects::default();
        o.put(obj(1, 1, 1));
        o.put(obj(1, 9, 9));
        assert_eq!(o.len(), 1);
        assert_eq!(o.get(ObjectId(1)).unwrap().tile, Tile::new(9, 9));
    }

    #[test]
    fn set_activity_keeps_the_clip_in_sync() {
        let mut a = Agents::default();
        a.spawn(EntId(1), "omni", Tile::new(0, 0), Fixed::ZERO, Rng::new(1))
            .unwrap();
        a.set_activity(0, Activity::Working(ObjectId(2)));
        assert_eq!(a.anim[0], crate::ents::agent::ANIM_WORK);
        assert_eq!(a.activity[0], Activity::Working(ObjectId(2)));
    }
}
