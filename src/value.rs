use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::rc::Rc;

/// Bytes kept in the value itself. A city-lots coordinate fits, so the
/// stream parser does not allocate one string per number.
const INLINE: usize = 23;
const HEAP_TAG: u8 = 0xFF;

/// The two words of a `*const str`.
#[repr(C)]
struct FatStr {
    data: *const u8,
    len: usize,
}

/// Characters several values can share.
///
/// Short text lives in the value. Longer text is reference-counted, so a
/// path copied for every `--stream` item does not allocate those bytes again.
#[repr(C, align(8))]
pub struct Text {
    buf: [u8; INLINE],
    /// `0..=INLINE` is the inline length. [`HEAP_TAG`] stores an `Rc<str>`
    /// pointer in the first word of `buf`.
    tag: u8,
    _thread: PhantomData<Rc<()>>,
}

impl Text {
    pub fn as_str(&self) -> &str {
        if self.tag != HEAP_TAG {
            // Only `From` writes these bytes, and it writes them from a `str`.
            unsafe { std::str::from_utf8_unchecked(&self.buf[..self.tag as usize]) }
        } else {
            unsafe { &*self.heap_ptr() }
        }
    }

    fn blank(len: u8) -> Self {
        Self {
            buf: [0; INLINE],
            tag: len,
            _thread: PhantomData,
        }
    }

    fn heap_ptr(&self) -> *const str {
        let mut word = [0u8; 8];
        word.copy_from_slice(&self.buf[..8]);
        let data = usize::from_ne_bytes(word) as *const u8;
        let mut len_bytes = [0u8; 8];
        len_bytes.copy_from_slice(&self.buf[8..16]);
        // `*const str` is the data address and the byte length.
        let fat = FatStr {
            data,
            len: usize::from_ne_bytes(len_bytes),
        };
        unsafe { std::mem::transmute(fat) }
    }

    fn from_heap(ptr: *const str) -> Self {
        let fat: FatStr = unsafe { std::mem::transmute(ptr) };
        let mut text = Self::blank(HEAP_TAG);
        text.buf[..8].copy_from_slice(&(fat.data as usize).to_ne_bytes());
        text.buf[8..16].copy_from_slice(&fat.len.to_ne_bytes());
        text
    }

    fn share(text: &str) -> Self {
        if text.len() <= INLINE {
            let mut out = Self::blank(text.len() as u8);
            out.buf[..text.len()].copy_from_slice(text.as_bytes());
            out
        } else {
            Self::from_heap(Rc::into_raw(Rc::<str>::from(text)))
        }
    }
}

impl Clone for Text {
    fn clone(&self) -> Self {
        if self.tag == HEAP_TAG {
            unsafe { Rc::increment_strong_count(self.heap_ptr()) };
        }
        Self {
            buf: self.buf,
            tag: self.tag,
            _thread: PhantomData,
        }
    }
}

impl Drop for Text {
    fn drop(&mut self) {
        if self.tag == HEAP_TAG {
            let ptr = self.heap_ptr();
            unsafe { drop(Rc::from_raw(ptr)) };
        }
    }
}

impl PartialEq for Text {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for Text {}

impl PartialOrd for Text {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Text {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl Hash for Text {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

impl std::ops::Deref for Text {
    type Target = str;

    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for Text {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::borrow::Borrow<str> for Text {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl From<&str> for Text {
    fn from(text: &str) -> Self {
        Text::share(text)
    }
}

impl From<String> for Text {
    fn from(text: String) -> Self {
        Text::share(&text)
    }
}

impl From<&String> for Text {
    fn from(text: &String) -> Self {
        Text::share(text)
    }
}

impl std::fmt::Debug for Text {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self.as_str(), formatter)
    }
}

impl std::fmt::Display for Text {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A value several parents can hold. Cloning bumps the count. A write copies
/// the container first when another parent still has it, so the other parent
/// keeps the old contents.
#[derive(Debug)]
pub struct Share<T>(Rc<T>);

impl<T> Clone for Share<T> {
    fn clone(&self) -> Self {
        Share(Rc::clone(&self.0))
    }
}

impl<T> std::ops::Deref for Share<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> Share<T> {
    pub fn new(value: T) -> Self {
        Share(Rc::new(value))
    }

    /// The value, with no other parent. `None` leaves this share in place.
    pub(crate) fn unique_mut(&mut self) -> Option<&mut T> {
        Rc::get_mut(&mut self.0)
    }
}

impl<T: Clone> Share<T> {
    /// A container this owner can write. Other owners keep the previous one.
    pub(crate) fn make_mut(&mut self) -> &mut T {
        Rc::make_mut(&mut self.0)
    }

    pub(crate) fn unwrap_or_clone(self) -> T {
        match Rc::try_unwrap(self.0) {
            Ok(value) => value,
            Err(shared) => (*shared).clone(),
        }
    }
}

fn empty_array() -> Rc<Vec<Value>> {
    thread_local! {
        static EMPTY: Rc<Vec<Value>> = Rc::new(Vec::new());
    }
    // A value can be dropped while thread-local storage is going away.
    // `with` panics then. A fresh empty array is only a placeholder.
    EMPTY
        .try_with(Rc::clone)
        .unwrap_or_else(|_| Rc::new(Vec::new()))
}

struct VecPool {
    bins: [Vec<Vec<Value>>; 5],
    /// Unique array boxes waiting for the next short array.
    arrays: Vec<Rc<Vec<Value>>>,
}

impl Drop for VecPool {
    fn drop(&mut self) {
        // Bins still hold values. Recycling those would borrow this pool
        // while it is going away.
        let _ = POOL_OPEN.try_with(|flag| flag.set(false));
    }
}

thread_local! {
    static POOL_OPEN: Cell<bool> = const { Cell::new(true) };
    static POOL: RefCell<VecPool> = const { RefCell::new(VecPool {
        bins: [Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()],
        arrays: Vec::new(),
    }) };
}

fn vec_bin(cap: usize) -> Option<usize> {
    Some(match cap {
        0..=2 => 0,
        3..=4 => 1,
        5..=8 => 2,
        9..=16 => 3,
        17..=32 => 4,
        _ => return None,
    })
}

/// A buffer for a short array. `--stream` builds one path per item.
pub(crate) fn scratch_vec(min_cap: usize) -> Vec<Value> {
    let open = POOL_OPEN.try_with(Cell::get).unwrap_or(false);
    if !open {
        return Vec::with_capacity(min_cap);
    }
    POOL.with(|pool| {
        let mut pool = pool.borrow_mut();
        let start = vec_bin(min_cap).unwrap_or(4);
        for bin in start..pool.bins.len() {
            if let Some(items) = pool.bins[bin].pop() {
                return items;
            }
        }
        Vec::with_capacity(min_cap.max(2))
    })
}

fn recycle_vec(mut items: Vec<Value>) {
    items.clear();
    let Some(bin) = vec_bin(items.capacity()) else {
        return;
    };
    if !POOL_OPEN.try_with(Cell::get).unwrap_or(false) {
        return;
    }
    let _ = POOL.try_with(|pool| {
        if let Ok(mut pool) = pool.try_borrow_mut() {
            if pool.bins[bin].len() < 2048 {
                pool.bins[bin].push(items);
            }
        }
    });
}

/// A short array box. The stream builds two of these per item.
fn take_array(mut items: Vec<Value>) -> Rc<Vec<Value>> {
    if items.capacity() > 32 {
        return Rc::new(items);
    }
    let open = POOL_OPEN.try_with(Cell::get).unwrap_or(false);
    let popped = if open {
        POOL.try_with(|pool| pool.borrow_mut().arrays.pop())
            .ok()
            .flatten()
    } else {
        None
    };
    if let Some(mut rc) = popped {
        if let Some(slot) = Rc::get_mut(&mut rc) {
            slot.clear();
            slot.append(&mut items);
            recycle_vec(items);
            return rc;
        }
    }
    Rc::new(items)
}

fn recycle_array(rc: Rc<Vec<Value>>) {
    if !POOL_OPEN.try_with(Cell::get).unwrap_or(false) {
        return;
    }
    let _ = POOL.try_with(|pool| {
        if let Ok(mut pool) = pool.try_borrow_mut() {
            if pool.arrays.len() < 8192 {
                pool.arrays.push(rc);
                return;
            }
        }
        drop(rc);
    });
}

impl<'a, T> IntoIterator for &'a Share<Vec<T>> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

#[derive(Debug)]
pub enum Value {
    Null,
    Bool(bool),
    /// Text jq would print for this number.
    Number(Text),
    String(Text),
    Array(Share<Vec<Value>>),
    Object(Share<Vec<(String, Value)>>),
}

/// Past this depth the rest of a nest is freed on the heap. A wide array
/// still drops in a loop.
const DROP_RECURSION: usize = 32;

impl Drop for Value {
    fn drop(&mut self) {
        drop_in_place(self, 0);
    }
}

fn drop_in_place(value: &mut Value, depth: usize) {
    if depth >= DROP_RECURSION {
        drop_deep(value);
        return;
    }
    match value {
        Value::Array(items) => {
            // A stream item is thrown away after the filter sees it. The
            // box comes back for the next item instead of going to the
            // allocator. Another parent keeps this array as it is.
            if Rc::strong_count(&items.0) != 1 {
                return;
            }
            let mut rc = std::mem::replace(&mut items.0, empty_array());
            let recycle = if let Some(vec) = Rc::get_mut(&mut rc) {
                for mut item in vec.drain(..) {
                    drop_in_place(&mut item, depth + 1);
                }
                vec.capacity() <= 32
            } else {
                false
            };
            if recycle {
                recycle_array(rc);
            }
        }
        Value::Object(items) => {
            if let Some(items) = items.unique_mut() {
                for (_, mut item) in std::mem::take(items) {
                    drop_in_place(&mut item, depth + 1);
                }
            }
        }
        _ => {}
    }
}

/// Children are removed before each value is dropped, so `Drop` sees an
/// empty container and does not walk the nest again.
fn drop_deep(value: &mut Value) {
    let mut stack = Vec::new();
    take_children(value, &mut stack);
    while let Some(mut next) = stack.pop() {
        take_children(&mut next, &mut stack);
    }
}

fn take_children(value: &mut Value, stack: &mut Vec<Value>) {
    match value {
        Value::Array(items) => {
            if let Some(items) = items.unique_mut() {
                stack.extend(std::mem::take(items));
            }
        }
        Value::Object(items) => {
            if let Some(items) = items.unique_mut() {
                stack.extend(std::mem::take(items).into_iter().map(|(_, item)| item));
            }
        }
        _ => {}
    }
}

impl Clone for Value {
    fn clone(&self) -> Self {
        match self {
            Value::Null => Value::Null,
            Value::Bool(bit) => Value::Bool(*bit),
            Value::Number(text) => Value::Number(text.clone()),
            Value::String(text) => Value::String(text.clone()),
            // The copy sees the same array or object until a write.
            Value::Array(items) => Value::Array(items.clone()),
            Value::Object(items) => Value::Object(items.clone()),
        }
    }
}

impl Value {
    pub(crate) fn into_string(mut self) -> Result<String, Self> {
        match &mut self {
            Value::String(text) => Ok(std::mem::replace(text, Text::from("")).to_string()),
            _ => Err(self),
        }
    }

    pub(crate) fn into_array(mut self) -> Result<Vec<Value>, Self> {
        match &mut self {
            Value::Array(items) => {
                let taken = std::mem::replace(items, Share::new(Vec::new()));
                Ok(taken.unwrap_or_clone())
            }
            _ => Err(self),
        }
    }

    pub(crate) fn into_object(mut self) -> Result<Vec<(String, Value)>, Self> {
        match &mut self {
            Value::Object(items) => {
                let taken = std::mem::replace(items, Share::new(Vec::new()));
                Ok(taken.unwrap_or_clone())
            }
            _ => Err(self),
        }
    }

    pub fn array(items: Vec<Value>) -> Self {
        Value::Array(Share(take_array(items)))
    }

    pub fn object(pairs: Vec<(String, Value)>) -> Self {
        Value::Object(Share::new(pairs))
    }

    pub(crate) fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "boolean",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Text;

    #[test]
    fn text_roundtrip_short_and_long() {
        for len in [0, 1, 23, 24, 80] {
            let text_src = "y".repeat(len);
            let text = Text::from(text_src.as_str());
            assert_eq!(&*text, text_src);
            let copy = text.clone();
            drop(text);
            assert_eq!(&*copy, text_src);
        }
        let left = Text::from("same-long-string-value-aaaa");
        let right = Text::from("same-long-string-value-aaaa");
        assert_eq!(left, right);
    }
}
