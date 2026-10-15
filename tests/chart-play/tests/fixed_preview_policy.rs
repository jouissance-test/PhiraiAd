use phira_chart_play_tests::fixed_preview_policy::{applies_to_note, hide_before_target};


#[test]
fn fake_notes_keep_their_authored_show_timing_and_flow() {
    assert!(!applies_to_note(true));
    for now in [0., 30., 60.] {
        assert!(!hide_before_target(true, now, 100., Some(1.5)));
    }
}

#[test]
fn regular_notes_still_obey_the_fixed_preview_safety_gate() {
    assert!(applies_to_note(false));
    assert!(hide_before_target(false, 8.49, 10., Some(1.5)));
    assert!(!hide_before_target(false, 8.5, 10., Some(1.5)));
    assert!(!hide_before_target(false, 0., 10., None));
}
