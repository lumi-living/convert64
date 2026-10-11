//! Cell steering (UX §7): pins, locks and allow/deny lists that the user (or an agent)
//! places on cells to steer the solver. Part of the [`crate::Project`] and changed only
//! through the `cell_*` commands, so every steering action is undoable and scriptable.

use super::cell::Constraints;
use crate::colour::{ColourIndex, ColourSet};
use crate::geom::CellSet;
use crate::model::CellPos;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A pin on a cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pin {
    /// The pinned colours.
    pub colours: ColourSet,
    /// `true`: the cell's set is exactly these colours. `false` (partial pin): the set
    /// must contain them; the solver picks the rest.
    pub full: bool,
}

/// Steering state for every cell.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Steering {
    pins: BTreeMap<CellPos, Pin>,
    locks: CellSet,
    allow: BTreeMap<CellPos, ColourSet>,
    deny: BTreeMap<CellPos, ColourSet>,
}

impl Steering {
    /// No steering.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether nothing is steered.
    pub fn is_empty(&self) -> bool {
        self.pins.is_empty()
            && self.locks.is_empty()
            && self.allow.is_empty()
            && self.deny.is_empty()
    }

    /// The pin on `cell`, if any.
    pub fn pin(&self, cell: CellPos) -> Option<Pin> {
        self.pins.get(&cell).copied()
    }

    /// Pins `cell`.
    pub fn set_pin(&mut self, cell: CellPos, pin: Pin) {
        self.pins.insert(cell, pin);
    }

    /// Locked cells: copied from the previous solution untouched.
    pub const fn locks(&self) -> CellSet {
        self.locks
    }

    /// Locks or unlocks `cell`.
    pub fn set_lock(&mut self, cell: CellPos, locked: bool) {
        let mut s = CellSet::EMPTY;
        s.insert(cell);
        self.locks = if locked {
            self.locks.union(s)
        } else {
            let mut keep = CellSet::EMPTY;
            for c in self.locks.iter().filter(|c| *c != cell) {
                keep.insert(c);
            }
            keep
        };
    }

    /// The colours `cell` may use (`ColourSet::ALL` when unrestricted).
    pub fn allow(&self, cell: CellPos) -> ColourSet {
        self.allow.get(&cell).copied().unwrap_or(ColourSet::ALL)
    }

    /// Restricts `cell` to `colours`.
    pub fn set_allow(&mut self, cell: CellPos, colours: ColourSet) {
        self.allow.insert(cell, colours);
    }

    /// The colours `cell` must not use.
    pub fn deny(&self, cell: CellPos) -> ColourSet {
        self.deny.get(&cell).copied().unwrap_or(ColourSet::EMPTY)
    }

    /// Forbids `colours` in `cell`.
    pub fn set_deny(&mut self, cell: CellPos, colours: ColourSet) {
        self.deny.insert(cell, colours);
    }

    /// Removes every pin, lock, allow and deny on `cell`.
    pub fn clear(&mut self, cell: CellPos) {
        self.pins.remove(&cell);
        self.allow.remove(&cell);
        self.deny.remove(&cell);
        self.set_lock(cell, false);
    }

    /// The search constraints steering puts on `cell` (before the attribute map's
    /// allowed colours, which the input applies). A full pin fixes the set; a partial
    /// pin requires its colours; allow and deny filter.
    pub fn constraints(&self, cell: CellPos) -> Constraints {
        let filter = self.allow(cell).difference(self.deny(cell));
        match self.pin(cell) {
            Some(Pin {
                colours,
                full: true,
            }) => Constraints {
                candidates: colours,
                required: colours,
            },
            Some(Pin {
                colours,
                full: false,
            }) => Constraints {
                candidates: filter.union(colours),
                required: colours,
            },
            None => Constraints {
                candidates: filter,
                required: ColourSet::EMPTY,
            },
        }
    }

    /// Pinned cells and their pins, in cell order (for reports and serialisation).
    pub fn pins(&self) -> impl Iterator<Item = (CellPos, Pin)> + '_ {
        self.pins.iter().map(|(c, p)| (*c, *p))
    }
}

/// A colour set from a list of colours, for steering commands.
pub fn colour_set(colours: &[ColourIndex]) -> ColourSet {
    colours.iter().copied().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::colour::ColourIndex as C;

    #[test]
    fn constraints_follow_pins_allow_and_deny() {
        let mut s = Steering::new();
        let a = CellPos::new(1, 2);
        assert_eq!(s.constraints(a), Constraints::FREE);
        s.set_deny(a, colour_set(&[C::RED, C::BLUE]));
        assert!(!s.constraints(a).candidates.contains(C::RED));
        s.set_pin(
            a,
            Pin {
                colours: colour_set(&[C::YELLOW]),
                full: false,
            },
        );
        let c = s.constraints(a);
        assert!(c.required.contains(C::YELLOW) && !c.candidates.contains(C::BLUE));
        let full = colour_set(&[C::WHITE, C::BLACK, C::CYAN]);
        s.set_pin(
            a,
            Pin {
                colours: full,
                full: true,
            },
        );
        assert_eq!(
            s.constraints(a),
            Constraints {
                candidates: full,
                required: full
            }
        );
        s.set_lock(a, true);
        assert!(s.locks().contains(a));
        s.clear(a);
        assert!(s.is_empty());
    }
}
