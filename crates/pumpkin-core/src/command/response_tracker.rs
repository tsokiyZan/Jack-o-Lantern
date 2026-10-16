use pumpkin_util::text::TextComponent;

use crate::command::CommandSource;
use crate::command::errors::command_syntax_error::CommandSyntaxError;
use crate::command::errors::error_types::CommandErrorType;

/// Vanilla `CommandResponseTracker`, counting the elements a command changed to pick its feedback.
pub struct CommandResponseTracker<T> {
    total_value: i32,
    only_non_zero_element: Option<T>,
    non_zero_element_count: i32,
}

impl<T> Default for CommandResponseTracker<T> {
    fn default() -> Self {
        Self {
            total_value: 0,
            only_non_zero_element: None,
            non_zero_element_count: 0,
        }
    }
}

impl<T> CommandResponseTracker<T> {
    pub fn track(&mut self, element: T, value: bool) {
        if value {
            self.total_value += 1;
            self.non_zero_element_count += 1;
            self.only_non_zero_element = (self.non_zero_element_count == 1).then_some(element);
        }
    }

    /// Vanilla `Messages.throwIfZero` for `ElementType.NON_ZERO`.
    pub fn throw_if_zero(
        &self,
        on_zero: &'static CommandErrorType<0>,
    ) -> Result<(), CommandSyntaxError> {
        if self.non_zero_element_count == 0 {
            return Err(on_zero.create_without_context());
        }
        Ok(())
    }

    /// Vanilla `sendFeedback` with `ElementType.NON_ZERO`.
    pub fn send_feedback(
        self,
        source: &CommandSource,
        broadcast: bool,
        on_single: impl FnOnce(T, i32) -> TextComponent,
        on_multiple: impl FnOnce(i32, i32) -> TextComponent,
    ) -> i32 {
        let message = match self.only_non_zero_element {
            Some(element) => on_single(element, self.total_value),
            None => on_multiple(self.non_zero_element_count, self.total_value),
        };
        source.send_feedback(message, broadcast);
        self.total_value
    }
}
