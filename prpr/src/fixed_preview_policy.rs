/// Whether Fixed Preview Time owns the visual timing and flow of this note.
pub fn applies_to_note(fake: bool) -> bool {
    !fake
}

/// A render-only, per-Note decision. Original deliberately has no time gate.
/// All Hold pieces consume the same plan; no chart or judgement data is edited.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PreviewPlan {
    Constant { flow: f32, start: f64 },
    GateOnly { original_flow: f32, start: f64 },
    Original { original_flow: f32, reason: FallbackReason },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FallbackReason {
    Disabled,
    FakeNote,
    InvalidTarget,
    ValidationBudget,
    NoBracket,
    VisibilityGap,
    MotionSpike,
    AuthoredVisibility,
}

impl FallbackReason {
    pub fn name(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::FakeNote => "fake_note_not_applicable",
            Self::InvalidTarget => "target_before_chart_or_nonfinite",
            Self::ValidationBudget => "validation_budget_exceeded",
            Self::NoBracket => "no_valid_bracket",
            Self::VisibilityGap => "visibility_interval_rejected",
            Self::MotionSpike => "motion_spike",
            Self::AuthoredVisibility => "authored_visibility_at_target",
        }
    }
}

impl PreviewPlan {
    #[inline]
    pub fn original(original_flow: f32, reason: FallbackReason) -> Self {
        Self::Original { original_flow, reason }
    }

    #[inline]
    pub fn flow(self) -> f32 {
        match self {
            Self::Constant { flow, .. } => flow,
            Self::GateOnly { original_flow, .. } | Self::Original { original_flow, .. } => original_flow,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Constant { .. } => "constant",
            Self::GateOnly { .. } => "gate_only",
            Self::Original { .. } => "original",
        }
    }

    pub fn reason(self) -> &'static str {
        match self {
            Self::Constant { .. } => "solved",
            Self::GateOnly { .. } => "gate_only",
            Self::Original { reason, .. } => reason.name(),
        }
    }

    /// Constant time/space hot path. Fake notes always retain authored timing,
    /// even if a stale/misaligned caller were to supply a gated plan.
    #[inline]
    pub fn hides(self, fake: bool, now: f64) -> bool {
        !fake
            && match self {
                Self::Constant { start, .. } | Self::GateOnly { start, .. } => now < start,
                Self::Original { .. } => false,
            }
    }
}
