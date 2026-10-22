//! Vanilla `InteractionResult`: what an entity's `interact`, a mob's `mobInteract` or an item's
//! `interactLivingEntity` did with a right click.

/// Vanilla `InteractionResult.SwingSource`: who swings the arm after a success.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwingSource {
    /// `NONE`: nobody swings.
    None,
    /// `PREDICTED`: the client already swung, so only the players tracking it see the swing.
    Client,
    /// `SERVER_ONLY`: the server swings for everyone, the swinging player included.
    Server,
}

/// Vanilla `InteractionResult`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InteractionResult {
    Success { swing: SwingSource },
    Fail,
    Pass,
    TryEmptyHandInteraction,
}

impl InteractionResult {
    /// Vanilla `InteractionResult.SUCCESS`.
    pub const SUCCESS: Self = Self::Success {
        swing: SwingSource::Client,
    };
    /// Vanilla `InteractionResult.SUCCESS_SERVER`.
    pub const SUCCESS_SERVER: Self = Self::Success {
        swing: SwingSource::Server,
    };
    /// Vanilla `InteractionResult.CONSUME`.
    pub const CONSUME: Self = Self::Success {
        swing: SwingSource::None,
    };
    /// Vanilla `InteractionResult.FAIL`.
    pub const FAIL: Self = Self::Fail;
    /// Vanilla `InteractionResult.PASS`.
    pub const PASS: Self = Self::Pass;
    /// Vanilla `InteractionResult.TRY_WITH_EMPTY_HAND`.
    pub const TRY_WITH_EMPTY_HAND: Self = Self::TryEmptyHandInteraction;

    /// Vanilla `consumesAction`: only a success takes the click.
    #[must_use]
    pub const fn consumes_action(self) -> bool {
        matches!(self, Self::Success { .. })
    }

    /// The swing a success asks for; `None` for anything else.
    #[must_use]
    pub const fn swing_source(self) -> SwingSource {
        match self {
            Self::Success { swing } => swing,
            _ => SwingSource::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{InteractionResult, SwingSource};

    #[test]
    fn only_successes_consume() {
        assert!(InteractionResult::SUCCESS.consumes_action());
        assert!(InteractionResult::SUCCESS_SERVER.consumes_action());
        assert!(InteractionResult::CONSUME.consumes_action());
        assert!(!InteractionResult::FAIL.consumes_action());
        assert!(!InteractionResult::PASS.consumes_action());
        assert!(!InteractionResult::TRY_WITH_EMPTY_HAND.consumes_action());
    }

    #[test]
    fn swing_sources_match_vanilla_constants() {
        assert_eq!(
            InteractionResult::SUCCESS.swing_source(),
            SwingSource::Client
        );
        assert_eq!(
            InteractionResult::SUCCESS_SERVER.swing_source(),
            SwingSource::Server
        );
        assert_eq!(InteractionResult::CONSUME.swing_source(), SwingSource::None);
        assert_eq!(InteractionResult::FAIL.swing_source(), SwingSource::None);
    }
}
