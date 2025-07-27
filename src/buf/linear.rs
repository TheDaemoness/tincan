use alloc::alloc::Layout;
use core::{mem::MaybeUninit, ptr::NonNull};

use super::{IoRepr, UninitSlice};

/// Linear resizeable byte buffer.
///
/// Unlike a ring buffer, this buffer does not wrap around at the end.
/// This can result in additional copies and wasted space,
/// however it guarantees that the data is always contiguous.
#[repr(C)]
pub struct LinearBuf {
    bytes: NonNull<MaybeUninit<u8>>,
    capacity: usize,
    /// Right index: the start of the part of the buffer for input.
    input_idx: usize,
    /// Left index: the start of the part of the buffer for output.
    output_idx: usize,
}

impl Drop for LinearBuf {
    fn drop(&mut self) {
        if self.capacity > 0 {
            unsafe {
                let layout = Layout::array::<u8>(self.capacity).unwrap();
                alloc::alloc::dealloc(self.bytes.as_ptr().cast::<u8>(), layout);
            }
        }
    }
}

impl Clone for LinearBuf {
    fn clone(&self) -> Self {
        let len = self.len();
        let mut b = Self::with_capacity(self.capacity_min());
        let src = self.output_slice().as_ptr().cast::<MaybeUninit<u8>>();
        let dest = b.input_slice_mut(len).as_mut_ptr();
        unsafe { core::ptr::copy_nonoverlapping(src, dest, len) }
        b
    }
}

impl Default for LinearBuf {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub struct AllocFailure;

impl core::fmt::Display for AllocFailure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "memory allocation failure")
    }
}

impl LinearBuf {
    pub const fn new() -> Self {
        LinearBuf { bytes: NonNull::dangling(), capacity: 0, input_idx: 0, output_idx: 0 }
    }
    /// Allocates a `LinearBuf` with a starting capacity that is at least `size` bytes.
    ///
    /// The allocated capacity may be less than requested upon allocation failure
    /// or if more bytes are requested than `isize::MAX`.
    /// Always verify the size of the input buffer before writing to it.
    pub fn with_capacity(capacity: usize) -> LinearBuf {
        let mut this = Self::new();
        this.realloc(capacity);
        this
    }
    /// Returns true if there is no output available.
    pub fn is_empty(&self) -> bool {
        self.input_idx == self.output_idx
    }
    /// Returns how many bytes of memory are allocated by `self`.
    ///
    /// This value may be more than the sum of available input and output bytes.
    pub fn capacity(&self) -> usize {
        self.capacity
    }
    /// Returns how many bytes of space are available to read into.
    pub fn capacity_in(&self) -> usize {
        self.capacity - self.input_idx
    }
    /// Returns how many bytes are available to read out of.
    pub fn len(&self) -> usize {
        self.input_idx - self.output_idx
    }
    /// Reborrows `self` as a [`LinearBufReader`], giving access to read operations.
    pub fn reader(&mut self) -> &mut LinearBufReader {
        unsafe { &mut *(self as *mut Self as *mut LinearBufReader) }
    }
    /// Reborrows `self` as a [`LinearBufWriter`], giving access to write operations.
    pub fn writer(&mut self) -> &mut LinearBufWriter {
        unsafe { &mut *(self as *mut Self as *mut LinearBufWriter) }
    }
    /// Shrinks `self`'s capacity to the size of the contained data or `min`, whichever is greater.
    ///
    /// The allocated capacity may be different than requested upon allocation failure
    /// or if more bytes are requested than `isize::MAX`.
    /// Always verify the size of the input buffer before writing to it.
    pub fn shrink_to_fit(&mut self, min: usize) {
        self.shift_to_start();
        // Special case: input_idx is equal to len() following shift_to_start.
        let new_size = core::cmp::max(min, self.input_idx);
        self.realloc(new_size);
    }
    fn capacity_min(&self) -> usize {
        self.capacity - self.output_idx
    }
    #[inline]
    /// # Safety
    /// Assumes that len will not be less than the right index of the buffer.
    fn realloc(&mut self, mut len: usize) -> bool {
        use alloc::alloc::{alloc, dealloc, realloc};
        len = core::cmp::min(len, isize::MAX as usize);
        if len == self.capacity {
            true
        } else if self.capacity > 0 {
            // Unwrap: something has gone horribly wrong if this isn't a valid layout.
            let layout_old = Layout::array::<u8>(self.capacity).unwrap();
            if len > 0 {
                let bytes = unsafe { realloc(self.bytes.as_ptr().cast::<u8>(), layout_old, len) };
                let Some(bytes) = NonNull::new(bytes) else {
                    return false;
                };
                self.bytes = bytes.cast::<MaybeUninit<u8>>();
            } else {
                unsafe { dealloc(self.bytes.as_ptr().cast::<u8>(), layout_old) };
                self.bytes = NonNull::dangling();
            }
            self.capacity = len;
            true
        } else {
            // Capacity is 0 and len != capacity (so len > 0).
            let Ok(layout) = Layout::array::<u8>(len) else {
                return false;
            };
            let Some(bytes) = NonNull::new(unsafe { alloc(layout) }) else {
                return false;
            };
            self.bytes = bytes.cast::<MaybeUninit<u8>>();
            self.capacity = len;
            true
        }
    }
    fn reserve(&mut self, bytes: usize) -> bool {
        if self.capacity_in() < bytes && self.shift_to_start() < bytes {
            let new_capacity =
                core::cmp::min(self.capacity + self.input_idx + bytes, isize::MAX as usize);
            self.realloc(new_capacity)
        } else {
            true
        }
    }
    fn input_slice_mut(&mut self, min: usize) -> &mut [MaybeUninit<u8>] {
        self.reserve(min);
        let range = self.input_idx..;
        &mut self.full_slice_mut()[range]
    }
    fn output_slice(&self) -> &[u8] {
        let len = self.input_idx - self.output_idx;
        // Safety: output_idx <= capacity_in <= isize::MAX
        let ptr = unsafe { self.full_slice().as_ptr().byte_add(self.output_idx) };
        // Safety: The output slice is initialized.
        unsafe { core::slice::from_raw_parts(ptr.cast::<u8>(), len) }
    }
    fn output_slice_mut(&mut self) -> &mut [u8] {
        // Conniptions, borrowck.
        let output_idx = self.output_idx;
        let len = self.input_idx - output_idx;
        // Safety: output_idx <= capacity_in <= isize::MAX
        let ptr = unsafe { self.full_slice_mut().as_mut_ptr().byte_add(output_idx) };
        // Safety: The output slice is initialized.
        unsafe { core::slice::from_raw_parts_mut(ptr.cast::<u8>(), len) }
    }
    #[inline]
    unsafe fn consume_unchecked(&mut self, count: usize) {
        assert!(count <= self.len());
        self.output_idx += count;
        if self.is_empty() {
            self.output_idx = 0;
            self.input_idx = 0;
        }
    }
    #[inline(always)]
    fn consume(&mut self, count: usize) {
        assert!(count <= self.len());
        unsafe {
            self.consume_unchecked(count);
        }
    }
    #[inline(always)]
    unsafe fn supply_unchecked(&mut self, count: usize) {
        self.input_idx += count;
    }
    fn full_slice(&self) -> &[MaybeUninit<u8>] {
        unsafe { core::slice::from_raw_parts(self.bytes.as_ptr(), self.capacity) }
    }
    fn full_slice_mut(&mut self) -> &mut [MaybeUninit<u8>] {
        unsafe { core::slice::from_raw_parts_mut(self.bytes.as_mut(), self.capacity) }
    }
    /// Move all elements to the start in order to maximize input space.
    fn shift_to_start(&mut self) -> usize {
        if self.output_idx == 0 {
            return self.capacity_in();
        }
        let range = self.output_idx..self.input_idx;
        let slice = self.full_slice_mut();
        slice.copy_within(range, 0);
        let retval = self.capacity + self.output_idx - self.input_idx;
        self.input_idx -= self.output_idx;
        self.output_idx = 0;
        retval
    }
}

impl core::ops::Deref for LinearBuf {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.output_slice()
    }
}

impl core::ops::DerefMut for LinearBuf {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.output_slice_mut()
    }
}

#[cfg(feature = "std")]
impl std::io::Read for LinearBuf {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let len = UninitSlice::new(buf).write_from(self.output_slice()).len();
        unsafe {
            self.consume_unchecked(len);
        }
        Ok(len)
    }
    // TODO: Default impls could be better.
}

#[cfg(feature = "std")]
impl std::io::BufRead for LinearBuf {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        Ok(self.output_slice())
    }
    fn consume(&mut self, amt: usize) {
        self.consume(amt);
    }
}

#[cfg(feature = "std")]
impl std::io::Write for LinearBuf {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let len = UninitSlice::uninit(self.input_slice_mut(buf.len())).write_from(buf).len();
        unsafe {
            self.supply_unchecked(len);
        }
        Ok(len)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Output interface to [`LinearBuf`].
///
/// `LinearBuf`s can be used as this type with [`LinearBuf::reader`].
#[repr(transparent)]
pub struct LinearBufReader(LinearBuf);

impl LinearBufReader {
    /// Returns a shared reference to a slice for reading out of.
    ///
    /// If a read opertion conceptually consumes bytes
    /// (e.g. due to message parsing), [`LinearBufReader::consume`]
    /// should be called afterward.
    #[inline(always)]
    pub fn slice(&self) -> &[u8] {
        self.0.output_slice()
    }
    /// Returns a mutable reference to a slice for reading out of.
    ///
    /// If a read opertion conceptually consumes bytes
    /// (e.g. due to message parsing), [`LinearBufReader::consume`]
    /// should be called afterward.
    #[inline(always)]
    pub fn slice_mut(&mut self) -> &mut [u8] {
        self.0.output_slice_mut()
    }
    /// Marks `count` bytes of the front of the output slice as having been read out of.
    ///
    /// # Panics
    /// Panics if `count` is greater than the number of bytes available for output,
    /// as this likely indicates a logic bug in the caller.
    #[inline(always)]
    pub fn consume(&mut self, count: usize) {
        self.0.consume(count)
    }
    /// Marks the entire output slice as having been read out of.
    #[inline(always)]
    pub fn consume_all(&mut self) {
        self.0.output_idx = 0;
        self.0.input_idx = 0;
    }
    /// Parses a value out of the output slice.
    ///
    /// Accepts a fallible closure that is expected to return both the parsed value and how many
    /// bytes were consumed during parsing.
    pub fn parse<'a, O, F, E>(&'a mut self, f: F) -> Result<O, E>
    where
        O: 'a,
        F: FnOnce(&'a [u8]) -> Result<(O, usize), E>,
    {
        // Get refs to each needed member so that we don't have to worry about aliasing.
        let LinearBuf { bytes, input_idx, output_idx, .. } = &mut self.0;
        // Safety: Bytes between output_idx and input_idx are guaranteed to be filled.
        // This duplication is necessary because the returned value can borrow data from the buffer,
        // but we still need to be able to increment output_idx.
        let slice = unsafe { core::slice::from_raw_parts(bytes.as_ptr() as *const u8, *input_idx) };
        match f(&slice[*output_idx..]) {
            Ok((retval, consume)) => {
                *output_idx += consume;
                if *output_idx == *input_idx {
                    *output_idx = 0;
                    *input_idx = 0;
                } else if *output_idx > *input_idx {
                    panic!(
                        "Parser consumed {} more byte(s) than were available to read",
                        *output_idx - *input_idx
                    )
                }
                Ok(retval)
            }
            Err(e) => Err(e),
        }
    }
    #[cfg(feature = "std")]
    /// Writes data to a provided [`std::io::Write`].
    #[inline(always)]
    pub fn write_to<T: std::io::Write>(&mut self, write: &mut T) -> std::io::Result<usize> {
        let count = write.write(self.0.output_slice())?;
        self.0.consume(count);
        Ok(count)
    }
}

impl core::ops::Deref for LinearBufReader {
    type Target = LinearBuf;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl super::BufRead for LinearBufReader {
    fn get_read_bufs<'a, 'b: 'a>(&'b self, bufs: &'a mut [super::IoRepr<&'b [u8]>]) {
        if let Some(buf) = bufs.first_mut() {
            *buf = IoRepr::new(self.slice());
        }
    }

    fn consume(&mut self, len: usize) {
        self.consume(len);
    }
}

#[cfg(feature = "std")]
impl std::io::Read for LinearBufReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
    // TODO: Default impls could be better.
}

#[cfg(feature = "std")]
impl std::io::BufRead for LinearBufReader {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        self.0.fill_buf()
    }
    fn consume(&mut self, amt: usize) {
        self.0.consume(amt);
    }
}

/// Input interface to [`LinearBuf`].
///
/// `LinearBuf`s can be used as this type with [`LinearBuf::writer`].
#[repr(transparent)]
pub struct LinearBufWriter(LinearBuf);

impl LinearBufWriter {
    /// Returns a mutable reference to a slice for writing to.
    /// The slice will be at least `min` bytes long,
    /// except in cases of allocation failure or more than `isize::MAX`
    /// bytes of capacity would be required.
    /// Always check the size of the input buffer before unsafely writing to it.
    ///
    /// After writing, [`LinearBufWriter::supply`] should be called
    /// with how many bites have been written.
    #[inline(always)]
    pub fn slice_mut(&mut self, min: usize) -> &mut [MaybeUninit<u8>] {
        self.0.input_slice_mut(min)
    }
    /// Marks `count` bytes of the front of the input slice as having been read into,
    /// making them available at the end of the output slice.
    ///
    /// # Safety
    /// `count` must be less than the number of bytes available for input.
    /// `count` must be greater than or equal to the number of bytes initialized,
    /// as this method asserts that those bytes are initialized and therefore safe
    /// to make references to.
    #[inline(always)]
    pub unsafe fn supply(&mut self, count: usize) {
        unsafe {
            self.0.supply_unchecked(count);
        }
    }
    /// Ensures that at least `bytes` bytes are available for input to the buffer.
    ///
    /// # Panics
    /// Panics if the total size of the buffer would exceed `isize::MAX`
    /// as a result of this operation.
    #[inline(always)]
    pub fn reserve(&mut self, bytes: usize) {
        self.0.reserve(bytes);
    }

    // TODO: read_from.
    // In order to safely pass an input slice to an std::io::Read,
    // the slice has to be zeroed.
}

impl core::ops::Deref for LinearBufWriter {
    type Target = LinearBuf;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl core::ops::DerefMut for LinearBufWriter {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl super::BufWrite for LinearBufWriter {
    fn get_write_bufs<'a, 'b: 'a>(
        &'b mut self,
        req_len: usize,
        bufs: &'a mut [IoRepr<UninitSlice<'b>>],
    ) {
        if let Some(buf) = bufs.first_mut() {
            let slice = self.slice_mut(req_len);
            *buf = IoRepr::new(UninitSlice::uninit(slice));
        }
    }

    unsafe fn supply(&mut self, len: usize) {
        unsafe {
            self.supply(len);
        }
    }
}

#[cfg(feature = "std")]
impl std::io::Write for LinearBufWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::LinearBuf;

    #[test]
    fn zero_capacity() {
        let mut buffer = LinearBuf::with_capacity(0);
        assert_eq!(buffer.capacity_in(), 0);
        buffer.input_slice_mut(64);
        assert!(buffer.capacity_in() >= 64);
    }
    #[cfg(feature = "std")]
    fn io_test(in_rate: usize, out_rate: usize) {
        use crate::buf::UninitSlice;
        use std::io::Cursor;
        let byte_count = 5000usize;
        let bytes: Vec<u8> =
            core::iter::successors(Some(1u8), |byte| Some(byte.overflowing_add(3u8).0))
                .take(byte_count)
                .collect();
        let mut buffer = LinearBuf::with_capacity(1024);
        let mut read = Cursor::new(bytes);
        let output = vec![0u8; byte_count];
        let mut write = Cursor::new(output);
        let mut should_loop = true;
        while should_loop {
            use std::io::{Read, Write};
            should_loop = false;
            // Input.
            let mut slice = UninitSlice::uninit(buffer.input_slice_mut(in_rate)).into_zeroed();
            let len = core::cmp::min(slice.len(), in_rate);
            slice = &mut slice[..len];
            let byte_count = read.read(slice).unwrap();
            buffer.supply(byte_count);
            should_loop |= byte_count != 0;
            // Output.
            let mut slice = buffer.output_slice();
            let len = core::cmp::min(slice.len(), out_rate);
            slice = &slice[..len];
            let byte_count = write.write(slice).unwrap();
            buffer.consume(byte_count);
            should_loop |= byte_count != 0;
        }
        assert_eq!(read.into_inner(), write.into_inner());
    }
    #[cfg(feature = "std")]
    #[test]
    fn equal_rates() {
        io_test(300, 300);
    }
    #[cfg(feature = "std")]
    #[test]
    fn slow_input() {
        io_test(300, 500);
    }
    #[cfg(feature = "std")]
    #[test]
    fn slow_output() {
        io_test(500, 300);
    }
    #[cfg(feature = "std")]
    #[test]
    fn very_slow_output() {
        io_test(500, 30);
    }
    #[cfg(feature = "std")]
    #[test]
    fn full_ops() {
        io_test(5000, 5000);
    }
    #[cfg(feature = "std")]
    #[test]
    fn single_resizing_input() {
        io_test(6000, 1000);
    }
}
