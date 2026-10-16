//! Vanilla's `TextComponentTagVisitor` with `RichStyling`, behind `NbtUtils.toPrettyComponent`.

use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::java_fmt::{double_to_string, float_to_string};
use pumpkin_nbt::tag::{NbtTag, double_value_of, float_value_of, quote_and_escape};

use super::TextComponent;
use super::color::NamedColor;

const MAX_DEPTH: usize = 64;
const MAX_LENGTH: usize = 128;
const INLINE_LIST_THRESHOLD: usize = 8;
const NEWLINE: &str = "\n";
const ELEMENT_SPACING: &str = " ";

const KEY_COLOR: NamedColor = NamedColor::Aqua;
const STRING_COLOR: NamedColor = NamedColor::Green;
const NUMBER_COLOR: NamedColor = NamedColor::Gold;
const NUMBER_TYPE_COLOR: NamedColor = NamedColor::Red;

#[derive(Clone, Copy)]
enum Token {
    Folded,
    ElementSeparator,
    ListClose,
    ListOpen,
    ListTypeSeparator,
    StructClose,
    StructOpen,
    NameValueSeparator,
    ByteSuffix,
    ByteArrayPrefix,
    ShortSuffix,
    IntArrayPrefix,
    LongSuffix,
    LongArrayPrefix,
    FloatSuffix,
    DoubleSuffix,
}

impl Token {
    const fn text(self) -> &'static str {
        match self {
            Self::Folded => "<...>",
            Self::ElementSeparator => ",",
            Self::ListClose => "]",
            Self::ListOpen => "[",
            Self::ListTypeSeparator => ";",
            Self::StructClose => "}",
            Self::StructOpen => "{",
            Self::NameValueSeparator => ":",
            Self::ByteSuffix => "b",
            Self::ByteArrayPrefix => "B",
            Self::ShortSuffix => "s",
            Self::IntArrayPrefix => "I",
            Self::LongSuffix | Self::LongArrayPrefix => "L",
            Self::FloatSuffix => "f",
            Self::DoubleSuffix => "d",
        }
    }

    /// `RichStyling.token`.
    fn component(self) -> TextComponent {
        let text = TextComponent::text(self.text());
        match self {
            Self::Folded => text.color_named(NamedColor::Gray),
            Self::ByteSuffix
            | Self::ByteArrayPrefix
            | Self::ShortSuffix
            | Self::IntArrayPrefix
            | Self::LongSuffix
            | Self::LongArrayPrefix
            | Self::FloatSuffix
            | Self::DoubleSuffix => text.color_named(NUMBER_TYPE_COLOR),
            _ => text,
        }
    }
}

/// Vanilla `NbtUtils.toPrettyComponent`.
#[must_use]
pub fn to_pretty_component(tag: &NbtTag) -> TextComponent {
    TextComponentTagVisitor::new("").visit(tag)
}

/// Vanilla `TextComponentTagVisitor` with `RichStyling`. Keys print in the compound's iteration
/// order, as vanilla only sorts them with debug logging enabled.
pub struct TextComponentTagVisitor {
    indentation: &'static str,
    indent_depth: usize,
    depth: usize,
    result: TextComponent,
}

impl TextComponentTagVisitor {
    #[must_use]
    pub fn new(indentation: &'static str) -> Self {
        Self {
            indentation,
            indent_depth: 0,
            depth: 0,
            result: TextComponent::empty(),
        }
    }

    #[must_use]
    pub fn visit(mut self, tag: &NbtTag) -> TextComponent {
        self.accept(tag);
        self.result
    }

    fn append(&mut self, component: TextComponent) -> &mut Self {
        self.result.0.extra.push(component.0);
        self
    }

    fn append_text(&mut self, text: impl Into<String>) -> &mut Self {
        self.append(TextComponent::text(text.into()))
    }

    fn append_styled(&mut self, text: impl Into<String>, color: NamedColor) -> &mut Self {
        self.append(TextComponent::text(text.into()).color_named(color))
    }

    fn append_token(&mut self, token: Token) -> &mut Self {
        self.append(token.component())
    }

    fn accept(&mut self, tag: &NbtTag) {
        match tag {
            NbtTag::End => {}
            NbtTag::String(value) => self.visit_string(value),
            NbtTag::Byte(v) => {
                self.append_styled(v.to_string(), NUMBER_COLOR)
                    .append_token(Token::ByteSuffix);
            }
            NbtTag::Short(v) => {
                self.append_styled(v.to_string(), NUMBER_COLOR)
                    .append_token(Token::ShortSuffix);
            }
            NbtTag::Int(v) => {
                self.append_styled(v.to_string(), NUMBER_COLOR);
            }
            NbtTag::Long(v) => {
                self.append_styled(v.to_string(), NUMBER_COLOR)
                    .append_token(Token::LongSuffix);
            }
            NbtTag::Float(v) => {
                self.append_styled(float_to_string(float_value_of(*v)), NUMBER_COLOR)
                    .append_token(Token::FloatSuffix);
            }
            NbtTag::Double(v) => {
                self.append_styled(double_to_string(double_value_of(*v)), NUMBER_COLOR)
                    .append_token(Token::DoubleSuffix);
            }
            NbtTag::ByteArray(data) => {
                self.visit_array(Token::ByteArrayPrefix, data, Some(Token::ByteSuffix));
            }
            NbtTag::IntArray(data) => self.visit_array(Token::IntArrayPrefix, data, None),
            NbtTag::LongArray(data) => {
                self.visit_array(Token::LongArrayPrefix, data, Some(Token::LongSuffix));
            }
            NbtTag::List(list) => self.visit_list(list),
            NbtTag::Compound(compound) => self.visit_compound(compound),
        }
    }

    fn visit_string(&mut self, value: &str) {
        let quoted = quote_and_escape(value);
        let quote = &quoted[..1];
        let inner = &quoted[1..quoted.len() - 1];
        self.append_text(quote)
            .append_styled(inner, STRING_COLOR)
            .append_text(quote);
    }

    /// `visitByteArray`, `visitIntArray` and `visitLongArray`.
    fn visit_array<T: ToString>(&mut self, prefix: Token, data: &[T], suffix: Option<Token>) {
        self.append_token(Token::ListOpen)
            .append_token(prefix)
            .append_token(Token::ListTypeSeparator);
        for (i, value) in data.iter().enumerate().take(MAX_LENGTH) {
            self.append_text(ELEMENT_SPACING)
                .append_styled(value.to_string(), NUMBER_COLOR);
            if let Some(suffix) = suffix {
                self.append_token(suffix);
            }
            if i != data.len() - 1 {
                self.append_token(Token::ElementSeparator);
            }
        }
        if data.len() > MAX_LENGTH {
            self.append_token(Token::Folded);
        }
        self.append_token(Token::ListClose);
    }

    fn should_wrap_list_elements(list: &[NbtTag]) -> bool {
        if list.len() >= INLINE_LIST_THRESHOLD {
            return false;
        }
        list.iter().any(|element| {
            !matches!(
                element,
                NbtTag::Byte(_)
                    | NbtTag::Short(_)
                    | NbtTag::Int(_)
                    | NbtTag::Long(_)
                    | NbtTag::Float(_)
                    | NbtTag::Double(_)
            )
        })
    }

    fn visit_list(&mut self, list: &[NbtTag]) {
        if list.is_empty() {
            self.append_token(Token::ListOpen)
                .append_token(Token::ListClose);
        } else if self.depth >= MAX_DEPTH {
            self.append_token(Token::ListOpen)
                .append_token(Token::Folded)
                .append_token(Token::ListClose);
        } else if !Self::should_wrap_list_elements(list) {
            self.append_token(Token::ListOpen);
            for (i, element) in list.iter().enumerate() {
                if i != 0 {
                    self.append_token(Token::ElementSeparator)
                        .append_text(ELEMENT_SPACING);
                }
                self.append_sub_tag(element, false);
            }
            self.append_token(Token::ListClose);
        } else {
            self.append_token(Token::ListOpen);
            if !self.indentation.is_empty() {
                self.append_text(NEWLINE);
            }
            let entry_indent = self.indentation.repeat(self.indent_depth + 1);
            let element_spacing = if self.indentation.is_empty() {
                ELEMENT_SPACING
            } else {
                NEWLINE
            };
            for (i, element) in list.iter().enumerate().take(MAX_LENGTH) {
                self.append_text(entry_indent.clone());
                self.append_sub_tag(element, true);
                if i != list.len() - 1 {
                    self.append_token(Token::ElementSeparator)
                        .append_text(element_spacing);
                }
            }
            if list.len() > MAX_LENGTH {
                self.append_text(entry_indent).append_token(Token::Folded);
            }
            if !self.indentation.is_empty() {
                let close_indent = self.indentation.repeat(self.indent_depth);
                self.append_text(NEWLINE).append_text(close_indent);
            }
            self.append_token(Token::ListClose);
        }
    }

    fn visit_compound(&mut self, compound: &NbtCompound) {
        if compound.is_empty() {
            self.append_token(Token::StructOpen)
                .append_token(Token::StructClose);
        } else if self.depth >= MAX_DEPTH {
            self.append_token(Token::StructOpen)
                .append_token(Token::Folded)
                .append_token(Token::StructClose);
        } else {
            self.append_token(Token::StructOpen);
            if !self.indentation.is_empty() {
                self.append_text(NEWLINE);
            }
            let entry_indent = self.indentation.repeat(self.indent_depth + 1);
            let element_spacing = if self.indentation.is_empty() {
                ELEMENT_SPACING
            } else {
                NEWLINE
            };
            let mut entries = compound.child_tags.iter().peekable();
            while let Some((key, value)) = entries.next() {
                let key = Self::handle_escape_pretty(key);
                self.append_text(entry_indent.clone())
                    .append(key)
                    .append_token(Token::NameValueSeparator)
                    .append_text(ELEMENT_SPACING);
                self.append_sub_tag(value, true);
                if entries.peek().is_some() {
                    self.append_token(Token::ElementSeparator)
                        .append_text(element_spacing);
                }
            }
            if !self.indentation.is_empty() {
                let close_indent = self.indentation.repeat(self.indent_depth);
                self.append_text(NEWLINE).append_text(close_indent);
            }
            self.append_token(Token::StructClose);
        }
    }

    fn append_sub_tag(&mut self, tag: &NbtTag, indent: bool) {
        if indent {
            self.indent_depth += 1;
        }
        self.depth += 1;
        self.accept(tag);
        if indent {
            self.indent_depth -= 1;
        }
        self.depth -= 1;
    }

    fn handle_escape_pretty(input: &str) -> TextComponent {
        let simple = !input.is_empty()
            && input
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'));
        if simple {
            return TextComponent::text(input.to_owned()).color_named(KEY_COLOR);
        }
        let quoted = quote_and_escape(input);
        let quote = quoted[..1].to_owned();
        let inner =
            TextComponent::text(quoted[1..quoted.len() - 1].to_owned()).color_named(KEY_COLOR);
        TextComponent::text(quote.clone())
            .add_child(inner)
            .add_child(TextComponent::text(quote))
    }
}
