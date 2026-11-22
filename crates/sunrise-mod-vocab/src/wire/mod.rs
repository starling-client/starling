//! Canonical binary encoding for owned API values, including typed payload tokens and bodies.
//! A typed body is one bounded Wire value with no envelope; raw channels use caller-supplied bytes.
//!
//! Everything is little-endian and self-delimiting: a length is a `u32` before its items, an
//! `Option` is a tag byte, an enum is a `u32` case then its fields, a record is its fields in
//! order. A reader trusts nothing: it checks every length against the bytes left and against the
//! smallest encoding of the items it promises, every tag against the cases the type has, strings
//! as UTF-8, and nesting against [`MAX_DEPTH`]. A bad input is a [`WireError`], never a panic and
//! never an allocation larger than the input can back.
//!
//! Derive [`Wire`](crate::Wire) on types that cross a target boundary. The derive encodes
//! fields in declaration order; ordinary Wire can use checked constructors via `#[wire(via = T)]`.
//! Hand-written codecs implement [`Wire::describe`] with [`WireSchema`].
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::hash::Hash;
use std::sync::Arc;

pub(crate) mod limits;
mod schema;

use limits::{Budget, PAYLOAD_MAX_DEPTH, PAYLOAD_MAX_LIST_ENTRIES, PAYLOAD_MAX_STRING_BYTES};

pub use schema::WireSchema;

/// The deepest nesting of one value (a component of a component of…) a reader accepts.
///
/// Structural codecs refuse before entering level 33. `Tag` and `Text` use
/// stack-independent tree codecs to preserve their vanilla depth of 512.
/// Container iteration does not itself consume reader depth.
pub const MAX_DEPTH: u16 = 32;

const UNBOUNDED_PAYLOAD_FORM: WireError =
    WireError::Invalid("this Wire form is not available in typed payloads");

/// Why a buffer is not a value of the type it was read as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: decoder and encoder diagnostics are returned locally, not encoded as values.
pub enum WireError {
    /// The buffer ends inside a value.
    Eof,
    /// A string is not UTF-8.
    Utf8,
    /// An enum case or option tag the type does not have.
    Tag(u32),
    /// A length longer than the rest of the buffer could hold.
    Length,
    /// Values nested deeper than the reader or writer allows (normally [`MAX_DEPTH`]).
    Depth,
    /// The typed value would exceed its allocation budget.
    Budget,
    /// A value its type refuses (an unknown key, a count out of range), and why.
    Invalid(&'static str),
    /// Bytes left over after the value.
    Trailing,
}

impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Eof => f.write_str("the data ends inside a value"),
            Self::Utf8 => f.write_str("a string is not UTF-8"),
            Self::Tag(tag) => write!(f, "case {tag} does not exist"),
            Self::Length => f.write_str("a length is longer than the data"),
            Self::Depth => f.write_str("values are nested too deeply"),
            Self::Budget => f.write_str("the value exceeds its allocation budget"),
            Self::Invalid(why) => f.write_str(why),
            Self::Trailing => f.write_str("data left over after the value"),
        }
    }
}

impl std::error::Error for WireError {}

// exhaustive: fields are private; use the codec methods.
/// A buffer values are written to. Reuse one to keep the allocation.
#[derive(Default)]
pub struct Writer {
    bytes: Vec<u8>,
    error: Option<WireError>,
    limit: Option<usize>,
    depth: u16,
    payload: Option<PayloadLimits>,
}

struct PayloadLimits {
    ceiling: usize,
    budget: Budget,
}

impl Writer {
    /// Creates an empty writer with no byte limit.
    pub fn new() -> Self {
        Self::default()
    }
    pub(crate) fn for_payload(limit: usize) -> Self {
        Self {
            limit: Some(limit),
            payload: Some(PayloadLimits {
                ceiling: limit,
                budget: Budget::payload(),
            }),
            ..Self::default()
        }
    }
    /// Empties it, keeping its allocation and resetting the local byte limit. In a typed
    /// payload the enclosing limits, budget already spent and first failure stay in force.
    pub fn clear(&mut self) {
        self.bytes.clear();
        self.limit = self.payload.as_ref().map(|limits| limits.ceiling);
        if self.payload.is_none() {
            self.error = None;
        }
    }
    /// Takes `buffer` as its storage, then resets as in [`Writer::clear`].
    pub fn replace(&mut self, buffer: Vec<u8>) {
        self.bytes = buffer;
        self.clear();
    }
    pub(crate) fn finish_payload(self) -> Result<Vec<u8>, WireError> {
        if self.payload.is_none() {
            return Err(WireError::Invalid("payload writer was replaced"));
        }
        match self.error {
            Some(error) => Err(error),
            None => Ok(self.bytes),
        }
    }
    /// The bytes written, leaving the writer's storage to the caller.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
    /// Borrows the bytes written so far; check `error` before sending them.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    /// Bound this call's encoded bytes without raising an enclosing typed payload limit.
    /// A refusal is recorded before any growth or copy. `clear` and `replace` remove only
    /// this local limit.
    pub fn limit(&mut self, max_bytes: usize) {
        self.limit = Some(
            self.payload
                .as_ref()
                .map_or(max_bytes, |limits| max_bytes.min(limits.ceiling)),
        );
        self.can_fit(0);
    }
    fn can_fit(&mut self, bytes: usize) -> bool {
        if self.error.is_some() {
            return false;
        }
        let fits = self.bytes.len().checked_add(bytes).is_some_and(|total| {
            total <= self.limit.unwrap_or(isize::MAX as usize) && total <= isize::MAX as usize
        });
        if !fits {
            self.fail(WireError::Length);
        }
        fits
    }
    /// Appends raw bytes without a length prefix.
    pub fn put_bytes(&mut self, bytes: &[u8]) {
        if self.can_fit(bytes.len()) {
            self.bytes.extend_from_slice(bytes);
        }
    }
    /// A string: its length, then its bytes.
    pub fn put_str(&mut self, text: &str) {
        if let Some(limits) = &mut self.payload {
            if text.len() > PAYLOAD_MAX_STRING_BYTES {
                self.fail(WireError::Length);
                return;
            }
            if let Err(error) = limits.budget.bytes(text.len() as u64) {
                self.fail(error);
                return;
            }
        }
        let (Ok(len), Some(size)) = (u32::try_from(text.len()), text.len().checked_add(4)) else {
            self.fail(WireError::Length);
            return;
        };
        if self.can_fit(size) {
            self.reserve_and_prefix(len, size);
            self.bytes.extend_from_slice(text.as_bytes());
        }
    }
    fn put_slice<T: Put>(&mut self, items: &[T], min: usize) {
        self.payload_nested(|out| {
            if out.payload.is_some() {
                let Some(len) = out.list_len(items.len()) else {
                    return;
                };
                let Some(size) = min
                    .checked_mul(items.len())
                    .and_then(|size| size.checked_add(4))
                else {
                    out.fail(WireError::Length);
                    return;
                };
                if !out.can_fit(size) || !out.charge::<T>(items.len()) {
                    return;
                }
                // When the count itself needs growth, reserve for the known minimum too.
                // Otherwise let the next value reserve its actual size (e.g. a long string).
                if out.bytes.capacity() - out.bytes.len() < 4 {
                    out.bytes.reserve(size);
                }
                out.bytes.extend_from_slice(&len.to_le_bytes());
            } else {
                out.put_len(items.len());
            }
            for item in items {
                out.put(item);
            }
        });
    }
    /// A length or count.
    pub fn put_len(&mut self, len: usize) {
        if let Some(len) = self.list_len(len) {
            self.put(&len);
        }
    }
    fn list_len(&mut self, len: usize) -> Option<u32> {
        if self.payload.is_some() && len > PAYLOAD_MAX_LIST_ENTRIES {
            self.fail(WireError::Length);
            return None;
        }
        match u32::try_from(len) {
            Ok(len) => Some(len),
            Err(_) => {
                self.fail(WireError::Length);
                None
            }
        }
    }
    // The caller checked that `size` (prefix plus body) fits. Grow for both in one step.
    fn reserve_and_prefix(&mut self, len: u32, size: usize) {
        self.bytes.reserve(size);
        self.bytes.extend_from_slice(&len.to_le_bytes());
    }
    /// An optional borrowed value, written as the `Option` it stands for.
    pub fn put_opt<T: Put + ?Sized>(&mut self, value: Option<&T>) {
        match value {
            Some(value) => {
                self.put(&1u8);
                self.payload_nested(|out| out.put(value));
            }
            None => self.put(&0u8),
        }
    }
    /// Encodes a borrowed value unless an earlier write failed.
    /// The caller keeps ownership and is responsible for disposing of the value.
    pub fn put<T: Put + ?Sized>(&mut self, value: &T) {
        if self.error.is_none() {
            value.put_to(self);
        }
    }
    /// Writes one explicit nesting level. A recursive handwritten codec must use this
    /// before descending on both the ordinary and the typed payload paths.
    pub fn nested(&mut self, write: impl FnOnce(&mut Self)) {
        let limit = if self.payload.is_some() {
            PAYLOAD_MAX_DEPTH
        } else {
            MAX_DEPTH
        };
        self.nest_at(limit, write);
    }
    // hidden: transport plumbing used by generated boundary code.
    #[doc(hidden)]
    pub fn record(&mut self, write: impl FnOnce(&mut Self)) {
        if self.payload.is_some() {
            write(self);
        } else {
            self.nest_at(MAX_DEPTH, write);
        }
    }
    fn nest_at(&mut self, limit: u16, write: impl FnOnce(&mut Self)) {
        if self.error.is_some() {
            return;
        }
        if self.depth >= limit {
            self.fail(WireError::Depth);
            return;
        }
        self.depth += 1;
        write(self);
        self.depth -= 1;
    }
    fn payload_nested(&mut self, write: impl FnOnce(&mut Self)) {
        if self.payload.is_some() {
            self.nest_at(PAYLOAD_MAX_DEPTH, write);
        } else {
            write(self);
        }
    }
    /// Charge storage for decoded `T`s before writing them in the bounded payload path.
    pub(crate) fn charge<T>(&mut self, count: usize) -> bool {
        self.charge_with(|budget| budget.charge::<T>(count))
    }
    fn charge_with(&mut self, charge: impl FnOnce(&mut Budget) -> Result<(), WireError>) -> bool {
        if let Some(limits) = &mut self.payload
            && let Err(error) = charge(&mut limits.budget)
        {
            self.fail(error);
            return false;
        }
        true
    }
    // hidden: generated Wire codecs check this before unmetered conversion.
    #[doc(hidden)]
    pub fn ordinary_only(&mut self) -> bool {
        if self.payload.is_some() {
            self.fail(UNBOUNDED_PAYLOAD_FORM);
            return false;
        }
        true
    }
    /// The first encoding failure; callers refuse the call rather than sending partial bytes.
    pub fn error(&self) -> Option<WireError> {
        self.error
    }
    /// Records the first failure, preventing subsequent writes.
    pub fn fail(&mut self, error: WireError) {
        self.error.get_or_insert(error);
    }
}

/// What can be written: every [`Wire`] value, and the borrowed forms a call passes
/// (`&str`, `&[T]`, `Option<&T>`), which are written as the owned value they stand for.
pub trait Put {
    /// Writes it.
    fn put_to(&self, out: &mut Writer);
}

impl<T: Wire> Put for T {
    fn put_to(&self, out: &mut Writer) {
        self.put(out);
    }
}

impl Put for str {
    fn put_to(&self, out: &mut Writer) {
        out.put_str(self);
    }
}

impl<T: Put> Put for [T] {
    fn put_to(&self, out: &mut Writer) {
        out.put_slice(self, 0);
    }
}

// exhaustive: fields are private; use the codec methods.
/// A buffer values are read from.
pub struct Reader<'a> {
    data: &'a [u8],
    depth: u16,
    budget: Option<Budget>,
}

impl<'a> Reader<'a> {
    /// Reads values from a borrowed buffer without copying it.
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            depth: 0,
            budget: None,
        }
    }
    pub(crate) fn for_payload(data: &'a [u8]) -> Self {
        Self {
            budget: Some(Budget::payload()),
            ..Self::new(data)
        }
    }
    /// The bytes not yet read.
    pub fn rest(&self) -> &'a [u8] {
        self.data
    }
    /// Takes exactly `len` bytes, refusing a length beyond the remaining buffer.
    pub fn take_bytes(&mut self, len: usize) -> Result<&'a [u8], WireError> {
        let (head, tail) = self.data.split_at_checked(len).ok_or(WireError::Eof)?;
        self.data = tail;
        Ok(head)
    }
    /// A length read before `items` values of at least `min` bytes each: refused when the rest
    /// of the buffer could not hold them, so no allocation outruns its input.
    pub fn take_len(&mut self, min: usize) -> Result<usize, WireError> {
        let len = u32::take(self)? as usize;
        let min = if self.budget.is_some() {
            if len > PAYLOAD_MAX_LIST_ENTRIES {
                return Err(WireError::Length);
            }
            min // Zero-width entries are safe when their count is capped independently.
        } else {
            min.max(1)
        };
        match len.checked_mul(min) {
            Some(needed) if needed <= self.data.len() => Ok(len),
            _ => Err(WireError::Length),
        }
    }
    fn take_str(&mut self) -> Result<&'a str, WireError> {
        let len = self.take_len(1)?;
        if self.budget.is_some() && len > PAYLOAD_MAX_STRING_BYTES {
            return Err(WireError::Length);
        }
        std::str::from_utf8(self.take_bytes(len)?).map_err(|_| WireError::Utf8)
    }
    /// Decodes one value from the current position.
    pub fn take<T: Wire>(&mut self) -> Result<T, WireError> {
        T::take(self)
    }
    /// Read one value from at most `max_bytes`, preserving the enclosing reader's depth and
    /// leaving the following value untouched. No nested allocation can outrun this byte window.
    pub fn bounded<T>(
        &mut self,
        max_bytes: usize,
        read: impl FnOnce(&mut Self) -> Result<T, WireError>,
    ) -> Result<T, WireError> {
        let data = self.data;
        let len = data.len().min(max_bytes);
        let mut inner = Self {
            data: &data[..len],
            depth: self.depth,
            budget: self.budget.take(),
        };
        let value = read(&mut inner);
        self.data = &data[len - inner.data.len()..];
        self.budget = inner.budget;
        value
    }
    /// A normal Wire list, with its element cap checked before allocating storage. `read`
    /// may impose a shared payload budget across its elements without a second list codec.
    pub fn list_with<T: Wire>(
        &mut self,
        max_items: usize,
        mut read: impl FnMut(&mut Self) -> Result<T, WireError>,
    ) -> Result<Vec<T>, WireError> {
        self.payload_nested(|input| {
            let len = input.take_len(T::MIN)?;
            if len > max_items {
                return Err(WireError::Length);
            }
            input.charge::<T>(len)?;
            let mut items = Vec::with_capacity(len);
            for _ in 0..len {
                items.push(read(input)?);
            }
            Ok(items)
        })
    }
    /// Reads one explicit nesting level. A recursive handwritten codec must use this
    /// before descending on both the ordinary and the typed payload paths.
    pub fn nested<T>(
        &mut self,
        read: impl FnOnce(&mut Self) -> Result<T, WireError>,
    ) -> Result<T, WireError> {
        let limit = if self.budget.is_some() {
            PAYLOAD_MAX_DEPTH
        } else {
            MAX_DEPTH
        };
        self.nest_at(limit, read)
    }
    // hidden: transport plumbing used by generated boundary code.
    #[doc(hidden)]
    pub fn record<T>(
        &mut self,
        read: impl FnOnce(&mut Self) -> Result<T, WireError>,
    ) -> Result<T, WireError> {
        if self.budget.is_some() {
            read(self)
        } else {
            self.nest_at(MAX_DEPTH, read)
        }
    }
    fn nest_at<T>(
        &mut self,
        limit: u16,
        read: impl FnOnce(&mut Self) -> Result<T, WireError>,
    ) -> Result<T, WireError> {
        if self.depth >= limit {
            return Err(WireError::Depth);
        }
        self.depth += 1;
        let value = read(self);
        self.depth -= 1;
        value
    }
    fn payload_nested<T>(
        &mut self,
        read: impl FnOnce(&mut Self) -> Result<T, WireError>,
    ) -> Result<T, WireError> {
        if self.budget.is_some() {
            self.nest_at(PAYLOAD_MAX_DEPTH, read)
        } else {
            read(self)
        }
    }
    /// Charge bounded payload allocations before reserving or copying their storage.
    pub(crate) fn charge<T>(&mut self, count: usize) -> Result<(), WireError> {
        self.charge_with(|budget| budget.charge::<T>(count))
    }
    fn charge_with(
        &mut self,
        charge: impl FnOnce(&mut Budget) -> Result<(), WireError>,
    ) -> Result<(), WireError> {
        match &mut self.budget {
            Some(budget) => charge(budget),
            None => Ok(()),
        }
    }
    // hidden: generated Wire codecs check this before unmetered conversion.
    #[doc(hidden)]
    pub fn ordinary_only(&self) -> Result<(), WireError> {
        if self.budget.is_some() {
            return Err(UNBOUNDED_PAYLOAD_FORM);
        }
        Ok(())
    }
    /// The whole of `data` as one `T`.
    pub fn decode<T: Wire>(data: &'a [u8]) -> Result<T, WireError> {
        let mut reader = Self::new(data);
        let value = T::take(&mut reader)?;
        match reader.data.is_empty() {
            true => Ok(value),
            false => Err(WireError::Trailing),
        }
    }
}

/// A type with a wire form.
#[diagnostic::on_unimplemented(
    message = "`{Self}` has no Wire form",
    label = "add #[derive(mod_api::Wire)] to it"
)]
pub trait Wire: Sized {
    /// Minimum encoded size, used to reject impossible collection lengths.
    const MIN: usize;
    /// Describes this codec's structural layout, independently of any particular value.
    /// Derives describe their fields in encoding order. A manual codec must distinguish
    /// changes to both the byte format and validation, so payload peers can reject a mismatch.
    fn describe(schema: &mut WireSchema) -> u64
    where
        Self: 'static;
    /// Encodes this value, recording failures on the writer.
    fn put(&self, out: &mut Writer);
    /// Decodes one value, leaving any following bytes for the caller.
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError>;
    // hidden: internal Vec encoding specialization, not a mod-facing API
    /// Internal list codec; primitive numbers override this with a contiguous codec.
    #[doc(hidden)]
    fn put_list(items: &[Self], out: &mut Writer) {
        out.put_slice(items, Self::MIN);
    }
    // hidden: internal Vec decoding specialization for contiguous numeric payloads
    #[doc(hidden)]
    fn take_list(input: &mut Reader<'_>) -> Result<Vec<Self>, WireError> {
        input.list_with(usize::MAX, Self::take)
    }
}

macro_rules! numbers {
    ($($ty:ty => $format:literal),*) => {$(
        impl Wire for $ty {
            const MIN: usize = std::mem::size_of::<$ty>();
            fn describe(_: &mut WireSchema) -> u64 { WireSchema::leaf($format) }
            fn put(&self, out: &mut Writer) {
                out.put_bytes(&self.to_le_bytes());
            }
            fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
                let bytes = input.take_bytes(std::mem::size_of::<$ty>())?;
                Ok(<$ty>::from_le_bytes(bytes.try_into().expect("a fixed-size slice")))
            }
            fn put_list(items: &[Self], out: &mut Writer) {
                out.payload_nested(|out| {
                    let Some(size) = items.len().checked_mul(<Self as Wire>::MIN).and_then(|n| n.checked_add(4)) else {
                        out.fail(WireError::Length);
                        return;
                    };
                    if !out.can_fit(size) || !out.charge::<Self>(items.len()) { return; }
                    let Some(len) = out.list_len(items.len()) else { return; };
                    out.reserve_and_prefix(len, size);
                    #[cfg(target_endian = "little")]
                    out.bytes.extend_from_slice(bytemuck::cast_slice(items));
                    #[cfg(target_endian = "big")]
                    for item in items { out.bytes.extend_from_slice(&item.to_le_bytes()); }
                });
            }
            fn take_list(input: &mut Reader<'_>) -> Result<Vec<Self>, WireError> {
                input.payload_nested(|input| {
                    let len = input.take_len(<Self as Wire>::MIN)?;
                    input.charge::<Self>(len)?;
                    let bytes = input.take_bytes(len * <Self as Wire>::MIN)?;
                    Ok(bytes.chunks_exact(<Self as Wire>::MIN).map(|chunk| {
                        Self::from_le_bytes(chunk.try_into().expect("a fixed-size chunk"))
                    }).collect())
                })
            }
        }
    )*};
}
numbers!(
    u8 => "u8", i8 => "i8", u16 => "u16-le", i16 => "i16-le",
    u32 => "u32-le", i32 => "i32-le", u64 => "u64-le", i64 => "i64-le",
    u128 => "u128-le", i128 => "i128-le", f32 => "f32-le", f64 => "f64-le"
);

/// A `usize` crosses as 64 bits: a guest's is 32.
impl Wire for usize {
    const MIN: usize = 8;
    fn describe(_: &mut WireSchema) -> u64 {
        WireSchema::leaf("usize:u64-le")
    }
    fn put(&self, out: &mut Writer) {
        out.put(&(*self as u64));
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        usize::try_from(u64::take(input)?).map_err(|_| WireError::Invalid("a size too large"))
    }
}

impl Wire for bool {
    const MIN: usize = 1;
    fn describe(_: &mut WireSchema) -> u64 {
        WireSchema::leaf("bool")
    }
    fn put(&self, out: &mut Writer) {
        out.put(&u8::from(*self));
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        match u8::take(input)? {
            0 => Ok(false),
            1 => Ok(true),
            tag => Err(WireError::Tag(tag.into())),
        }
    }
}

impl Wire for char {
    const MIN: usize = 4;
    fn describe(_: &mut WireSchema) -> u64 {
        WireSchema::leaf("char:u32-le")
    }
    fn put(&self, out: &mut Writer) {
        out.put(&u32::from(*self));
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        char::from_u32(u32::take(input)?).ok_or(WireError::Invalid("not a character"))
    }
}

impl Wire for () {
    const MIN: usize = 0;
    fn describe(_: &mut WireSchema) -> u64 {
        WireSchema::leaf("unit")
    }
    fn put(&self, _: &mut Writer) {}
    fn take(_: &mut Reader<'_>) -> Result<Self, WireError> {
        Ok(())
    }
}

impl Wire for String {
    const MIN: usize = 4;
    fn describe(_: &mut WireSchema) -> u64 {
        WireSchema::leaf("string:u32-le-length")
    }
    fn put(&self, out: &mut Writer) {
        out.put_str(self);
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        let text = input.take_str()?;
        input.charge_with(|budget| budget.bytes(text.len() as u64))?;
        Ok(text.to_owned())
    }
}

/// A mod's own text, owned once read: a `Cow<'static, str>` field is how the API takes either.
impl Wire for std::borrow::Cow<'_, str> {
    const MIN: usize = 4;
    fn describe(schema: &mut WireSchema) -> u64 {
        String::describe(schema)
    }
    fn put(&self, out: &mut Writer) {
        out.put_str(self);
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        String::take(input).map(Self::Owned)
    }
}

impl Wire for Box<str> {
    const MIN: usize = 4;
    fn describe(schema: &mut WireSchema) -> u64 {
        String::describe(schema)
    }
    fn put(&self, out: &mut Writer) {
        out.put_str(self);
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        String::take(input).map(String::into_boxed_str)
    }
}

impl Wire for Arc<str> {
    const MIN: usize = 4;
    fn describe(schema: &mut WireSchema) -> u64 {
        WireSchema::mix(WireSchema::leaf("arc-str"), String::describe(schema))
    }
    fn put(&self, out: &mut Writer) {
        // `put_str` charges the text bytes, so charge only the rest of the Arc here.
        if out.charge_with(|budget| budget.arc_str_overhead(self.len())) {
            out.put_str(self);
        }
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        let text = input.take_str()?;
        input.charge_with(|budget| budget.arc::<u8>(text.len()))?;
        Ok(Arc::from(text))
    }
}

impl<T: Wire> Wire for Option<T> {
    const MIN: usize = 1;
    fn describe(schema: &mut WireSchema) -> u64
    where
        Self: 'static,
    {
        WireSchema::mix(WireSchema::leaf("option"), T::describe(schema))
    }
    fn put(&self, out: &mut Writer) {
        out.put_opt(self.as_ref());
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        match u8::take(input)? {
            0 => Ok(None),
            1 => input.payload_nested(T::take).map(Some),
            tag => Err(WireError::Tag(tag.into())),
        }
    }
}

impl<T: Wire, E: Wire> Wire for Result<T, E> {
    const MIN: usize = 1;
    fn describe(schema: &mut WireSchema) -> u64
    where
        Self: 'static,
    {
        let hash = WireSchema::mix(WireSchema::leaf("result:u8-tag"), T::describe(schema));
        WireSchema::mix(hash, E::describe(schema))
    }
    fn put(&self, out: &mut Writer) {
        match self {
            Ok(value) => {
                out.put(&0u8);
                out.payload_nested(|out| out.put(value));
            }
            Err(error) => {
                out.put(&1u8);
                out.payload_nested(|out| out.put(error));
            }
        }
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        match u8::take(input)? {
            0 => input.payload_nested(T::take).map(Ok),
            1 => input.payload_nested(E::take).map(Err),
            tag => Err(WireError::Tag(tag.into())),
        }
    }
}

impl<T: Wire> Wire for Arc<[T]> {
    const MIN: usize = 4;
    fn describe(schema: &mut WireSchema) -> u64
    where
        Self: 'static,
    {
        WireSchema::mix(WireSchema::leaf("arc-slice"), Vec::<T>::describe(schema))
    }
    fn put(&self, out: &mut Writer) {
        if out.charge_with(|budget| budget.arc::<T>(self.len())) {
            T::put_list(self, out);
        }
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        let items = Vec::<T>::take(input)?;
        input.charge_with(|budget| budget.arc::<T>(items.len()))?;
        Ok(items.into())
    }
}

impl<T: Wire> Wire for Box<[T]> {
    const MIN: usize = 4;
    fn describe(schema: &mut WireSchema) -> u64
    where
        Self: 'static,
    {
        Vec::<T>::describe(schema)
    }
    fn put(&self, out: &mut Writer) {
        T::put_list(self, out);
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        Vec::<T>::take(input).map(Into::into)
    }
}

impl<T: Wire> Wire for Box<T> {
    const MIN: usize = T::MIN;
    fn describe(schema: &mut WireSchema) -> u64
    where
        Self: 'static,
    {
        WireSchema::mix(WireSchema::leaf("box"), T::describe(schema))
    }
    fn put(&self, out: &mut Writer) {
        out.nested(|out| {
            if out.charge::<T>(1) {
                (**self).put(out);
            }
        });
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        input.nested(|input| {
            input.charge::<T>(1)?;
            T::take(input).map(Box::new)
        })
    }
}

impl<T: Wire> Wire for Arc<T> {
    const MIN: usize = T::MIN;
    fn describe(schema: &mut WireSchema) -> u64
    where
        Self: 'static,
    {
        WireSchema::mix(WireSchema::leaf("arc"), T::describe(schema))
    }
    fn put(&self, out: &mut Writer) {
        out.nested(|out| {
            if out.charge_with(|budget| budget.arc::<T>(1)) {
                (**self).put(out);
            }
        });
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        input.nested(|input| {
            input.charge_with(|budget| budget.arc::<T>(1))?;
            T::take(input).map(Arc::new)
        })
    }
}

impl<T: Wire> Wire for Vec<T> {
    const MIN: usize = 4;
    fn describe(schema: &mut WireSchema) -> u64
    where
        Self: 'static,
    {
        WireSchema::mix(WireSchema::leaf("list:u32-le-length"), T::describe(schema))
    }
    fn put(&self, out: &mut Writer) {
        T::put_list(self, out);
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        T::take_list(input)
    }
}

impl<T: Wire> Wire for VecDeque<T> {
    const MIN: usize = 4;
    fn describe(schema: &mut WireSchema) -> u64
    where
        Self: 'static,
    {
        Vec::<T>::describe(schema)
    }
    fn put(&self, out: &mut Writer) {
        out.payload_nested(|out| {
            if out.charge::<T>(self.len()) {
                out.put_len(self.len());
                for item in self {
                    out.put(item);
                }
            }
        });
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        Vec::take(input).map(Into::into)
    }
}

impl<T: Wire, const N: usize> Wire for [T; N] {
    const MIN: usize = T::MIN * N;
    fn describe(schema: &mut WireSchema) -> u64
    where
        Self: 'static,
    {
        let hash = WireSchema::mix(WireSchema::leaf("array"), N as u64);
        WireSchema::mix(hash, T::describe(schema))
    }
    fn put(&self, out: &mut Writer) {
        if out.charge::<T>(N) {
            for item in self {
                item.put(out);
            }
        }
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        if input.budget.is_some() && Self::MIN > input.rest().len() {
            return Err(WireError::Eof);
        }
        input.charge::<T>(N)?;
        let mut items = Vec::with_capacity(N);
        for _ in 0..N {
            items.push(T::take(input)?);
        }
        items
            .try_into()
            .map_err(|_| WireError::Invalid("an array of the wrong length"))
    }
}

// Reserve the charged hash table once; insertion cannot temporarily hold two tables.
// Ordinary Wire keeps its existing incremental insertion path.
macro_rules! collection_storage {
    (btree, $input:ident, $len:ident) => {
        Self::new()
    };
    (hash_table, $input:ident, $len:ident) => {
        if $input.budget.is_some() {
            Self::with_capacity($len)
        } else {
            Self::new()
        }
    };
}

/// Sets and maps cross as a list of their entries; a key that repeats keeps the last.
macro_rules! sets {
    ($($ty:ident [$($bound:tt)+] => $budget:ident, $contract:literal;)*) => {$(
        impl<T: Wire + $($bound)+> Wire for $ty<T> {
            const MIN: usize = 4;
            fn describe(schema: &mut WireSchema) -> u64 where Self: 'static {
                WireSchema::mix(WireSchema::leaf($contract), T::describe(schema))
            }
            fn put(&self, out: &mut Writer) {
                out.payload_nested(|out| {
                    if !out.charge_with(|budget| budget.$budget::<T>(self.len())) { return; }
                    out.put_len(self.len());
                    for item in self {
                        item.put(out);
                    }
                });
            }
            fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
                input.payload_nested(|input| {
                    let len = input.take_len(T::MIN)?;
                    input.charge_with(|budget| budget.$budget::<T>(len))?;
                    let mut set = collection_storage!($budget, input, len);
                    for _ in 0..len {
                        set.insert(T::take(input)?);
                    }
                    Ok(set)
                })
            }
        }
    )*};
}
sets!(
    BTreeSet[Ord] => btree, "btree-set:u32-le-length:v1";
    HashSet[Eq + Hash] => hash_table, "hash-set:u32-le-length:v1";
);

macro_rules! maps {
    ($($ty:ident [$($bound:tt)+] => $budget:ident, $contract:literal;)*) => {$(
        impl<K: Wire + $($bound)+, V: Wire> Wire for $ty<K, V> {
            const MIN: usize = 4;
            fn describe(schema: &mut WireSchema) -> u64 where Self: 'static {
                let hash = WireSchema::mix(WireSchema::leaf($contract), K::describe(schema));
                WireSchema::mix(hash, V::describe(schema))
            }
            fn put(&self, out: &mut Writer) {
                out.payload_nested(|out| {
                    if !out.charge_with(|budget| budget.$budget::<(K, V)>(self.len())) { return; }
                    out.put_len(self.len());
                    for (key, value) in self {
                        key.put(out);
                        value.put(out);
                    }
                });
            }
            fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
                input.payload_nested(|input| {
                    let len = input.take_len(K::MIN + V::MIN)?;
                    input.charge_with(|budget| budget.$budget::<(K, V)>(len))?;
                    let mut map = collection_storage!($budget, input, len);
                    for _ in 0..len {
                        let key = K::take(input)?;
                        map.insert(key, V::take(input)?);
                    }
                    Ok(map)
                })
            }
        }
    )*};
}
maps!(
    BTreeMap[Ord] => btree, "btree-map:u32-le-length:v1";
    HashMap[Eq + Hash] => hash_table, "hash-map:u32-le-length:v1";
);

macro_rules! tuples {
    ($(($($name:ident),+)),*) => {$(
        #[allow(non_snake_case)]
        impl<$($name: Wire),+> Wire for ($($name,)+) {

            const MIN: usize = 0 $(+ $name::MIN)+;
            fn describe(schema: &mut WireSchema) -> u64 where Self: 'static {
                let hash = WireSchema::leaf("tuple");
                $(let hash = WireSchema::mix(hash, $name::describe(schema));)+
                hash
            }
            fn put(&self, out: &mut Writer) {
                let ($($name,)+) = self;
                $($name.put(out);)+
            }
            fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
                Ok(($($name::take(input)?,)+))
            }
        }
    )*};
}
tuples!(
    (A),
    (A, B),
    (A, B, C),
    (A, B, C, D),
    (A, B, C, D, E),
    (A, B, C, D, E, F)
);

/// A duration as seconds and nanoseconds.
impl Wire for std::time::Duration {
    crate::wire::layout_fields!("secs" => u64, "nanos" => u32);
    const MIN: usize = 12;
    fn put(&self, out: &mut Writer) {
        out.put(&self.as_secs());
        out.put(&self.subsec_nanos());
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        let secs = u64::take(input)?;
        let nanos = u32::take(input)?;
        match nanos < 1_000_000_000 {
            true => Ok(Self::new(secs, nanos)),
            false => Err(WireError::Invalid("nanoseconds past a second")),
        }
    }
}

// The same declarations feed the runtime fingerprint and the SDK surface generator.
macro_rules! layout_fields {
    ($availability:ident $contract:literal; $($field:literal => $ty:ty),* $(,)?) => {
        $crate::wire::layout_fields!(@describe ($availability) ($contract) $($field => $ty),*);
    };
    ($contract:literal; $($field:literal => $ty:ty),* $(,)?) => {
        $crate::wire::layout_fields!(@describe () ($contract) $($field => $ty),*);
    };
    ($($field:literal => $ty:ty),* $(,)?) => {
        $crate::wire::layout_fields!(@describe () () $($field => $ty),*);
    };
    (@describe ($($availability:ident)?) ($($contract:literal)?) $($field:literal => $ty:ty),*) => {
        fn describe(schema: &mut $crate::wire::WireSchema) -> u64 {
            use $crate::wire::WireSchema;
            $(schema.$availability();)?
            schema.named::<Self>(|schema| {
                let hash = WireSchema::leaf("wire-layout-v1");
                $(let hash = WireSchema::mix(hash, WireSchema::leaf($contract));)?
                $(
                    // Explicit enum tags are part of the bytes; field and variant names are not.
                    let hash = if let Some((tag, _)) = $field.split_once(':') {
                        WireSchema::mix(hash, WireSchema::leaf(tag))
                    } else {
                        hash
                    };
                    let hash = WireSchema::mix(hash, <$ty as $crate::wire::Wire>::describe(schema));
                )*
                hash
            })
        }
    };
}
pub(crate) use layout_fields;
macro_rules! layout_via {
    ($ty:ty, $contract:literal $(, $availability:ident)?) => {
        fn describe(schema: &mut $crate::wire::WireSchema) -> u64 {
            use $crate::wire::WireSchema;
            $(schema.$availability();)?
            schema.named::<Self>(|schema| {
                WireSchema::mix(
                    WireSchema::leaf($contract),
                    <$ty as $crate::wire::Wire>::describe(schema),
                )
            })
        }
    };
    ($ty:ty) => {
        fn describe(schema: &mut $crate::wire::WireSchema) -> u64 {
            schema.named::<Self>(|schema| <$ty as $crate::wire::Wire>::describe(schema))
        }
    };
}

#[cfg(test)]
mod tests {
    #[test]
    fn derive_shapes_roundtrip_with_golden_bytes() {
        #[derive(sunrise_mod_api_macros::Wire, Debug, PartialEq)]
        struct Named {
            x: u32,
            text: String,
        }
        #[derive(sunrise_mod_api_macros::Wire, Debug, PartialEq)]
        struct Tuple(u32);
        #[derive(sunrise_mod_api_macros::Wire, Debug, PartialEq)]
        struct Unit;
        #[derive(sunrise_mod_api_macros::Wire, Debug, PartialEq)]
        enum Plain {
            First = 7,
            Second = 9,
        }
        #[derive(sunrise_mod_api_macros::Wire, Debug, PartialEq)]
        enum Payload {
            Unit,
            Tuple(u8),
            Named { value: u32 },
        }
        #[derive(sunrise_mod_api_macros::Wire, Debug, PartialEq)]
        struct Generic<T> {
            value: T,
        }
        #[derive(sunrise_mod_api_macros::Wire, Debug, PartialEq)]
        struct Recursive {
            children: Vec<Recursive>,
        }
        fn check<T: super::Wire + std::fmt::Debug + PartialEq>(value: T, bytes: &[u8]) {
            let mut out = super::Writer::new();
            out.put(&value);
            assert_eq!(out.bytes(), bytes);
            assert_eq!(super::Reader::decode::<T>(bytes).unwrap(), value);
        }
        check(
            Named {
                x: 7,
                text: "ab".into(),
            },
            &[7, 0, 0, 0, 2, 0, 0, 0, 97, 98],
        );
        check(Tuple(7), &[7, 0, 0, 0]);
        check(Unit, &[]);
        check(Plain::First, &[0, 0, 0, 0]);
        check(Plain::Second, &[1, 0, 0, 0]);
        check(Payload::Tuple(5), &[1, 0, 0, 0, 5]);
        check(Payload::Named { value: 7 }, &[2, 0, 0, 0, 7, 0, 0, 0]);
        check(Generic { value: 5u8 }, &[5]);
        check(Recursive { children: vec![] }, &[0, 0, 0, 0]);
    }
    #[test]
    fn derive_field_hygiene_and_unknown_tags() {
        #[derive(mod_api::Wire, Debug, PartialEq)]
        struct Names {
            out: u8,
            input: u32,
        }
        #[derive(mod_api::Wire, Debug, PartialEq)]
        enum Event {
            Data { out: u8, input: u32 },
        }
        round(Names { out: 3, input: 7 });
        round(Event::Data { out: 3, input: 7 });
        assert_eq!(
            Reader::decode::<Event>(&[9, 0, 0, 0]),
            Err(WireError::Tag(9))
        );
        assert!(Reader::decode::<Names>(&[3, 7]).is_err());
    }

    #[test]
    fn derive_via_checks_its_constructor() {
        #[derive(sunrise_mod_api_macros::Wire, Debug, PartialEq)]
        #[wire(via = String)]
        struct Nonempty(String);
        impl From<&Nonempty> for String {
            fn from(value: &Nonempty) -> Self {
                value.0.clone()
            }
        }
        impl TryFrom<String> for Nonempty {
            type Error = ();
            fn try_from(value: String) -> Result<Self, Self::Error> {
                if value.is_empty() {
                    Err(())
                } else {
                    Ok(Self(value))
                }
            }
        }
        #[derive(mod_api::Wire, Debug, PartialEq)]
        struct Outer {
            first: u8,
            checked: Nonempty,
            last: u32,
        }
        assert_eq!(
            Reader::decode::<Outer>(&[1, 0, 0, 0, 0]),
            Err(WireError::Invalid("Nonempty"))
        );
        let mut out = Writer::new();
        out.put(&Nonempty("ab".into()));
        assert_eq!(out.bytes(), &[2, 0, 0, 0, 97, 98]);
        assert_eq!(
            Reader::decode::<Nonempty>(out.bytes()).unwrap(),
            Nonempty("ab".into())
        );
        assert_eq!(
            Reader::decode::<Nonempty>(&[0, 0, 0, 0]),
            Err(WireError::Invalid("Nonempty"))
        );
    }

    #[test]
    fn golden_value_bytes() {
        fn bytes<T: super::Wire>(value: T, expected: &[u8]) {
            let mut out = super::Writer::new();
            out.put(&value);
            assert_eq!(out.error(), None);
            assert_eq!(out.bytes(), expected);
        }
        bytes(
            crate::Vec3 {
                x: 1.0,
                y: 2.0,
                z: -0.5,
            },
            &[
                0, 0, 0, 0, 0, 0, 240, 63, 0, 0, 0, 0, 0, 0, 0, 64, 0, 0, 0, 0, 0, 0, 224, 191,
            ],
        );
        bytes(crate::Direction::North, &[2, 0, 0, 0]);
        bytes(crate::Tag::Byte(5), &[1, 0, 0, 0, 5]);
        bytes(Some(1u16), &[1, 1, 0]);
        bytes(vec!["ab".to_owned()], &[1, 0, 0, 0, 2, 0, 0, 0, 97, 98]);
        bytes(serde_json::json!(1), &[1, 0, 0, 0, 49]);
    }

    use super::*;

    fn round<T: Wire + PartialEq + std::fmt::Debug>(value: T) {
        let mut out = Writer::new();
        out.put(&value);
        assert_eq!(Reader::decode::<T>(out.bytes()), Ok(value));
    }

    #[test]
    fn reductions_survive_wire_in_order() {
        use crate::Reduction::*;
        let reductions = vec![
            Armor,
            Enchantments,
            MobEffects,
            Absorption,
            Invulnerability,
            InnateResistance,
        ];
        round(reductions);
        assert!(Reader::decode::<crate::Reduction>(&[6]).is_err());
    }

    #[test]
    fn provider_particle_state_survives_the_wire() {
        round(crate::ModParticle {
            position: [1.25, -2.5, 3.75],
            previous: [0.0, -1.0, 2.0],
            velocity: [0.1, 0.2, -0.3],
            age: 7,
            lifetime: 31,
            size: 0.5,
            rgba: [0.25, 0.5, 0.75, 1.0],
            frame: 3,
            removed: true,
            state: [8.0, -9.0, 10.0, 11.0],
            facing: crate::ParticleFacing::Fixed([0.5, -0.5, 0.5, 0.5]),
            roll: 0.75,
        });
    }

    #[test]
    fn a_particle_batch_survives_the_wire_in_order() {
        let mut second = crate::ModParticle {
            position: [9.0, 8.0, 7.0],
            previous: [0.0; 3],
            velocity: [0.0; 3],
            age: 2,
            lifetime: 4,
            size: 1.5,
            rgba: [1.0; 4],
            frame: 1,
            removed: false,
            state: [0.0; 4],
            facing: crate::ParticleFacing::AroundY,
            roll: -1.0,
        };
        let first = crate::ModParticle {
            facing: crate::ParticleFacing::Fixed([0.0, 0.0, 0.0, 1.0]),
            ..second
        };
        second.age = 3;
        round(crate::client::particle_providers::particle_batch_from_vec(
            vec![first, second],
        ));
        round(crate::client::particle_providers::particle_batch_from_vec(
            Vec::new(),
        ));
    }

    #[test]
    fn values_survive_a_round_trip() {
        round(7u32);
        round(-3i64);
        round(1.5f64);
        round(u128::MAX);
        round(true);
        round('x');
        round(String::from("héllo"));
        round(Some(vec![1u8, 2, 3]));
        round(None::<u16>);
        round((1u8, String::from("a"), [3u16; 4]));
        round(BTreeMap::from([
            (1u8, String::from("one")),
            (2, String::from("two")),
        ]));
        round(std::time::Duration::new(5, 7));
        round(vec![Some(Box::new(4i32)), None]);
    }

    #[test]
    fn bad_input_is_an_error_not_a_panic_or_a_huge_allocation() {
        // A list that promises four billion items in six bytes.
        assert_eq!(
            Reader::decode::<Vec<u64>>(&[0xff, 0xff, 0xff, 0xff, 0, 0]),
            Err(WireError::Length)
        );
        assert_eq!(Reader::decode::<u32>(&[1, 2]), Err(WireError::Eof));
        assert_eq!(Reader::decode::<bool>(&[2]), Err(WireError::Tag(2)));
        assert_eq!(Reader::decode::<Option<u8>>(&[9]), Err(WireError::Tag(9)));
        assert_eq!(
            Reader::decode::<String>(&[2, 0, 0, 0, 0xff, 0xfe]),
            Err(WireError::Utf8)
        );
        assert_eq!(Reader::decode::<u8>(&[1, 2]), Err(WireError::Trailing));
        assert!(Reader::decode::<char>(&0xd800u32.to_le_bytes()).is_err());
        assert!(
            Reader::decode::<std::time::Duration>(
                &[0; 8]
                    .iter()
                    .chain(&[0xff; 4])
                    .copied()
                    .collect::<Vec<u8>>()
            )
            .is_err()
        );
    }

    #[test]
    fn nesting_past_the_limit_is_refused() {
        let mut bytes = vec![1u8; MAX_DEPTH as usize + 1];
        bytes.push(0);
        let mut shallow = bytes[bytes.len() - MAX_DEPTH as usize..].to_vec();
        shallow.push(0);
        // `Box<Option<…>>` chains are not a real type; a depth guard is exercised directly.
        let mut reader = Reader::new(&bytes);
        fn dive(reader: &mut Reader<'_>, left: usize) -> Result<(), WireError> {
            match left {
                0 => Ok(()),
                _ => reader.nested(|r| dive(r, left - 1)),
            }
        }
        assert_eq!(dive(&mut reader, MAX_DEPTH as usize), Ok(()));
        assert_eq!(
            dive(&mut reader, MAX_DEPTH as usize + 1),
            Err(WireError::Depth)
        );
        // The depth unwinds: a refused read leaves the reader reusable.
        assert_eq!(dive(&mut reader, 3), Ok(()));
    }
}

#[cfg(test)]
mod depth_tests;
mod json_tree;
mod tag_tree;
mod text_tree;
pub(crate) mod wire_types;
