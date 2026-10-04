//! The newest memories for the Memory screen, by the same domain rule recall uses.
//!
//! VM 520 sweep, 4 October: the Memory screen was blank until something was searched, over a
//! store of about 1,036 memories. It opens on the newest ones now. What recall leaves out is
//! left out here too, by its own definition (`memory_evolution::is_excluded_domain`: audit/,
//! system/, self-reflection): the tools' audit lines are the companion's log, not something it
//! remembers about the person, and on a busy machine they would be every row.
use yantrik_companion::memory_evolution::is_excluded_domain;

/// Rows read per page while collecting.
const PAGE: usize = 100;

/// Collect up to `limit` of the newest memories whose domain recall would keep, reading `fetch`
/// a page at a time until there are enough or the store runs out. A page of nothing but audit
/// lines does not make the screen come back short (security review of #611: one fetch of
/// `limit * 5` did, whenever the newest hundred rows were all excluded).
///
/// `fetch(offset, page)` returns that page, newest first, and the total row count.
pub fn collect<T, E>(
    limit: usize,
    domain_of: impl Fn(&T) -> &str,
    mut fetch: impl FnMut(usize, usize) -> Result<(Vec<T>, usize), E>,
) -> Result<Vec<T>, E> {
    let mut kept = Vec::with_capacity(limit);
    let mut offset = 0;
    while kept.len() < limit {
        let (rows, total) = fetch(offset, PAGE)?;
        let read = rows.len();
        kept.extend(rows.into_iter().filter(|m| !is_excluded_domain(domain_of(m))));
        offset += read;
        if read == 0 || offset >= total {
            break;
        }
    }
    kept.truncate(limit);
    Ok(kept)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A store of `n` rows, newest first, whose domains come from `domain(i)`.
    fn store(n: usize, domain: impl Fn(usize) -> &'static str) -> Vec<(usize, &'static str)> {
        (0..n).map(|i| (i, domain(i))).collect()
    }

    fn page(rows: &[(usize, &'static str)], offset: usize, size: usize) -> Result<(Vec<(usize, &'static str)>, usize), ()> {
        Ok((rows.iter().skip(offset).take(size).cloned().collect(), rows.len()))
    }

    #[test]
    fn a_long_run_of_audit_lines_does_not_shorten_the_page() {
        // The newest 250 are tool-audit lines; the person's memories start after them.
        let rows = store(400, |i| if i < 250 { "audit/tools" } else { "general" });
        let got = collect(20, |r: &(usize, &str)| r.1, |o, s| page(&rows, o, s)).unwrap();
        assert_eq!(got.len(), 20);
        assert_eq!(got[0].0, 250, "newest kept first");
    }

    #[test]
    fn recalls_own_exclusions_are_left_out() {
        let domains = ["audit/tools", "system/boot", "self-reflection", "work", "people"];
        let rows = store(50, |i| domains[i % domains.len()]);
        let got = collect(20, |r: &(usize, &str)| r.1, |o, s| page(&rows, o, s)).unwrap();
        assert_eq!(got.len(), 20);
        assert!(got.iter().all(|r| r.1 == "work" || r.1 == "people"), "{got:?}");
    }

    #[test]
    fn a_small_store_returns_what_it_has() {
        let rows = store(7, |i| if i % 2 == 0 { "audit/tools" } else { "general" });
        let got = collect(20, |r: &(usize, &str)| r.1, |o, s| page(&rows, o, s)).unwrap();
        assert_eq!(got.len(), 3);
        let empty: Vec<(usize, &'static str)> = vec![];
        assert!(collect(20, |r: &(usize, &str)| r.1, |o, s| page(&empty, o, s)).unwrap().is_empty());
    }
}
