/// Whether Fixed Preview Time owns the visual timing and flow of this note.
pub fn applies_to_note(fake: bool) -> bool {
    !fake
}

/// Temporal safety gate used immediately before the real note renderer.
pub fn hide_before_target(fake: bool, now: f64, note_time: f64, preview_chart_time: Option<f64>) -> bool {
    applies_to_note(fake) && preview_chart_time.is_some_and(|preview| now < note_time - preview)
}
