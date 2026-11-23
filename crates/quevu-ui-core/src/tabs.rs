//! Where a tab goes among the others.
//!
//! No tab has a place of its own. Each one carries a weight the host gave
//! it, and the row of tabs is kept in order of weight: the heavier a tab,
//! the nearer the start. Tabs that weigh the same stay in the order they
//! were opened. A tab that says nothing weighs zero, so a negative weight
//! sends a tab after all of those.

/// The position a new tab of `weight` takes in a row whose tabs weigh
/// `weights`, in the order they are shown.
pub fn insert_at(weights: &[i32], weight: i32) -> usize {
    weights.iter().take_while(|&&w| w >= weight).count()
}

/// The tab that comes to the front when the one at `removed` goes away
/// from a row that is left with `left` tabs: the one that takes its place,
/// or the last one, or none.
pub fn front_after_remove(removed: usize, left: usize) -> Option<usize> {
    (left > 0).then(|| removed.min(left - 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_heavier_goes_first() {
        assert_eq!(insert_at(&[], 0), 0);
        assert_eq!(insert_at(&[30, 20, 10], 100), 0);
        assert_eq!(insert_at(&[30, 20, 10], 25), 1);
        assert_eq!(insert_at(&[30, 20, 10], 5), 3);
    }

    #[test]
    fn of_two_that_weigh_the_same_the_first_stays_first() {
        assert_eq!(insert_at(&[0, 0], 0), 2);
        assert_eq!(insert_at(&[30, 20, 20, 10], 20), 3);
    }

    #[test]
    fn a_negative_weight_goes_after_those_that_say_nothing() {
        assert_eq!(insert_at(&[5, 0, 0], -1), 3);
        assert_eq!(insert_at(&[5, 0, -1], 0), 2);
        assert_eq!(insert_at(&[0, -1, -5], -3), 2);
    }

    #[test]
    fn the_neighbour_takes_the_front() {
        assert_eq!(front_after_remove(0, 0), None);
        assert_eq!(front_after_remove(0, 2), Some(0));
        assert_eq!(front_after_remove(1, 2), Some(1));
        assert_eq!(front_after_remove(2, 2), Some(1));
    }
}
