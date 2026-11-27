use super::*;

fn id() -> ProfileId {
    ProfileId::parse(&format!("prof_{}", "a".repeat(32))).expect("id")
}

fn budget(room_limit: u32, account_limit: u32) -> WakeBudget {
    WakeBudget {
        room_limit,
        room_window: Duration::from_secs(600),
        account_limit,
        account_window: Duration::from_secs(3600),
        exempt_senders: Vec::new(),
    }
}

fn plain<'a>(rooms: &[&'a str]) -> Vec<Offer<'a>> {
    rooms
        .iter()
        .map(|room| Offer {
            room_id: room,
            exempt: false,
        })
        .collect()
}

#[test]
fn the_row_after_the_room_limit_is_held_and_noticed_once() {
    let budgets = Budgets::default();
    let now = Instant::now();
    let first = budgets.admit(
        &id(),
        &budget(3, 100),
        &plain(&["!a", "!a", "!a", "!a"]),
        now,
    );
    assert_eq!(first.admitted, 3);
    assert!(first.notify);
    let held = first.held.expect("held");
    assert_eq!(
        (held.window, held.room_id.as_str()),
        (BudgetWindow::Room, "!a")
    );
    // The same row offered again is still held, and is not announced again.
    let again = budgets.admit(&id(), &budget(3, 100), &plain(&["!a"]), now);
    assert_eq!((again.admitted, again.notify), (0, false));
    // A different room has its own count.
    let other = budgets.admit(&id(), &budget(3, 100), &plain(&["!b"]), now);
    assert_eq!(other.admitted, 1);
}

#[test]
fn the_account_window_counts_every_room() {
    let budgets = Budgets::default();
    let now = Instant::now();
    let rooms = ["!a", "!b", "!c"];
    let outcome = budgets.admit(&id(), &budget(10, 2), &plain(&rooms), now);
    assert_eq!(outcome.admitted, 2);
    assert_eq!(outcome.held.expect("held").window, BudgetWindow::Account);
}

#[test]
fn rows_are_admitted_again_once_the_window_has_passed() {
    let budgets = Budgets::default();
    let now = Instant::now();
    budgets.admit(&id(), &budget(1, 100), &plain(&["!a"]), now);
    let held = budgets.admit(&id(), &budget(1, 100), &plain(&["!a"]), now);
    assert_eq!(held.admitted, 0);
    let retry = held.held.expect("held").retry_after;
    assert_eq!(retry, Duration::from_secs(600));
    let later = now + Duration::from_secs(601);
    let after = budgets.admit(&id(), &budget(1, 100), &plain(&["!a"]), later);
    assert_eq!(after.admitted, 1);
    // A new hold is announced again.
    let next = budgets.admit(&id(), &budget(1, 100), &plain(&["!a"]), later);
    assert!(next.notify);
}

#[test]
fn a_reset_forgets_the_window() {
    let budgets = Budgets::default();
    let now = Instant::now();
    budgets.admit(&id(), &budget(1, 100), &plain(&["!a"]), now);
    assert_eq!(
        budgets
            .admit(&id(), &budget(1, 100), &plain(&["!a"]), now)
            .admitted,
        0
    );
    budgets.reset(&id());
    assert_eq!(
        budgets
            .admit(&id(), &budget(1, 100), &plain(&["!a"]), now)
            .admitted,
        1
    );
}

#[test]
fn an_exempt_row_is_not_counted_and_releases_the_rows_before_it() {
    let budgets = Budgets::default();
    let now = Instant::now();
    budgets.admit(&id(), &budget(1, 100), &plain(&["!a"]), now);
    let offers = [
        Offer {
            room_id: "!a",
            exempt: false,
        },
        Offer {
            room_id: "!a",
            exempt: true,
        },
        Offer {
            room_id: "!a",
            exempt: false,
        },
    ];
    let outcome = budgets.admit(&id(), &budget(1, 100), &offers, now);
    assert_eq!(
        outcome.admitted, 2,
        "held row and the owner DM go; the next waits"
    );
    assert!(outcome.held.is_some());
}

#[test]
fn a_stored_budget_that_cannot_be_read_is_the_default() {
    assert_eq!(
        WakeBudget::from_json(&json!({"room_limit": 0})),
        WakeBudget::default()
    );
    let stored = budget(5, 9);
    assert_eq!(WakeBudget::from_json(&stored.to_json()), stored);
    assert!(!budget(0, 1).is_valid());
}
