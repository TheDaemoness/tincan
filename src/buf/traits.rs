use core::mem::MaybeUninit;

use super::{IoRepr, UninitSlice};

/// Trait for cheaply converting `self` into a fat pointer to a slice and back.
///
/// # Safety
/// Implementing this trait asserts not only that `self` can be converted to a pointer,
/// but that `self` can be recovered from that pointer.
/// More generally, it must be possible to create an object of type `Self`
/// from certain pointers derived from the return value of [`BytesPtr::into_bytes_ptr`].
/// See [`BytesPtr::from_bytes_ptr`] for information about what pointers `Self` can be
/// constructed from.
///
/// For instance, `&str` cannot implement this trait because it cannot be constructed from
/// an arbitrary narrower slice of the data it points to.
pub unsafe trait BytesPtr {
    /// Returns `self` as a pointer to a slice of bytes.
    fn into_bytes_ptr(self) -> *const [u8];
    /// Converts a pointer to a slice of bytes into `Self`.
    ///
    /// # Safety
    /// `this` must have been derived from a pointer previously returned by `into_bytes_ptr`
    /// that is no-less-valid than a single instance of the original pointer.
    /// Specifically:
    /// * It must have the same provenance.
    /// * It must point to the same address range or an arbitrary narrower one.
    /// * If `Self` is not `Send`,
    ///   this method must run on the same thread as the `into_bytes_ptr` call did.
    /// * Creating, dereferencing, or dropping the return value
    ///   must not result in an invalid value or undefined behavior e.g. due to a race condition.
    /// * If the returned pointer is valid for reads,
    ///   the pointed-to data must not mutate (except for permitted interior mutability)
    ///   between the call to `into_bytes_ptr` and the call to `from_bytes_ptr`.
    unsafe fn from_bytes_ptr(this: *const [u8]) -> Self;
}

/// Asserts that the pointer returned by [`BytesPtr`], if valid, can be used for arbitrary writes.
///
/// In other words, asserts that the returned pointer can be sensibly cast to `*mut [u8]`
/// and that any bit pattern is valid for the pointee of `Self`.
///
/// # Safety
/// Implementing this trait removes a precondition of [`BytesPtr::from_bytes_ptr`] that
/// the pointed-to data must not have been modified between calls.
///
/// For instance, `&mut [NonZeroU8]` cannot implement this trait because `0` is not a valid value.
pub unsafe trait BytesPtrMut: BytesPtr {}

/// Asserts that the pointer returned by [`BytesPtr`] can be safely converted to a `&[u8]`.
///
/// # Safety
/// The returned pointer must satisfy all the preconditions of [`core::slice::from_raw_parts`].
pub unsafe trait ReadSafe: BytesPtr {}

/// Asserts that the pointer returned by [`BytesPtr`] can be safely written to.
///
/// # Safety
/// The returned pointer must be valid for any write within its bounds.
pub unsafe trait WriteSafe: BytesPtrMut {}

/// Asserts that the pointer returned by [`BytesPtr`] can be safely converted to a `&mut [u8]`.
///
/// # Safety
/// The returned pointer must satisfy all the preconditions of [`core::slice::from_raw_parts_mut`].
pub unsafe trait MutSafe: ReadSafe + WriteSafe {}

unsafe impl BytesPtr for *const [u8] {
    fn into_bytes_ptr(self) -> *const [u8] {
        self
    }

    unsafe fn from_bytes_ptr(this: *const [u8]) -> Self {
        this
    }
}
unsafe impl BytesPtr for *mut [u8] {
    fn into_bytes_ptr(self) -> *const [u8] {
        self.cast_const()
    }

    unsafe fn from_bytes_ptr(this: *const [u8]) -> Self {
        this.cast_mut()
    }
}
unsafe impl BytesPtrMut for *mut [u8] {}

unsafe impl BytesPtr for *const [MaybeUninit<u8>] {
    fn into_bytes_ptr(self) -> *const [u8] {
        self as *const [u8]
    }

    unsafe fn from_bytes_ptr(this: *const [u8]) -> Self {
        this as *const [MaybeUninit<u8>]
    }
}
unsafe impl BytesPtr for *mut [MaybeUninit<u8>] {
    fn into_bytes_ptr(self) -> *const [u8] {
        self as *const [u8]
    }

    unsafe fn from_bytes_ptr(this: *const [u8]) -> Self {
        this as *mut [MaybeUninit<u8>]
    }
}
unsafe impl BytesPtrMut for *mut [MaybeUninit<u8>] {}

unsafe impl BytesPtr for &[u8] {
    fn into_bytes_ptr(self) -> *const [u8] {
        core::ptr::slice_from_raw_parts(self.as_ptr(), self.len())
    }

    unsafe fn from_bytes_ptr(this: *const [u8]) -> Self {
        // TODO: Use `.as_ptr()` when it stabilizes.
        let ptr = this as *const u8;
        let len = this.len();
        unsafe { core::slice::from_raw_parts(ptr, len) }
    }
}
unsafe impl ReadSafe for &[u8] {}

unsafe impl BytesPtr for &mut [u8] {
    fn into_bytes_ptr(self) -> *const [u8] {
        core::ptr::slice_from_raw_parts(self.as_ptr(), self.len())
    }

    unsafe fn from_bytes_ptr(this: *const [u8]) -> Self {
        // TODO: Use `.cast_mut().as_mut_ptr()` when it stabilizes.
        let ptr = this as *mut u8;
        let len = this.len();
        unsafe { core::slice::from_raw_parts_mut(ptr, len) }
    }
}
unsafe impl ReadSafe for &mut [u8] {}
unsafe impl BytesPtrMut for &mut [u8] {}
unsafe impl WriteSafe for &mut [u8] {}
unsafe impl MutSafe for &mut [u8] {}

unsafe impl BytesPtr for &[MaybeUninit<u8>] {
    fn into_bytes_ptr(self) -> *const [u8] {
        core::ptr::slice_from_raw_parts(self.as_ptr() as *const u8, self.len())
    }

    unsafe fn from_bytes_ptr(this: *const [u8]) -> Self {
        let ptr = this as *mut MaybeUninit<u8>;
        let len = this.len();
        unsafe { core::slice::from_raw_parts_mut(ptr, len) }
    }
}
unsafe impl BytesPtr for &mut [MaybeUninit<u8>] {
    fn into_bytes_ptr(self) -> *const [u8] {
        core::ptr::slice_from_raw_parts(self.as_ptr() as *const u8, self.len())
    }

    unsafe fn from_bytes_ptr(this: *const [u8]) -> Self {
        let ptr = this as *mut MaybeUninit<u8>;
        let len = this.len();
        unsafe { core::slice::from_raw_parts_mut(ptr, len) }
    }
}
unsafe impl BytesPtrMut for &mut [MaybeUninit<u8>] {}
unsafe impl WriteSafe for &mut [MaybeUninit<u8>] {}

/// Trait for reading from buffer-like structures.
///
/// This trait has a notion of discontinuous buffers
/// which can be read from using vectorized I/O.
pub trait BufRead {
    /// Writes references to `self`'s buffers to `bufs`.
    ///
    /// If there are fewer buffers internally than the length of `bufs`,
    /// this function may leave a contiguous range of references at the end of `bufs` unchanged.
    fn get_read_bufs<'a, 'b: 'a>(&'b self, bufs: &'a mut [IoRepr<&'b [u8]>]);
    /// Mark `len` bytes as having been read.
    fn consume(&mut self, len: usize);
    /// Returns the suggested length of the `output` parameter of `get_read_bufs`.
    /// If `0` is returned, there is no data to read.
    ///
    /// Any non-zero values returned by this function are hints and should not be relied upon.
    fn read_bufs_hint(&self) -> usize {
        1
    }
}

/// Trait for writing to buffer-like structures.
///
/// This trait has a notion of discontinuous buffers,
/// which can be written to using vectorized I/O.
pub trait BufWrite {
    /// Writes references to `self`'s buffers to `bufs`.
    ///
    /// May allocate if the total length of the requested buffers is less than `req_len`.
    /// If there are fewer buffers internally than the length of `bufs`,
    /// this function may leave a contiguous range of references at the end of `bufs` unchanged.
    fn get_write_bufs<'a, 'b: 'a>(
        &'b mut self,
        req_len: usize,
        bufs: &'a mut [IoRepr<UninitSlice<'b>>],
    );
    /// Mark `len` bytes as having been written.
    ///
    /// # Safety
    /// Asserts that at least `len` bytes at the front of the buffer have been initialized
    /// and are therefore safe to create references to.
    unsafe fn supply(&mut self, len: usize);
    /// Returns the suggested length of the `output` parameter of `write_bufs`.
    /// If `0` is returned, there is no remaining capacity.
    ///
    /// Any non-zero values returned by this function are hints and should not be relied upon.
    fn write_bufs_hint(&self) -> usize {
        1
    }
}

// TODO: Function for copying between bufs.
