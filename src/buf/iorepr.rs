use core::{mem::MaybeUninit, num::NonZero};

use super::{BytesPtr, BytesPtrMut, MutSafe, ReadSafe, UninitSlice, WriteSafe};

#[repr(C)]
#[derive(Clone, Copy)]
#[allow(unused)]
struct IoSliceUnix {
    pub ptr: *mut core::ffi::c_void,
    pub len: usize,
}

#[allow(unused)]
impl IoSliceUnix {
    pub const fn new(ptr: *mut u8, len: usize) -> Self {
        IoSliceUnix { ptr: ptr as *mut core::ffi::c_void, len }
    }
    #[inline]
    pub const fn ptr(&self) -> *mut u8 {
        self.ptr as *mut u8
    }
    #[inline]
    pub const fn len(&self) -> usize {
        self.len
    }
    pub fn advance(&mut self, len: usize) -> Option<NonZero<usize>> {
        let offset = core::cmp::min(len, self.len);
        unsafe {
            self.ptr = self.ptr.byte_offset(offset.try_into().unwrap());
        }
        let retval = NonZero::new(len.saturating_sub(self.len));
        self.len = self.len.saturating_sub(len);
        retval
    }
}
unsafe impl Send for IoSliceUnix {}
unsafe impl Sync for IoSliceUnix {}

#[repr(C)]
#[derive(Clone, Copy)]
#[allow(unused)]
struct IoSliceWin {
    pub len: core::ffi::c_ulong,
    pub ptr: *mut core::ffi::c_char,
}

#[allow(unused)]
impl IoSliceWin {
    pub const fn new(ptr: *mut u8, len: usize) -> Self {
        let len = if len <= core::ffi::c_ulong::MAX as usize {
            len as core::ffi::c_ulong
        } else {
            core::ffi::c_ulong::MAX
        };
        IoSliceWin { len, ptr: ptr as *mut core::ffi::c_char }
    }
    #[inline]
    pub const fn ptr(&self) -> *mut u8 {
        self.ptr as *mut u8
    }
    #[inline]
    pub const fn len(&self) -> usize {
        self.len as usize
    }
    pub fn advance(&mut self, len: usize) -> Option<NonZero<usize>> {
        let len: core::ffi::c_ulong = len.try_into().unwrap();
        let offset = core::cmp::min(len, self.len);
        // On 64-bit platforms, the below try_into will always succeed.
        // Let the optimizer figure that out.
        unsafe {
            self.ptr = self.ptr.byte_offset(offset.try_into().unwrap());
        }
        let retval = NonZero::new(len.saturating_sub(self.len) as usize);
        self.len = self.len.saturating_sub(len);
        retval
    }
}
unsafe impl Send for IoSliceWin {}
unsafe impl Sync for IoSliceWin {}

#[cfg(target_os = "windows")]
type IoSliceInner = IoSliceWin;
#[cfg(not(target_os = "windows"))]
type IoSliceInner = IoSliceUnix;

/// Representation of byte buffers that is compatible with typical system ABIs for vectored I/O.
///
/// As with [`std::io::IoSlice`], its representation is guaranteed to match
/// [`iovec`](https://www.man7.org/linux/man-pages/man3/iovec.3type.html) on UNIXes
/// and [`WSABUF`](https://learn.microsoft.com/en-us/windows/win32/api/ws2def/ns-ws2def-wsabuf)
/// on Windows. Its representation should not be relied upon on other platforms.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct IoRepr<Slice>(IoSliceInner, core::marker::PhantomData<Slice>);

impl<T> IoRepr<T> {
    /// Advances the buffer, saturating and returning the remainder.
    pub fn advance(&mut self, len: usize) -> Option<NonZero<usize>> {
        self.0.advance(len)
    }
    /// Returns the length of the buffer in bytes.
    pub fn len(&self) -> usize {
        self.0.len()
    }
    /// Returns `true` if `self` is empty.
    pub fn is_empty(&self) -> bool {
        self.0.len() == 0
    }
    /// Returns a const pointer to the slice referenced by `self`.
    pub fn as_ptr(&self) -> *const [u8] {
        core::ptr::slice_from_raw_parts(self.0.ptr(), self.0.len())
    }
}

impl<T: BytesPtrMut> IoRepr<T> {
    /// Returns a mut pointer to the slice referenced by `self`.
    pub fn as_ptr_mut(&mut self) -> *mut [u8] {
        core::ptr::slice_from_raw_parts_mut(self.0.ptr(), self.0.len())
    }
}

impl<T: ReadSafe> IoRepr<T> {
    /// Returns the contained data as a slice.
    pub fn as_slice(&self) -> &[u8] {
        unsafe { self.as_ptr().as_ref().unwrap_unchecked() }
    }
}
impl<T: WriteSafe> IoRepr<T> {
    /// Converts a `&mut Self` into an [`UninitSlice`].
    pub fn as_slice_uninit(&mut self) -> UninitSlice<'_> {
        let slice =
            unsafe { (self.as_ptr_mut() as *mut [MaybeUninit<u8>]).as_mut().unwrap_unchecked() };
        UninitSlice::uninit(slice)
    }
}
impl<T: MutSafe> IoRepr<T> {
    /// Returns the contained data as a mutable slice.
    pub fn as_slice_mut(&mut self) -> &mut [u8] {
        unsafe { self.as_ptr_mut().as_mut().unwrap_unchecked() }
    }
}

impl<'a> IoRepr<UninitSlice<'a>> {
    /// Special-cased `const` constructor for `IoRepr`s meant to be written to.
    pub const fn new_write(mut slice: UninitSlice<'a>) -> Self {
        let ptr = slice.as_mut_ptr().cast();
        let len = slice.len();
        Self(IoSliceInner::new(ptr, len), core::marker::PhantomData)
    }
}

impl<'a> IoRepr<&'a [u8]> {
    /// Special-cased `const` constructor for `IoRepr`s meant to be read from.
    pub const fn new_read(slice: &'a [u8]) -> Self {
        let ptr = slice.as_ptr().cast_mut();
        let len = slice.len();
        Self(IoSliceInner::new(ptr, len), core::marker::PhantomData)
    }
}

impl<T: BytesPtr> IoRepr<T> {
    pub fn new(slice: T) -> Self {
        let slice = slice.into_bytes_ptr();
        // Obviously hazardous const casts go weee
        // TODO: Use `.cast_mut().as_mut_ptr()` when stable.
        let ptr = slice as *mut u8;
        let len = slice.len();
        Self(IoSliceInner::new(ptr, len), core::marker::PhantomData)
    }
    pub fn into_inner(self) -> T {
        let ptr = core::ptr::slice_from_raw_parts(self.0.ptr(), self.0.len);
        unsafe { T::from_bytes_ptr(ptr) }
    }
}

impl<'a, T> core::ops::Deref for IoRepr<&'a T>
where
    &'a T: BytesPtr,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        let ptr = core::ptr::slice_from_raw_parts(self.0.ptr(), self.0.len);
        unsafe { BytesPtr::from_bytes_ptr(ptr) }
    }
}

impl<'a, T> core::ops::Deref for IoRepr<&'a mut T>
where
    &'a T: BytesPtr,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { BytesPtr::from_bytes_ptr(self.as_ptr()) }
    }
}
impl<'a, T> core::ops::DerefMut for IoRepr<&'a mut T>
where
    &'a T: BytesPtr,
    &'a mut T: BytesPtrMut,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { BytesPtr::from_bytes_ptr(self.as_ptr()) }
    }
}
