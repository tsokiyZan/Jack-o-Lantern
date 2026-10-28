//! Storage and convenience methods for NBT compound tags.

use std::fmt::{Display, Formatter};
use uuid::Uuid;

use crate::deserializer::NbtReadHelper;
use crate::java_fmt::{double_to_string, float_to_string};
use crate::java_hash_map::JavaHashMap;
use crate::serializer::NbtWriteHelper;
use crate::tag::{NbtTag, double_value_of, float_value_of, quote_and_escape};
use crate::{END_ID, Error, Nbt};
use std::io::ErrorKind;

#[macro_export]
/// Creates an [`NbtTag::Compound`](crate::tag::NbtTag::Compound) from key-value pairs.
///
/// The macro also accepts an empty invocation to create an empty compound tag.
macro_rules! nbt_compound_tag {
    { $($key:literal : $tag:expr),+ $(,)* } => {
        {
            let mut compound = NbtCompound::new();
            $( compound.put($key, $tag); )+
            NbtTag::Compound(compound)
        }
    };
    // For empty compounds
    {} => {
        NbtTag::Compound(NbtCompound::new())
    };
}

/// Represents a Compound NBT tag, effectively a hash map.
///
/// Its tags iterate in the order of the Java `HashMap` behind vanilla's `CompoundTag`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NbtCompound {
    /// Tags in the compound, indexed by their names.
    pub child_tags: JavaHashMap<NbtTag>,
}

impl NbtCompound {
    /// Creates an empty compound.
    #[must_use]
    pub fn new() -> Self {
        Self {
            child_tags: JavaHashMap::new(),
        }
    }

    /// Advances a reader past a compound's payload without allocating its tags.
    pub fn skip_content<'a, R: NbtReadHelper<'a>>(reader: &mut R) -> Result<(), Error> {
        Self::skip_content_depth(reader, 0)
    }

    /// Advances a reader past a compound's payload with depth tracking.
    pub fn skip_content_depth<'a, R: NbtReadHelper<'a>>(
        reader: &mut R,
        depth: usize,
    ) -> Result<(), Error> {
        if depth > crate::MAX_NBT_DEPTH {
            return Err(Error::MaxDepthExceeded);
        }

        loop {
            let tag_id = match reader.get_u8() {
                Ok(id) => id,
                Err(Error::Incomplete(e)) if e.kind() == ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e),
            };

            if tag_id == END_ID {
                break;
            }

            reader.skip_string()?;

            // Skip Value
            NbtTag::skip_data_depth(reader, tag_id, depth + 1)?;
        }

        Ok(())
    }

    /// Deserializes a compound payload, starting after the compound's name.
    pub fn deserialize_content<'a, R: NbtReadHelper<'a>>(reader: &mut R) -> Result<Self, Error> {
        Self::deserialize_content_depth(reader, 0)
    }

    /// Deserializes a compound payload with depth tracking.
    pub fn deserialize_content_depth<'a, R: NbtReadHelper<'a>>(
        reader: &mut R,
        depth: usize,
    ) -> Result<Self, Error> {
        if depth > crate::MAX_NBT_DEPTH {
            return Err(Error::MaxDepthExceeded);
        }

        let mut compound = Self::new();

        loop {
            let tag_id = match reader.get_u8() {
                Ok(id) => id,
                Err(Error::Incomplete(e)) if e.kind() == ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e),
            };

            if tag_id == END_ID {
                break;
            }

            let name = reader.get_string()?;
            let tag = NbtTag::deserialize_data_depth(reader, tag_id, depth + 1)?;

            compound.child_tags.insert(name.into(), tag);
        }

        Ok(compound)
    }

    /// Serializes the compound's entries followed by an end tag.
    pub fn serialize_content<W: NbtWriteHelper>(self, w: &mut W) -> Result<(), Error> {
        for (name, tag) in self.child_tags {
            w.write_u8(tag.get_type_id())?;
            w.write_string(&name)?;
            tag.serialize_data(w)?;
        }
        w.write_u8(END_ID)?;
        Ok(())
    }

    /// Returns `true` when the compound contains no child tags.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.child_tags.is_empty()
    }

    /// Inserts or replaces a tag for `name`.
    pub fn put(&mut self, name: &str, value: impl Into<NbtTag>) {
        self.child_tags.insert(name.into(), value.into());
    }

    /// Inserts a string tag when `name` is not already present.
    pub fn put_string(&mut self, name: &str, value: String) {
        self.put(name, NbtTag::String(value.into()));
    }

    /// Inserts a list tag when `name` is not already present.
    pub fn put_list(&mut self, name: &str, value: Vec<NbtTag>) {
        self.put(name, NbtTag::List(value));
    }

    /// Inserts a byte tag when `name` is not already present.
    pub fn put_byte(&mut self, name: &str, value: i8) {
        self.put(name, NbtTag::Byte(value));
    }

    /// Inserts a boolean encoded as a byte tag when `name` is not already present.
    pub fn put_bool(&mut self, name: &str, value: bool) {
        self.put(name, NbtTag::Byte(i8::from(value)));
    }

    /// Inserts a short tag when `name` is not already present.
    pub fn put_short(&mut self, name: &str, value: i16) {
        self.put(name, NbtTag::Short(value));
    }

    /// Inserts an integer tag when `name` is not already present.
    pub fn put_int(&mut self, name: &str, value: i32) {
        self.put(name, NbtTag::Int(value));
    }
    /// Inserts a long tag when `name` is not already present.
    pub fn put_long(&mut self, name: &str, value: i64) {
        self.put(name, NbtTag::Long(value));
    }

    /// Inserts a float tag when `name` is not already present.
    pub fn put_float(&mut self, name: &str, value: f32) {
        self.put(name, NbtTag::Float(value));
    }

    /// Inserts a double tag when `name` is not already present.
    pub fn put_double(&mut self, name: &str, value: f64) {
        self.put(name, NbtTag::Double(value));
    }

    /// Inserts a compound tag when `name` is not already present.
    pub fn put_compound(&mut self, name: &str, value: Self) {
        self.put(name, NbtTag::Compound(value));
    }

    /// Stores a UUID as a 4-element int array, most significant bits first
    /// (the vanilla `UUID` layout).
    pub fn put_uuid(&mut self, name: &str, value: Uuid) {
        let value = value.as_u128();
        self.put(
            name,
            NbtTag::IntArray(vec![
                (value >> 96) as i32,
                ((value >> 64) & 0xFFFF_FFFF) as i32,
                ((value >> 32) & 0xFFFF_FFFF) as i32,
                (value & 0xFFFF_FFFF) as i32,
            ]),
        );
    }

    /// Copies `other`'s tags into this compound, merging nested compounds (vanilla `CompoundTag.merge`).
    pub fn merge(&mut self, other: &Self) {
        for (name, tag) in &other.child_tags {
            match (tag, self.child_tags.get_mut(name)) {
                (NbtTag::Compound(from), Some(NbtTag::Compound(into))) => into.merge(from),
                _ => {
                    self.child_tags.insert(name.clone(), tag.clone());
                }
            }
        }
    }

    /// Returns the named byte value, converting any numeric tag like vanilla
    /// `CompoundTag`, or `None` if the tag is absent or not a number.
    #[must_use]
    pub fn get_byte(&self, name: &str) -> Option<i8> {
        self.get(name).and_then(super::tag::NbtTag::as_numeric_byte)
    }

    /// Returns the named tag.
    #[inline]
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&NbtTag> {
        self.child_tags.get(name)
    }

    /// Returns whether the compound contains `name`.
    #[inline]
    #[must_use]
    pub fn has(&self, name: &str) -> bool {
        self.child_tags.contains_key(name)
    }

    /// Returns the named short value, converting any numeric tag like vanilla
    /// `CompoundTag`, or `None` if the tag is absent or not a number.
    #[must_use]
    pub fn get_short(&self, name: &str) -> Option<i16> {
        self.get(name)
            .and_then(super::tag::NbtTag::as_numeric_short)
    }

    /// Returns any named numeric value as a short
    #[must_use]
    pub fn get_numeric_short(&self, name: &str) -> Option<i16> {
        self.get(name)
            .and_then(super::tag::NbtTag::as_numeric_short)
    }

    /// Returns the named integer value, converting any numeric tag like vanilla
    /// `CompoundTag`, or `None` if the tag is absent or not a number.
    #[must_use]
    pub fn get_int(&self, name: &str) -> Option<i32> {
        self.get(name).and_then(super::tag::NbtTag::as_numeric_int)
    }

    /// Returns the named long value, converting any numeric tag like vanilla
    /// `CompoundTag`, or `None` if the tag is absent or not a number.
    #[must_use]
    pub fn get_long(&self, name: &str) -> Option<i64> {
        self.get(name).and_then(super::tag::NbtTag::as_numeric_long)
    }

    /// Returns the named float value, converting any numeric tag like vanilla
    /// `CompoundTag`, or `None` if the tag is absent or not a number.
    #[must_use]
    pub fn get_float(&self, name: &str) -> Option<f32> {
        self.get(name)
            .and_then(super::tag::NbtTag::as_numeric_float)
    }

    /// Returns any named numeric value as a float
    #[must_use]
    pub fn get_numeric_float(&self, name: &str) -> Option<f32> {
        self.get(name)
            .and_then(super::tag::NbtTag::as_numeric_float)
    }

    /// Returns the named double value, converting any numeric tag like vanilla
    /// `CompoundTag`, or `None` if the tag is absent or not a number.
    #[must_use]
    pub fn get_double(&self, name: &str) -> Option<f64> {
        self.get(name)
            .and_then(super::tag::NbtTag::as_numeric_double)
    }

    /// Returns the named number as a boolean (its byte value is nonzero), like vanilla
    /// `CompoundTag.getBoolean`.
    #[must_use]
    pub fn get_bool(&self, name: &str) -> Option<bool> {
        self.get(name)
            .and_then(super::tag::NbtTag::as_numeric_byte)
            .map(|byte| byte != 0)
    }

    /// Returns the named string, or `None` if the tag is absent or has another type.
    #[must_use]
    pub fn get_string(&self, name: &str) -> Option<&str> {
        self.get(name).and_then(|tag| tag.extract_string())
    }

    /// Returns the named list, or `None` if the tag is absent or has another type.
    #[must_use]
    pub fn get_list(&self, name: &str) -> Option<&[NbtTag]> {
        self.get(name).and_then(|tag| tag.extract_list())
    }

    /// Returns the named compound, or `None` if the tag is absent or has another type.
    #[must_use]
    pub fn get_compound(&self, name: &str) -> Option<&Self> {
        self.get(name).and_then(|tag| tag.extract_compound())
    }

    /// Returns the named byte array, or `None` if the tag is absent or has another type.
    #[must_use]
    pub fn get_byte_array(&self, name: &str) -> Option<&[i8]> {
        self.get(name).and_then(|tag| tag.extract_byte_array())
    }

    /// Returns the named integer array, or `None` if the tag is absent or has another type.
    #[must_use]
    pub fn get_int_array(&self, name: &str) -> Option<&[i32]> {
        self.get(name).and_then(|tag| tag.extract_int_array())
    }

    /// Returns the named long array, or `None` if the tag is absent or has another type.
    #[must_use]
    pub fn get_long_array(&self, name: &str) -> Option<&[i64]> {
        self.get(name).and_then(|tag| tag.extract_long_array())
    }

    /// Reads a UUID stored as a 4-element int array, most significant bits
    /// first (the vanilla `UUID` layout written by [`Self::put_uuid`]).
    #[must_use]
    pub fn get_uuid(&self, name: &str) -> Option<Uuid> {
        let &[a, b, c, d] = self.get_int_array(name)? else {
            return None;
        };
        Some(Uuid::from_u128(
            ((a as u32 as u128) << 96)
                | ((b as u32 as u128) << 64)
                | ((c as u32 as u128) << 32)
                | (d as u32 as u128),
        ))
    }
}

impl From<Nbt> for NbtCompound {
    fn from(value: Nbt) -> Self {
        value.root_tag
    }
}

impl FromIterator<(String, NbtTag)> for NbtCompound {
    fn from_iter<T: IntoIterator<Item = (String, NbtTag)>>(iter: T) -> Self {
        let mut compound = Self::new();
        for (key, value) in iter {
            compound.put(&key, value);
        }
        compound
    }
}

impl FromIterator<(Box<str>, NbtTag)> for NbtCompound {
    fn from_iter<T: IntoIterator<Item = (Box<str>, NbtTag)>>(iter: T) -> Self {
        let mut compound = Self::new();
        for (key, value) in iter {
            compound.child_tags.insert(key, value);
        }
        compound
    }
}

impl IntoIterator for NbtCompound {
    type Item = (Box<str>, NbtTag);
    type IntoIter = <JavaHashMap<NbtTag> as IntoIterator>::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        self.child_tags.into_iter()
    }
}

impl Extend<(String, NbtTag)> for NbtCompound {
    fn extend<T: IntoIterator<Item = (String, NbtTag)>>(&mut self, iter: T) {
        for (key, value) in iter {
            self.put(&key, value);
        }
    }
}

impl Extend<(Box<str>, NbtTag)> for NbtCompound {
    fn extend<T: IntoIterator<Item = (Box<str>, NbtTag)>>(&mut self, iter: T) {
        self.child_tags.extend(iter);
    }
}

// Rust's AsRef is currently not reflexive so we need to implement it manually
impl AsRef<Self> for NbtCompound {
    fn as_ref(&self) -> &Self {
        self
    }
}

impl From<NbtCompound> for NbtTag {
    fn from(value: NbtCompound) -> Self {
        Self::Compound(value)
    }
}

/// Vanilla `CompoundTag.toString`, which prints through `StringTagVisitor`.
impl Display for NbtCompound {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("{")?;
        let mut entries: Vec<_> = self.child_tags.iter().collect();
        entries.sort_by(|(a, _), (b, _)| a.encode_utf16().cmp(b.encode_utf16()));
        for (i, (key, value)) in entries.into_iter().enumerate() {
            if i != 0 {
                f.write_str(",")?;
            }
            write_key(f, key)?;
            write!(f, ":{value}")?;
        }
        f.write_str("}")
    }
}

/// `StringTagVisitor.handleKeyEscape`.
fn write_key(f: &mut Formatter<'_>, key: &str) -> std::fmt::Result {
    let mut chars = key.chars();
    let unquoted = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '.' || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'))
        && !key.eq_ignore_ascii_case("true")
        && !key.eq_ignore_ascii_case("false");
    if unquoted {
        f.write_str(key)
    } else {
        f.write_str(&quote_and_escape(key))
    }
}

/// Vanilla `Tag.toString`, which prints through `StringTagVisitor`.
impl Display for NbtTag {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::End => f.write_str("END"),
            Self::Byte(v) => write!(f, "{v}b"),
            Self::Short(v) => write!(f, "{v}s"),
            Self::Int(v) => write!(f, "{v}"),
            Self::Long(v) => write!(f, "{v}L"),
            Self::Float(v) => write!(f, "{}f", float_to_string(float_value_of(*v))),
            Self::Double(v) => write!(f, "{}d", double_to_string(double_value_of(*v))),
            Self::String(v) => f.write_str(&quote_and_escape(v)),
            Self::Compound(v) => write!(f, "{v}"),
            Self::ByteArray(v) => {
                f.write_str("[B;")?;
                for (i, byte) in v.iter().enumerate() {
                    if i != 0 {
                        f.write_str(",")?;
                    }
                    write!(f, "{byte}B")?;
                }
                f.write_str("]")
            }
            Self::List(v) => {
                f.write_str("[")?;
                for (i, tag) in v.iter().enumerate() {
                    if i != 0 {
                        f.write_str(",")?;
                    }
                    write!(f, "{tag}")?;
                }
                f.write_str("]")
            }
            Self::IntArray(v) => {
                f.write_str("[I;")?;
                for (i, int) in v.iter().enumerate() {
                    if i != 0 {
                        f.write_str(",")?;
                    }
                    write!(f, "{int}")?;
                }
                f.write_str("]")
            }
            Self::LongArray(v) => {
                f.write_str("[L;")?;
                for (i, long) in v.iter().enumerate() {
                    if i != 0 {
                        f.write_str(",")?;
                    }
                    write!(f, "{long}L")?;
                }
                f.write_str("]")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::NbtCompound;
    use uuid::Uuid;

    #[test]
    fn uuid_int_array_round_trip() {
        let original = Uuid::from_u128(0x0123_4567_89ab_cdef_fedc_ba98_7654_3210);
        let mut nbt = NbtCompound::new();
        nbt.put_uuid("UUID", original);
        assert_eq!(nbt.get_uuid("UUID"), Some(original));

        // Missing or malformed entries fall back to None.
        assert_eq!(nbt.get_uuid("missing"), None);
        let mut short = NbtCompound::new();
        short.put("UUID", crate::tag::NbtTag::IntArray(vec![1, 2, 3]));
        assert_eq!(short.get_uuid("UUID"), None);
    }
}
