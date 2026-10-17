use phira_chart_play_tests::fixed_preview_policy::{applies_to_note, FallbackReason, PreviewPlan};

#[test]
fn fake_notes_keep_their_authored_show_timing_and_flow() {
    assert!(!applies_to_note(true));
    for now in [0., 30., 60.] {
        assert!(!PreviewPlan::Constant { flow: 6., start: 98.5 }.hides(true, now));
        assert!(!PreviewPlan::GateOnly {
            original_flow: 1.,
            start: 98.5
        }
        .hides(true, now));
    }
}

#[test]
fn regular_notes_still_obey_the_fixed_preview_safety_gate() {
    assert!(applies_to_note(false));
    for plan in [
        PreviewPlan::Constant { flow: 3., start: 8.5 },
        PreviewPlan::GateOnly {
            original_flow: 1.,
            start: 8.5,
        },
    ] {
        assert!(plan.hides(false, 8.49));
        assert!(!plan.hides(false, 8.5));
    }
    assert!(!PreviewPlan::original(1., FallbackReason::Disabled).hides(false, 0.));
}

#[test]
fn every_original_fallback_is_ungated_even_when_fixed_preview_is_enabled() {
    for reason in [
        FallbackReason::FakeNote,
        FallbackReason::InvalidTarget,
        FallbackReason::ValidationBudget,
        FallbackReason::NoBracket,
        FallbackReason::VisibilityGap,
        FallbackReason::MotionSpike,
    ] {
        let plan = PreviewPlan::original(0.7, reason);
        assert_eq!(plan.flow(), 0.7);
        assert!(!plan.hides(false, -100.));
        assert!(!plan.hides(false, 10.));
    }
}
