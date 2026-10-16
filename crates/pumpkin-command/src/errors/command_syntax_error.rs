use pumpkin_util::text::TextComponent;

use crate::errors::error_types::AnyCommandErrorType;

/// Brigadier `CommandSyntaxException.CONTEXT_AMOUNT`.
const CONTEXT_AMOUNT: usize = 10;

/// A struct detailing the context of a syntax error, including where it happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSyntaxErrorContext {
    pub input: String,
    pub cursor: usize,
}

/// Indicates an object that can provide context of a command syntax error from itself.
pub trait ContextProvider {
    fn context(&self) -> CommandSyntaxErrorContext;
}

impl ContextProvider for CommandSyntaxErrorContext {
    fn context(&self) -> CommandSyntaxErrorContext {
        self.clone()
    }
}

/// A struct detailing a syntax error.
///
/// Despite its name, this error can
/// also be caused during command execution.
///
/// However, most `CommandSyntaxError`s are thrown during
/// parsing, which carry an additional detail about where
/// they  occurred (in the command) compared to the
/// `CommandSyntaxError`s thrown during command execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSyntaxError {
    pub error_type: &'static dyn AnyCommandErrorType,
    pub message: TextComponent,
    pub context: Option<CommandSyntaxErrorContext>,
}

impl CommandSyntaxError {
    /// Brigadier `CommandSyntaxException.getMessage`: the message, then where in the input it
    /// happened.
    #[must_use]
    pub fn get_message(&self) -> String {
        let mut message = self.message.clone().get_java_text();
        if let Some(context) = &self.context {
            let cursor = context.input.len().min(context.cursor);
            message.push_str(" at position ");
            message.push_str(&context.cursor.to_string());
            message.push_str(": ");
            if cursor > CONTEXT_AMOUNT {
                message.push_str("...");
            }
            message.push_str(&context.input[cursor.saturating_sub(CONTEXT_AMOUNT)..cursor]);
            message.push_str("<--[HERE]");
        }
        message
    }

    /// Constructs a new [`CommandSyntaxError`] without any context of the error,
    /// and only the error message itself.
    ///
    /// This means this error will not print a context to the client, which
    /// includes the string and the location the error was caused.
    #[must_use]
    pub const fn create_without_context(
        error_type: &'static dyn AnyCommandErrorType,
        message: TextComponent,
    ) -> Self {
        Self {
            error_type,
            message,
            context: None,
        }
    }

    /// Constructs a new [`CommandSyntaxError`] with a given context of the error,
    /// which includes the string and the location the error was caused,
    /// along with the error message itself.
    #[must_use]
    pub fn create<C>(
        error_type: &'static dyn AnyCommandErrorType,
        message: TextComponent,
        context_provider: &C,
    ) -> Self
    where
        C: ContextProvider,
    {
        Self {
            error_type,
            message,
            context: Some(context_provider.context()),
        }
    }

    /// Returns whether this error's type is similar to the provided type.
    pub fn is(&self, error_type: &'static dyn AnyCommandErrorType) -> bool {
        self.error_type == error_type
    }
}
