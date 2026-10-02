//! The Alt+Tab switcher's model: which windows, in what order, which is selected, which page is
//! showing. No UI and no compositor in here, so every rule is a test; `control_switcher` draws
//! it and runs the actions that move it.
//!
//! Pointer, keys and `yos act shell` all end in these functions, because a switcher whose
//! keyboard path and control path disagreed about "next" would be two switchers.

/// Cells to a page. Fixed: a card that grew with the window count would shrink its thumbnails
/// to stamps, which the spec rules out.
pub const PAGE: usize = 8;

#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    /// The window's title, which is also how the compositor is asked to bring it forward.
    pub title: String,
    pub app_id: String,
    pub app_name: String,
}

/// The windows, most recently used first. `recency` is what the compositor's focus stream has
/// seen (`toplevel_watch::recency`); windows it has not seen take focus follow in the order they
/// were listed in, behind every window that is known. The same title is never listed twice, since
/// the title is the only handle there is.
pub fn order(open: Vec<Cell>, recency: &[String]) -> Vec<Cell> {
    let mut rest = open;
    let mut ordered = Vec::with_capacity(rest.len());
    for title in recency {
        if let Some(at) = rest.iter().position(|c| &c.title == title) {
            ordered.push(rest.remove(at));
        }
    }
    for cell in rest {
        if !ordered.iter().any(|c| c.title == cell.title) {
            ordered.push(cell);
        }
    }
    ordered
}

/// An open switcher. The cells are frozen at the moment it opened: a window that opens
/// afterwards waits for the next time. One that closes while the switcher is up stays listed until
/// it closes; choosing it then finds no window, and says so in the log.
#[derive(Clone, Debug)]
pub struct Switcher {
    cells: Vec<Cell>,
    selected: usize,
    /// The window that had focus before the shell was raised to show the switcher, so Escape can
    /// give it back.
    origin: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Move {
    Next,
    Previous,
    Left,
    Right,
    Up,
    Down,
    PageNext,
    PagePrev,
}

impl Move {
    pub fn parse(word: &str) -> Option<Move> {
        Some(match word {
            "next" => Move::Next,
            "previous" => Move::Previous,
            "left" => Move::Left,
            "right" => Move::Right,
            "up" => Move::Up,
            "down" => Move::Down,
            "page-next" => Move::PageNext,
            "page-prev" => Move::PagePrev,
            _ => return None,
        })
    }

    pub const WORDS: &'static str = "next, previous, left, right, up, down, page-next, page-prev";
}

impl Switcher {
    /// The first step selects the previous window — the one before the window in front — so
    /// Alt+Tab, Alt+Tab toggles between two. Backwards starts from the oldest. With one window
    /// there is nothing to step to and it stays selected.
    pub fn open(cells: Vec<Cell>, origin: Option<String>, backwards: bool) -> Switcher {
        let selected = match cells.len() {
            0 | 1 => 0,
            n if backwards => n - 1,
            _ => 1,
        };
        Switcher { cells, selected, origin }
    }

    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }

    pub fn origin(&self) -> Option<&str> {
        self.origin.as_deref()
    }

    pub fn selected(&self) -> Option<&Cell> {
        self.cells.get(self.selected)
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    pub fn pages(&self) -> usize {
        self.cells.len().div_ceil(PAGE).max(1)
    }

    pub fn page(&self) -> usize {
        self.selected / PAGE
    }

    /// The cells on the page being shown.
    pub fn page_cells(&self) -> &[Cell] {
        let from = self.page() * PAGE;
        &self.cells[from.min(self.cells.len())..(from + PAGE).min(self.cells.len())]
    }

    /// The selection's place on its page, which is what the card draws.
    pub fn selected_on_page(&self) -> Option<usize> {
        (!self.cells.is_empty()).then(|| self.selected % PAGE)
    }

    /// Step one cell, wrapping: Tab crosses pages and the last window leads back to the first.
    pub fn step(&mut self, forward: bool) {
        let n = self.cells.len();
        if n > 1 {
            self.selected = if forward { (self.selected + 1) % n } else { (self.selected + n - 1) % n };
        }
    }

    /// Move as the keys do. `columns` is how many the card is drawn with (4, 3 or 2), so that up
    /// and down mean what the person sees. An arrow that has nowhere to go stays where it is,
    /// except left and right, which are the same walk as Tab and Shift+Tab across pages.
    pub fn go(&mut self, how: Move, columns: usize) {
        let columns = columns.clamp(1, PAGE);
        let n = self.cells.len();
        if n == 0 {
            return;
        }
        match how {
            Move::Next | Move::Right => self.step(true),
            Move::Previous | Move::Left => self.step(false),
            Move::Down => {
                let next = self.selected + columns;
                if next < n && next / PAGE == self.page() {
                    self.selected = next;
                }
            }
            Move::Up => {
                if self.selected % PAGE >= columns {
                    self.selected -= columns;
                }
            }
            // The same place on the next page, or its last cell if that page is shorter.
            Move::PageNext | Move::PagePrev => {
                let pages = self.pages();
                if pages > 1 {
                    let to = if how == Move::PageNext { (self.page() + 1) % pages } else { (self.page() + pages - 1) % pages };
                    let at = to * PAGE + self.selected % PAGE;
                    self.selected = at.min(n - 1);
                }
            }
        }
    }

    /// The pointer moved over cell `i` of the page.
    pub fn point_at(&mut self, i: usize) {
        let at = self.page() * PAGE + i;
        if at < self.cells.len() {
            self.selected = at;
        }
    }

    /// "9–16 of 37 windows · Page 2 of 5", or just the count when it all fits on one page.
    pub fn status(&self) -> String {
        let n = self.cells.len();
        let noun = if n == 1 { "window" } else { "windows" };
        if self.pages() == 1 {
            return format!("{n} {noun}");
        }
        let from = self.page() * PAGE + 1;
        let to = (from + PAGE - 1).min(n);
        format!("{from}–{to} of {n} {noun} · Page {} of {}", self.page() + 1, self.pages())
    }

    /// What the plate under the card says: the full title of the selected window.
    pub fn plate(&self) -> String {
        self.selected().map(|c| c.title.clone()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(title: &str) -> Cell {
        Cell { title: title.into(), app_id: "x".into(), app_name: title.into() }
    }

    fn cells(n: usize) -> Vec<Cell> {
        (0..n).map(|i| cell(&format!("w{i}"))).collect()
    }

    fn titles(cs: &[Cell]) -> Vec<&str> {
        cs.iter().map(|c| c.title.as_str()).collect()
    }

    /// The compositor's recency leads, and windows it never saw take focus come after, in the
    /// order they were listed — never dropped, never doubled.
    #[test]
    fn recent_windows_lead_and_unseen_ones_follow_in_listed_order() {
        let open = vec![cell("Files"), cell("Notes"), cell("Terminal"), cell("Mind View")];
        let recency = vec!["Terminal".to_string(), "Gone".to_string(), "Notes".to_string()];
        assert_eq!(titles(&order(open, &recency)), ["Terminal", "Notes", "Files", "Mind View"]);
        assert_eq!(titles(&order(vec![cell("A"), cell("A")], &[])), ["A"]);
    }

    /// The first Tab goes to the window before the one in front, so two presses toggle; the
    /// reverse chord starts from the oldest.
    #[test]
    fn the_first_step_selects_the_previous_window() {
        assert_eq!(Switcher::open(cells(5), None, false).selected().unwrap().title, "w1");
        assert_eq!(Switcher::open(cells(5), None, true).selected().unwrap().title, "w4");
        assert_eq!(Switcher::open(cells(1), None, false).selected().unwrap().title, "w0");
        assert!(Switcher::open(vec![], None, false).selected().is_none());
    }

    #[test]
    fn tab_wraps_and_crosses_pages() {
        let mut s = Switcher::open(cells(10), None, false);
        for _ in 0..7 {
            s.step(true);
        }
        assert_eq!((s.selected_index(), s.page()), (8, 1));
        s.step(true);
        s.step(true);
        assert_eq!(s.selected_index(), 0, "past the last window it is the first again");
        s.step(false);
        assert_eq!(s.selected_index(), 9);
    }

    /// 37 windows read "9–16 of 37 windows · Page 2 of 5", the last page is short, and a single
    /// page just counts.
    #[test]
    fn the_status_line_counts_pages_the_way_the_spec_words_it() {
        let mut s = Switcher::open(cells(37), None, false);
        s.selected = 10;
        assert_eq!(s.status(), "9–16 of 37 windows · Page 2 of 5");
        s.selected = 36;
        assert_eq!(s.status(), "33–37 of 37 windows · Page 5 of 5");
        assert_eq!(s.page_cells().len(), 5);
        assert_eq!(Switcher::open(cells(8), None, false).status(), "8 windows");
        assert_eq!(Switcher::open(cells(1), None, false).status(), "1 window");
    }

    /// Up and down follow the columns the card is drawn with and stay on their page; the arrows
    /// do not wrap, because a spatial move that jumps across the card is not spatial.
    #[test]
    fn arrows_move_by_the_drawn_columns_and_stay_on_the_page() {
        let mut s = Switcher::open(cells(20), None, false);
        s.selected = 1;
        s.go(Move::Down, 4);
        assert_eq!(s.selected_index(), 5);
        s.go(Move::Down, 4);
        assert_eq!(s.selected_index(), 5, "the row below is the next page's, so it stays");
        s.go(Move::Up, 4);
        assert_eq!(s.selected_index(), 1);
        s.go(Move::Up, 4);
        assert_eq!(s.selected_index(), 1);
        // Two columns: a row is two cells.
        s.go(Move::Down, 2);
        assert_eq!(s.selected_index(), 3);
    }

    #[test]
    fn page_keys_keep_the_place_and_clamp_to_a_short_last_page() {
        let mut s = Switcher::open(cells(20), None, false);
        s.selected = 6;
        s.go(Move::PageNext, 4);
        assert_eq!(s.selected_index(), 14);
        s.go(Move::PageNext, 4);
        assert_eq!(s.selected_index(), 19, "page 3 has four cells; the place clamps to its last");
        s.go(Move::PageNext, 4);
        assert_eq!(s.page(), 0);
        s.go(Move::PagePrev, 4);
        assert_eq!(s.page(), 2);
    }

    /// A cell on the page is chosen by its place on that page.
    #[test]
    fn pointing_picks_a_cell_on_the_page_being_shown() {
        let mut s = Switcher::open(cells(20), None, false);
        s.selected = 9;
        s.point_at(3);
        assert_eq!(s.selected_index(), 11);
        s.point_at(7);
        assert_eq!(s.selected_index(), 15);
        s.selected = 17;
        s.point_at(7);
        assert_eq!(s.selected_index(), 17, "no cell there on a short page: nothing changes");
    }

    #[test]
    fn every_word_the_surface_documents_parses() {
        for word in Move::WORDS.split(", ") {
            assert!(Move::parse(word).is_some(), "`{word}` is documented and not parsed");
        }
        assert!(Move::parse("sideways").is_none());
    }
}
