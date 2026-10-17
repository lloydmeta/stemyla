//! The Reset dialog: what it says, and which choice each button makes.

use app::reset::{
    DialogOutcome, ProgressOutcome, Reset, dialog, not_deleted, not_refreshed, progress,
};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

/// The dialog's outcome after the last frame, with the given reason.
fn harness(reason: Option<&'static str>) -> Harness<'static, DialogOutcome> {
    let mut h = Harness::builder().with_size([800.0, 600.0]).build_ui_state(
        move |ui, outcome: &mut DialogOutcome| {
            if *outcome == DialogOutcome::Open {
                *outcome = dialog(ui.ctx(), reason, 2, 512.4);
            }
        },
        DialogOutcome::Open,
    );
    h.run();
    h
}

#[test]
fn shows_the_reason_and_what_deleting_removes() {
    let h = harness(Some("RuntimeError: unreachable"));
    assert!(
        h.query_by_label_contains("The separation worker stopped: RuntimeError: unreachable")
            .is_some()
    );
    assert!(
        h.query_by_label_contains("2 kept songs with their stems, 512 MB in all")
            .is_some()
    );
    assert_eq!(*h.state(), DialogOutcome::Open);
}

#[test]
fn opened_from_the_storage_menu_it_gives_no_reason() {
    let h = harness(None);
    assert!(h.query_by_label_contains("worker stopped").is_none());
}

#[test]
fn refresh_keeps_stored_data() {
    let mut h = harness(Some("its script did not load"));
    h.get_by_label("Refresh app files and reload").click();
    h.run();
    assert_eq!(*h.state(), DialogOutcome::Chosen(Reset::RefreshFiles));
}

#[test]
fn delete_is_its_own_button() {
    let mut h = harness(None);
    h.get_by_label("Delete stored data, refresh and reload")
        .click();
    h.run();
    assert_eq!(*h.state(), DialogOutcome::Chosen(Reset::DeleteStored));
}

#[test]
fn cancel_and_escape_close_without_a_reset() {
    let mut h = harness(None);
    h.get_by_label("Cancel").click();
    h.run();
    assert_eq!(*h.state(), DialogOutcome::Cancelled);

    let mut h = harness(None);
    h.key_press(eframe::egui::Key::Escape);
    h.run();
    assert_eq!(*h.state(), DialogOutcome::Cancelled);
}

/// The progress dialog's outcome after the last frame.
fn progress_harness(failure: Option<&'static str>) -> Harness<'static, ProgressOutcome> {
    let mut h = Harness::builder().with_size([800.0, 600.0]).build_ui_state(
        move |ui, outcome: &mut ProgressOutcome| {
            if *outcome == ProgressOutcome::Waiting {
                *outcome = progress(ui.ctx(), failure);
            }
        },
        ProgressOutcome::Waiting,
    );
    h.run_steps(2);
    h
}

#[test]
fn while_running_there_is_nothing_to_press() {
    let mut h = progress_harness(None);
    assert!(
        h.query_by_label_contains("Resetting, then reloading")
            .is_some()
    );
    assert!(h.query_by_label("Try again").is_none());
    assert!(h.query_by_label("Reload").is_none());
    h.key_press(eframe::egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(*h.state(), ProgressOutcome::Waiting);
}

#[test]
fn a_failed_deletion_is_shown_and_waits_for_the_user() {
    let failure = not_deleted(&["sessions".to_string(), "download.part".to_string()]);
    assert!(failure.starts_with("Could not delete from browser storage: sessions, download.part."));
    assert!(failure.contains("Another tab"));
    let failure: &'static str = Box::leak(failure.into_boxed_str());
    let mut h = progress_harness(Some(failure));
    assert!(h.query_by_label(failure).is_some());
    h.get_by_label("Try again").click();
    h.run_steps(2);
    assert_eq!(*h.state(), ProgressOutcome::TryAgain);

    let mut h = progress_harness(Some(failure));
    h.get_by_label("Reload").click();
    h.run_steps(2);
    assert_eq!(*h.state(), ProgressOutcome::Reload);
}

/// A file that could not be fetched again is named, and the user is asked
/// to retry rather than reload into the outdated copies.
#[test]
fn a_failed_refetch_names_the_files_and_asks_for_a_retry() {
    let failure = not_refreshed(&[
        "https://beachape.com/stemyla/?selftest".to_string(),
        "https://beachape.com/stemyla/stem-worker.js".to_string(),
        "https://beachape.com/stemyla/snippets/app-0cd1cead3f0174fd/inline1.js#x".to_string(),
    ]);
    assert!(
        failure.starts_with("Could not fetch again: the page, stem-worker.js, inline1.js."),
        "{failure}"
    );
    assert!(failure.contains("try again"));
    let failure: &'static str = Box::leak(failure.into_boxed_str());
    let mut h = progress_harness(Some(failure));
    assert!(h.query_by_label(failure).is_some());
    h.get_by_label("Try again").click();
    h.run_steps(2);
    assert_eq!(*h.state(), ProgressOutcome::TryAgain);
}
