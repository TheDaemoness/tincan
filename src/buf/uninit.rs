use core::mem::MaybeUninit;

use super::{BytesPtr, BytesPtrMut, WriteSafe};

/// A reference to maybe-uninitialized data that cannot be deinitialized.
///
/// This is essentially a `&'a mut [MaybeUninit<u8>]` that, like `Pin`,
/// does not allow you to get the underlying reference.
/// Where `Pin` does this to prevent pointed-to data from moving,
/// this type uses it to prevent copying uninitialized data to the destination,
/// possibly deinitializing the pointee.
///
/// This makes it safe to construct `self` out of references to mutable data.
///
/// This is similar to Tokio's `ReadBuf`, but unlike that type it does not keep track of
/// what part of the buffer is initialized but not filled, which may result in redundant
/// initialization. The benefit is that this type can be represented by a
/// thin pointer and length, which in turn means it can be used for vectorized I/O
/// with [`IoRepr`][crate::buf::IoRepr].
#[repr(transparent)]
pub struct UninitSlice<'a>(&'a mut [MaybeUninit<u8>]);

impl<'a> core::ops::Deref for UninitSlice<'a> {
    type Target = [MaybeUninit<u8>];

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl<'a> core::default::Default for UninitSlice<'a> {
    fn default() -> Self {
        Self::empty()
    }
}

impl<'a> UninitSlice<'a> {
    /// Constructs an empty `Self`.
    pub const fn empty() -> Self {
        // Safety: Dangling pointer, len 0.
        Self(unsafe { core::slice::from_raw_parts_mut(core::ptr::dangling_mut(), 0) })
    }
    /// Constructs `self` out of a mutable reference to initialized data.
    pub fn new(ref_mut: &'a mut [u8]) -> Self {
        let ptr = ref_mut.as_mut_ptr() as *mut MaybeUninit<u8>;
        let len = ref_mut.len();
        UninitSlice(unsafe { core::slice::from_raw_parts_mut(ptr, len) })
    }
    /// Constructs `self` out of a mutable reference to maybe-initialized data.
    pub fn uninit(ref_mut: &'a mut [MaybeUninit<u8>]) -> Self {
        UninitSlice(ref_mut)
    }
    /// Returns a const pointer to the starting address of the data pointed to by `self`.
    ///
    /// This function is equivalent to calling [`as_ptr`](slice::as_ptr) on the internal slice.
    pub const fn as_ptr(&self) -> *const MaybeUninit<u8> {
        self.0.as_ptr()
    }
    /// Returns a mutable pointer to the starting address of the data pointed to by `self`.
    ///
    /// This function is equivalent to calling [`as_ptr`](slice::as_ptr) on the internal slice.
    pub const fn as_mut_ptr(&mut self) -> *mut MaybeUninit<u8> {
        self.0.as_mut_ptr()
    }
    /// Returns the length in bytes of data pointed to by `self`.
    ///
    /// This function is equivalent to calling [`len`](slice::len) on the internal slice.
    pub const fn len(&self) -> usize {
        self.0.len()
    }
    /// Returns `true` if `self` is empty.
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[inline]
    unsafe fn do_advance(&mut self, to_advance: usize) -> *mut MaybeUninit<u8> {
        // Safety: Pointer is advanced by no more than len.
        let ptr = unsafe { self.0.as_mut_ptr().byte_add(to_advance) };
        let ptr_slice = core::ptr::slice_from_raw_parts_mut(ptr, self.0.len() - to_advance);
        // Safety: Pointer points to a range in the original slice.
        self.0 = unsafe { ptr_slice.as_mut().unwrap_unchecked() };
        ptr
    }
    /// Shortens the buffer from the front. If `count` is longer than `usize`, returns the excess.
    pub fn advance(&mut self, count: usize) -> usize {
        let len_orig = self.0.len();
        let to_advance = core::cmp::min(len_orig, count);
        unsafe { self.do_advance(to_advance) };
        core::cmp::max(count, len_orig) - len_orig
    }
    /// Writes data from the provided slice. Returns reference to the written bytes.
    ///
    /// If the provided slice is longer than `self`, only writes the first `len` bytes.
    pub fn write_from(&mut self, src: &[u8]) -> &'a mut [u8] {
        let count = core::cmp::min(self.len(), src.len());
        let src = unsafe { src.split_at_unchecked(count).0 };
        let src = core::ptr::from_ref(src) as *const [MaybeUninit<u8>];
        self.0.copy_from_slice(unsafe { src.as_ref().unwrap_unchecked() });
        let ptr = unsafe { self.do_advance(count) };
        unsafe { core::slice::from_raw_parts_mut(ptr as *mut u8, count) }
    }
    /// Writes data from the provided closure. Returns how many bytes were written.
    ///
    /// The closure should return how many bytes it wrote, which must be no more than `max`.
    /// Internally, the underlying buffer is guaranteed to be initialized before the write,
    /// but its value must not be relied upon.
    pub fn write_with(&mut self, max: usize, f: impl FnOnce(&mut [u8]) -> usize) -> usize {
        let written = f(self.zeroed(max));
        assert!(written <= max);
        // Safety: That's what the above assert is for.
        unsafe {
            self.do_advance(written);
        }
        written
    }
    #[inline]
    unsafe fn do_zero(&mut self, len: usize) -> *mut [u8] {
        let slice = unsafe { self.0.split_at_mut_unchecked(len).0 };
        slice.fill(MaybeUninit::zeroed());
        let ptr = slice.as_mut_ptr() as *mut u8;
        core::ptr::slice_from_raw_parts_mut(ptr, len)
    }
    /// Zeroes `len` bytes at the start of `self` and returns a reference to them.
    pub fn zeroed(&mut self, len: usize) -> &mut [u8] {
        let len = core::cmp::min(len, self.0.len());
        unsafe { self.do_zero(len).as_mut().unwrap_unchecked() }
    }
    /// Zeroes the entire slice and converts it into `&mut [u8]`.
    pub fn into_zeroed(mut self) -> &'a mut [u8] {
        let len = self.0.len();
        unsafe { self.do_zero(len).as_mut().unwrap_unchecked() }
    }
}

unsafe impl<'a> BytesPtr for UninitSlice<'a> {
    fn into_bytes_ptr(self) -> *const [u8] {
        let ptr = self.0.as_ptr() as *const u8;
        let len = self.0.len();
        core::ptr::slice_from_raw_parts(ptr, len)
    }

    unsafe fn from_bytes_ptr(this: *const [u8]) -> Self {
        let this = this as *mut [MaybeUninit<u8>];
        // Safety: Enforced by `from_bytes_ptr`'s preconditions for this type.
        Self(unsafe { this.as_mut().unwrap_unchecked() })
    }
}
unsafe impl<'a> BytesPtrMut for UninitSlice<'a> {}
unsafe impl<'a> WriteSafe for UninitSlice<'a> {}
