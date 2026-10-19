//! What plays next: a list of tracks and which one is current.
//!
//! Pure data. Starting a track from the library makes the list on screen the queue, from the
//! top, with the chosen one current; opening a file makes that file the whole queue. A track that
//! reaches its end advances the queue; the last one ending leaves it where it is.

use crate::library::Track;

#[derive(Debug, Default, Clone)]
pub struct Queue {
    tracks: Vec<Track>,
    current: Option<usize>,
}

impl Queue {
    /// Replace the queue with `tracks` and make `at` current. `None` if `at` is not in it.
    pub fn start(&mut self, tracks: Vec<Track>, at: usize) -> Option<&Track> {
        if at >= tracks.len() {
            return None;
        }
        self.tracks = tracks;
        self.current = Some(at);
        self.current()
    }

    /// Make `at` current in the queue as it is.
    pub fn jump(&mut self, at: usize) -> Option<&Track> {
        if at >= self.tracks.len() {
            return None;
        }
        self.current = Some(at);
        self.current()
    }

    pub fn current(&self) -> Option<&Track> {
        self.current.and_then(|i| self.tracks.get(i))
    }

    pub fn current_index(&self) -> Option<usize> {
        self.current
    }

    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    /// Step forward. `None` at the end of the queue, which stays on its last track.
    pub fn next(&mut self) -> Option<&Track> {
        let i = self.current?;
        if i + 1 >= self.tracks.len() {
            return None;
        }
        self.current = Some(i + 1);
        self.current()
    }

    /// Step back. `None` at the top, which stays on the first track.
    pub fn previous(&mut self) -> Option<&Track> {
        let i = self.current?;
        if i == 0 {
            return None;
        }
        self.current = Some(i - 1);
        self.current()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracks(n: usize) -> Vec<Track> {
        (0..n)
            .map(|i| Track { title: format!("t{i}"), album: "a".into(), path: format!("/m/t{i}.mp3") })
            .collect()
    }

    #[test]
    fn a_queue_walks_forward_and_back_and_stops_at_its_ends() {
        let mut q = Queue::default();
        assert!(q.next().is_none(), "an empty queue goes nowhere");
        assert_eq!(q.start(tracks(3), 1).map(|t| t.title.clone()).as_deref(), Some("t1"));
        assert_eq!(q.next().map(|t| t.title.clone()).as_deref(), Some("t2"));
        assert!(q.next().is_none(), "the end of the queue");
        assert_eq!(q.current_index(), Some(2), "and it stays on the last track");
        assert_eq!(q.previous().map(|t| t.title.clone()).as_deref(), Some("t1"));
        assert_eq!(q.previous().map(|t| t.title.clone()).as_deref(), Some("t0"));
        assert!(q.previous().is_none());
        assert_eq!(q.current_index(), Some(0));
    }

    #[test]
    fn starting_or_jumping_past_the_end_changes_nothing() {
        let mut q = Queue::default();
        assert!(q.start(tracks(2), 5).is_none());
        assert!(q.is_empty());
        q.start(tracks(2), 0);
        assert!(q.jump(9).is_none());
        assert_eq!(q.current_index(), Some(0));
        assert_eq!(q.jump(1).map(|t| t.path.clone()).as_deref(), Some("/m/t1.mp3"));
    }
}
