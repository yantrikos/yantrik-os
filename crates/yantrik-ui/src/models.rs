//! Keep stable list models when polling produces no visible change.
use slint::{Model, ModelRc, VecModel};

pub fn changed<T: Clone + PartialEq + 'static>(
    current: ModelRc<T>,
    rows: Vec<T>,
) -> Option<ModelRc<T>> {
    if current.row_count() == rows.len()
        && rows
            .iter()
            .enumerate()
            .all(|(i, row)| current.row_data(i).as_ref() == Some(row))
    {
        None
    } else {
        Some(ModelRc::new(VecModel::from(rows)))
    }
}

/// Like [`changed`], but a refresh that keeps the same rows (by `key`) and only changes what is
/// inside them is written into the existing model, row by row, and the model itself is not
/// replaced. A resting pointer, a hover label and a list the person is reading are all tied to
/// the model object: replacing it makes the screen think every row is new. Returns the model to
/// set, or `None` when the current one was kept (updated in place or already identical).
pub fn update<T, K>(current: ModelRc<T>, rows: Vec<T>, key: impl Fn(&T) -> K) -> Option<ModelRc<T>>
where
    T: Clone + PartialEq + 'static,
    K: PartialEq,
{
    let same_rows = current.row_count() == rows.len()
        && rows.iter().enumerate().all(|(i, row)| current.row_data(i).is_some_and(|old| key(&old) == key(row)));
    if !same_rows {
        return Some(ModelRc::new(VecModel::from(rows)));
    }
    for (i, row) in rows.into_iter().enumerate() {
        if current.row_data(i).as_ref() != Some(&row) {
            current.set_row_data(i, row);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    fn model(rows: &[&str]) -> ModelRc<String> {
        ModelRc::new(VecModel::from(
            rows.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        ))
    }
    fn rows(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| s.to_string()).collect()
    }
    #[test]
    fn unchanged_polls_keep_the_existing_model() {
        assert!(changed(model(&["Terminal", "Notes"]), rows(&["Terminal", "Notes"])).is_none());
        assert!(changed(model(&[]), vec![]).is_none());
    }
    #[test]
    fn additions_removals_updates_and_order_changes_still_publish() {
        for values in [
            &["Terminal", "Notes", "Editor"][..],
            &["Terminal"],
            &["Notes", "Terminal"],
            &["Terminal", "Notes — unsaved"],
            &[],
        ] {
            let updated = changed(model(&["Terminal", "Notes"]), rows(values)).unwrap();
            assert_eq!(updated.iter().collect::<Vec<_>>(), rows(values));
        }
    }

    #[test]
    fn a_refresh_that_keeps_the_rows_updates_them_in_place() {
        // (key, title): the title changes, the row does not.
        let current = ModelRc::new(VecModel::from(vec![("term".to_string(), "a".to_string()), ("files".to_string(), "x".to_string())]));
        let kept = current.clone();
        let out = update(current, vec![("term".into(), "b".into()), ("files".into(), "x".into())], |r| r.0.clone());
        assert!(out.is_none(), "the same model stays on screen, so hover and the list survive");
        assert_eq!(kept.row_data(0).unwrap().1, "b", "and the new title is in it");
    }

    #[test]
    fn a_refresh_that_changes_the_rows_replaces_the_model() {
        let current = ModelRc::new(VecModel::from(vec![("term".to_string(), "a".to_string())]));
        assert!(update(current.clone(), vec![("files".into(), "a".into())], |r| r.0.clone()).is_some(), "another app");
        assert!(update(current, vec![("term".into(), "a".into()), ("files".into(), "a".into())], |r| r.0.clone()).is_some(), "another count");
    }
}
