//! The Reset dialog: what it says, and which choice each button makes.

use app::reset::{DialogOutcome, Reset, dialog};
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
