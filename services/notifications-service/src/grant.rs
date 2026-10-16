//! Whether a mind's notice interrupts (a toast) or waits in the inbox.
//!
//! Decided once, by the store, when it stores the card, and recorded with the reason
//! (design/initiative-inbox-2026-10-16.md, "How the OS decides"). The mind asks; the OS spends
//! the attention, from one budget shared by every mind, because the OS is one entity with many
//! minds. A pure function of its inputs, so every rule here is tested without a clock or a file.

use chrono::{DateTime, Local};
use yantrik_ipc_contracts::notifications::notice::{
    Attention, Family, FamilySetting, GrantReason, Initiative, DAILY_INTERRUPTS,
};

/// What the OS decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub granted: Attention,
    pub reason: GrantReason,
    /// Quiet hours held an interrupt until then.
    pub held_until: Option<DateTime<Local>>,
    /// Whether this spends one of the day's interrupts.
    pub spends: bool,
}

impl Decision {
    fn inbox(reason: GrantReason) -> Decision {
        Decision { granted: Attention::Inbox, reason, held_until: None, spends: false }
    }
}

/// Decide, in the spec's order.
///
/// `used` is how many interrupts have been granted today, across every harness.
pub fn decide(family: &Family, asked: &Attention, settings: &Initiative, now: DateTime<Local>, used: u32) -> Decision {
    let setting = settings.setting(family);
    // 1. A family the person switched off: stored, never shown, answered as stored so the mind
    //    stops retrying. Only an initiative family can be off.
    if setting == FamilySetting::Off && !family.is_obligation() {
        return Decision::inbox(GrantReason::FamilyOff);
    }
    // 2. The mind asked for the inbox.
    if *asked != Attention::Interrupt {
        return Decision::inbox(GrantReason::AskedInbox);
    }
    let rest = after_quiet_hours(family, setting, settings.quiet, used);
    // 3. Quiet hours apply to every family, obligations included. The card waits in the inbox;
    //    one that would have interrupted is toasted when they end, if it is still useful then.
    //    It is counted against the budget now, so the morning cannot bring more than the day
    //    allows.
    if let Some(end) = settings.quiet_hours.and_then(|q| q.ends_after(now)) {
        let would = rest.granted == Attention::Interrupt;
        return Decision {
            granted: Attention::Inbox,
            reason: GrantReason::QuietHours,
            held_until: would.then_some(end),
            spends: would && rest.spends,
        };
    }
    rest
}

/// Steps 4 to 7.
fn after_quiet_hours(family: &Family, setting: FamilySetting, quiet: bool, used: u32) -> Decision {
    // 4. What the person asked for skips the initiative rules and the budget, though their
    //    own "Inbox only" for it still holds: that row offers it, so it has to mean something.
    if family.is_obligation() {
        if setting == FamilySetting::InboxOnly {
            return Decision::inbox(GrantReason::FamilyInboxOnly);
        }
        return Decision { granted: Attention::Interrupt, reason: GrantReason::Granted, held_until: None, spends: false };
    }
    // 5. Quiet initiative.
    if quiet {
        return Decision::inbox(GrantReason::QuietInitiative);
    }
    // 6. This family is inbox-only.
    if setting == FamilySetting::InboxOnly {
        return Decision::inbox(GrantReason::FamilyInboxOnly);
    }
    // 7. The day's budget.
    if used >= DAILY_INTERRUPTS {
        return Decision::inbox(GrantReason::BudgetSpent);
    }
    Decision { granted: Attention::Interrupt, reason: GrantReason::Granted, held_until: None, spends: true }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use yantrik_ipc_contracts::notifications::notice::QuietHours;

    fn at(hour: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 10, 16, hour, 30, 0).earliest().unwrap()
    }

    #[test]
    fn the_third_interrupt_of_the_day_goes_to_the_inbox() {
        let s = Initiative::default();
        let first = decide(&Family::Discovery, &Attention::Interrupt, &s, at(10), 0);
        assert_eq!((first.granted, first.spends), (Attention::Interrupt, true));
        assert_eq!(decide(&Family::Calendar, &Attention::Interrupt, &s, at(10), 1).granted, Attention::Interrupt);
        let third = decide(&Family::Machine, &Attention::Interrupt, &s, at(10), 2);
        assert_eq!((third.granted, third.reason), (Attention::Inbox, GrantReason::BudgetSpent));
        // A reminder is something the person asked for: it does not spend, and is not refused by,
        // the budget.
        let reminder = decide(&Family::Reminder, &Attention::Interrupt, &s, at(10), 2);
        assert_eq!((reminder.granted, reminder.spends), (Attention::Interrupt, false));
    }

    #[test]
    fn quiet_hours_hold_a_reminder_until_they_end() {
        let s = Initiative { quiet_hours: Some(QuietHours { start: 22, end: 7 }), ..Default::default() };
        let held = decide(&Family::Reminder, &Attention::Interrupt, &s, at(23), 0);
        assert_eq!((held.granted.clone(), held.reason.clone()), (Attention::Inbox, GrantReason::QuietHours));
        let end = held.held_until.expect("toasted when quiet hours end");
        assert_eq!(end, Local.with_ymd_and_hms(2026, 10, 17, 7, 0, 0).earliest().unwrap());
        // Outside them, it interrupts.
        assert_eq!(decide(&Family::Reminder, &Attention::Interrupt, &s, at(12), 0).granted, Attention::Interrupt);
        // An inbox card in quiet hours is not held for a toast.
        assert_eq!(decide(&Family::Reminder, &Attention::Inbox, &s, at(23), 0).held_until, None);
    }

    #[test]
    fn quiet_initiative_holds_discovery_even_when_it_asks_to_interrupt() {
        let s = Initiative { quiet: true, ..Default::default() };
        let d = decide(&Family::Discovery, &Attention::Interrupt, &s, at(10), 0);
        assert_eq!((d.granted, d.reason), (Attention::Inbox, GrantReason::QuietInitiative));
        // Obligations still arrive.
        assert_eq!(decide(&Family::Delegated, &Attention::Interrupt, &s, at(10), 0).granted, Attention::Interrupt);
    }

    #[test]
    fn a_family_set_off_is_stored_hidden_and_inbox_only_is_honoured() {
        let mut s = Initiative::default();
        s.set(&Family::Discovery, FamilySetting::Off).unwrap();
        s.set(&Family::Calendar, FamilySetting::InboxOnly).unwrap();
        s.set(&Family::Reminder, FamilySetting::InboxOnly).unwrap();
        assert_eq!(decide(&Family::Discovery, &Attention::Inbox, &s, at(10), 0).reason, GrantReason::FamilyOff);
        assert_eq!(decide(&Family::Calendar, &Attention::Interrupt, &s, at(10), 0).reason, GrantReason::FamilyInboxOnly);
        assert_eq!(decide(&Family::Reminder, &Attention::Interrupt, &s, at(10), 0).reason, GrantReason::FamilyInboxOnly);
        assert_eq!(decide(&Family::Machine, &Attention::Inbox, &s, at(10), 0).reason, GrantReason::AskedInbox);
    }
}
