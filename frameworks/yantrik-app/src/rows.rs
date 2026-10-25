//! A list on screen, set only when its rows changed.
//!
//! A `render` draws a list by setting its property to a new model: `ModelRc::new(VecModel::from(
//! rows))`. Slint compares a model property by identity, not by its rows, so a new model holding
//! the same rows is a changed property, the repeater under it is rebuilt, and the window draws a
//! frame for nothing. Done on a timer, that is a window that never rests: the agent rail did it
//! every four seconds, and Network did it on every three-second reading of an unchanged network.
//! [`set`] is the one way a list is drawn: compared row by row against what the window holds, and
//! replaced only when something in it differs.
//!
//! A list whose rows changed but not their number (a monitor's processes, every reading) is
//! changed in place, row by row, in the model the window already holds: only the rows that differ
//! are told to the repeater, and the rest keep their instances. A new model would rebuild every
//! row, a hundred of them every two seconds in System Monitor, for one or two that moved.

use slint::{Model, ModelRc, VecModel};

/// Whether `shown` already holds exactly `rows`, in order.
pub fn same<T: Clone + PartialEq + 'static>(shown: &ModelRc<T>, rows: &[T]) -> bool {
    shown.row_count() == rows.len()
        && rows
            .iter()
            .enumerate()
            .all(|(i, row)| shown.row_data(i).as_ref() == Some(row))
}

/// Sets `rows` through `put` unless `shown`, what the window holds now, already has them.
/// Whether anything changed. When `shown` is a list this module set (a `VecModel`) with as many
/// rows, only the rows that differ are written into it, and `put` is not called: the window
/// already holds that model.
///
/// ```ignore
/// rows::set(ui.get_hourly(), hourly, |m| ui.set_hourly(m));
/// ```
pub fn set<T: Clone + PartialEq + 'static>(
    shown: ModelRc<T>,
    rows: Vec<T>,
    put: impl FnOnce(ModelRc<T>),
) -> bool {
    if same(&shown, &rows) {
        return false;
    }
    if let Some(list) = shown.as_any().downcast_ref::<VecModel<T>>() {
        if list.row_count() == rows.len() {
            for (i, row) in rows.into_iter().enumerate() {
                if list.row_data(i).as_ref() != Some(&row) {
                    list.set_row_data(i, row);
                }
            }
            return true;
        }
    }
    put(ModelRc::new(VecModel::from(rows)));
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(rows: Vec<i32>) -> ModelRc<i32> {
        ModelRc::new(VecModel::from(rows))
    }

    #[test]
    fn the_same_rows_are_not_set_again_and_any_difference_is() {
        let shown = list(vec![1, 2, 3]);
        let mut put = None;
        assert!(!set(shown.clone(), vec![1, 2, 3], |m| put = Some(m)));
        assert!(put.is_none(), "the same rows: nothing set, nothing to draw");
        for rows in [vec![1, 2], vec![1, 2, 3, 4], Vec::new()] {
            let mut put = None;
            assert!(set(list(vec![1, 2, 3]), rows.clone(), |m| put = Some(m)), "{rows:?}");
            let set_to: Vec<i32> = put.expect("another number of rows: a new list").iter().collect();
            assert_eq!(set_to, rows);
        }
        assert!(same(&ModelRc::default(), &Vec::<i32>::new()), "an empty list is the empty list");
    }

    #[test]
    fn as_many_rows_are_changed_in_place_and_only_those_that_differ() {
        for rows in [vec![1, 2, 4], vec![3, 2, 1]] {
            let shown = list(vec![1, 2, 3]);
            let mut put = None;
            assert!(set(shown.clone(), rows.clone(), |m| put = Some(m)), "{rows:?}");
            assert!(put.is_none(), "the window's own list, changed: nothing new to set");
            assert_eq!(shown.iter().collect::<Vec<_>>(), rows);
        }
    }

    #[test]
    fn a_list_this_module_did_not_make_is_replaced_not_written_into() {
        // A `.slint` default, or any model that is not a VecModel, may not take a row written
        // into it: it is replaced whole, as before.
        let shown: ModelRc<i32> = ModelRc::new(slint::FilterModel::new(list(vec![1, 2, 3]), |_| true));
        let mut put = None;
        assert!(set(shown, vec![1, 2, 4], |m| put = Some(m)));
        assert_eq!(put.expect("replaced").iter().collect::<Vec<_>>(), vec![1, 2, 4]);
    }
}
