//! [`Buffer`] and functions for working with it.
//!
//! `Buffer` is a linear resizeable byte buffer.
//! Unlike a ring buffer, this buffer does not wrap around at the end.
//! This can result in additional copies and wasted space,
//! however it guarantees that the data is always contiguous.

use core::ptr::NonNull;

use alloc::alloc::Layout;

#[cfg(feature = "std")]
fn copy_partial(output: &mut [u8], input: &[u8]) -> usize {
    let len = core::cmp::min(input.len(), output.len());
    let output = &mut output[..len];
    let input = &input[..len];
    output.copy_from_slice(input);
    len
}

/// Linear resizeable byte buffer.
///
/// Refer to the [module-level documentation][self] for more info.
#[repr(C)]
pub struct Buffer {
    bytes: NonNull<u8>,
    capacity: usize,
    /// Right index: the start of the part of the buffer for input.
    input_idx: usize,
    /// Left index: the start of the part of the buffer for output.
    output_idx: usize,
}

impl Drop for Buffer {
    fn drop(&mut self) {
        if self.capacity > 0 {
            unsafe {
                let layout = Layout::array::<u8>(self.capacity).unwrap();
                alloc::alloc::dealloc(self.bytes.as_ptr(), layout);
            }
        }
    }
}

impl Clone for Buffer {
    fn clone(&self) -> Self {
        let mut b = Self::with_capacity(self.capacity_min());
        let src = self.output_slice();
        let dest = b.input_slice_mut(src.len());
        dest.copy_from_slice(src);
        b
    }
}

impl Default for Buffer {
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

impl Buffer {
    pub const fn new() -> Self {
        Buffer { bytes: NonNull::dangling(), capacity: 0, input_idx: 0, output_idx: 0 }
    }
    /// Allocates a `Buffer` with a starting capacity that is at least `size` bytes.
    ///
    /// The allocated capacity may be less than requested upon allocation failure
    /// or if more bytes are requested than `isize::MAX`.
    /// Always verify the size of the input buffer before writing to it.
    pub fn with_capacity(capacity: usize) -> Buffer {
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
    /// Reborrows `self` as a [`BufferReader`], giving access to read operations.
    pub fn reader(&mut self) -> &mut BufferReader {
        unsafe { &mut *(self as *mut Self as *mut BufferReader) }
    }
    /// Reborrows `self` as a [`BufferWriter`], giving access to write operations.
    pub fn writer(&mut self) -> &mut BufferWriter {
        unsafe { &mut *(self as *mut Self as *mut BufferWriter) }
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
        use alloc::alloc::{alloc_zeroed, dealloc, realloc};
        len = core::cmp::min(len, isize::MAX as usize);
        if len == self.capacity {
            true
        } else if self.capacity > 0 {
            // Unwrap: something has gone horribly wrong if this isn't a valid layout.
            let layout_old = Layout::array::<u8>(self.capacity).unwrap();
            if len > 0 {
                let bytes = unsafe { realloc(self.bytes.as_ptr(), layout_old, len) };
                let Some(bytes) = NonNull::new(bytes) else {
                    return false;
                };
                self.bytes = bytes;
                if len > self.capacity {
                    // Zero the new bytes, since realloc doesn't guarantee zero-init.
                    // Annoying that realloc_zeroed doesn't exist, since depending on the allocator,
                    // zeroing the memory can sometimes be redundant.
                    use core::ptr::write_bytes;
                    let new_bytes = len - self.capacity;
                    unsafe { write_bytes(self.bytes.as_ptr().add(self.capacity), 0, new_bytes) };
                }
            } else {
                unsafe { dealloc(self.bytes.as_ptr(), layout_old) };
                self.bytes = NonNull::dangling();
            }
            self.capacity = len;
            true
        } else {
            // Capacity is 0 and len != capacity (so len > 0).
            let Ok(layout) = Layout::array::<u8>(len) else {
                return false;
            };
            let Some(bytes) = NonNull::new(unsafe { alloc_zeroed(layout) }) else {
                return false;
            };
            self.bytes = bytes;
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
    fn input_slice_mut(&mut self, min: usize) -> &mut [u8] {
        self.reserve(min);
        let range = self.input_idx..;
        &mut self.full_slice_mut()[range]
    }
    fn output_slice(&self) -> &[u8] {
        &self.full_slice()[self.output_idx..self.input_idx]
    }
    fn output_slice_mut(&mut self) -> &mut [u8] {
        // Conniptions, borrowck.
        let a = self.output_idx;
        let b = self.input_idx;
        &mut self.full_slice_mut()[a..b]
    }
    #[inline]
    fn consume(&mut self, count: usize) {
        assert!(count <= self.len());
        self.output_idx += count;
        if self.is_empty() {
            self.output_idx = 0;
            self.input_idx = 0;
        }
    }
    #[inline]
    fn advance(&mut self, count: usize) {
        assert!(count <= self.capacity_in());
        self.input_idx += count;
    }
    fn full_slice(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.bytes.as_ptr(), self.capacity) }
    }
    fn full_slice_mut(&mut self) -> &mut [u8] {
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

impl core::ops::Deref for Buffer {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.output_slice()
    }
}

impl core::ops::DerefMut for Buffer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.output_slice_mut()
    }
}

#[cfg(feature = "std")]
impl std::io::Read for Buffer {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let len = copy_partial(buf, self.output_slice());
        self.consume(len);
        Ok(len)
    }
    // TODO: Default impls could be better.
}

#[cfg(feature = "std")]
impl std::io::BufRead for Buffer {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        Ok(self.output_slice())
    }
    fn consume(&mut self, amt: usize) {
        self.consume(amt);
    }
}

#[cfg(feature = "std")]
impl std::io::Write for Buffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let len = copy_partial(self.input_slice_mut(buf.len()), buf);
        self.advance(len);
        Ok(len)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Output interface to [`Buffer`].
///
/// `Buffer`s can be used as this type with [`Buffer::reader`].
#[repr(transparent)]
pub struct BufferReader(Buffer);

impl BufferReader {
    /// Returns a shared reference to a slice for reading out of.
    ///
    /// If a read opertion conceptually consumes bytes
    /// (e.g. due to message parsing), [`BufferReader::consume`]
    /// should be called afterward.
    #[inline(always)]
    pub fn slice(&self) -> &[u8] {
        self.0.output_slice()
    }
    /// Returns a mutable reference to a slice for reading out of.
    ///
    /// If a read opertion conceptually consumes bytes
    /// (e.g. due to message parsing), [`BufferReader::consume`]
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
        self.0.consume(count);
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
        let slice = unsafe { core::slice::from_raw_parts(self.0.bytes.as_ptr(), self.0.input_idx) };
        match f(&slice[self.0.output_idx..]) {
            Ok((retval, consume)) => {
                self.consume(consume);
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

impl core::ops::Deref for BufferReader {
    type Target = Buffer;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[cfg(feature = "std")]
impl std::io::Read for BufferReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
    // TODO: Default impls could be better.
}

#[cfg(feature = "std")]
impl std::io::BufRead for BufferReader {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        self.0.fill_buf()
    }
    fn consume(&mut self, amt: usize) {
        self.0.consume(amt)
    }
}

/// Input interface to [`Buffer`].
///
/// `Buffer`s can be used as this type with [`Buffer::writer`].
#[repr(transparent)]
pub struct BufferWriter(Buffer);

impl BufferWriter {
    /// Returns a mutable reference to a slice for writing to.
    /// The slice will be at least `min` bytes long,
    /// except in cases of allocation failure or more than `isize::MAX`
    /// bytes of capacity would be required.
    /// Always check the size of the input buffer before unsafely writing to it.
    ///
    /// After writing, [`BufferWriter::advance`] should be called
    /// with how many bites have been written.
    #[inline(always)]
    pub fn slice_mut(&mut self, min: usize) -> &mut [u8] {
        self.0.input_slice_mut(min)
    }
    /// Marks `count` bytes of the front of the input slice as having been read into,
    /// making them available at the end of the output slice.
    ///
    /// # Panics
    /// Panics if `count` is greater than the number of bytes available for input,
    /// as this likely indicates a logic bug in the caller.
    #[inline(always)]
    pub fn advance(&mut self, count: usize) {
        self.0.advance(count);
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
    #[cfg(feature = "std")]
    /// Reads data once from a provided [`std::io::Read`].
    pub fn read_from<T: std::io::Read>(
        &mut self,
        min: usize,
        read: &mut T,
    ) -> std::io::Result<usize> {
        let count = read.read(self.0.input_slice_mut(min))?;
        self.advance(count);
        Ok(count)
    }
}

impl core::ops::Deref for BufferWriter {
    type Target = Buffer;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[cfg(feature = "std")]
impl std::io::Write for BufferWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::Buffer;

    #[test]
    fn zero_capacity() {
        let mut buffer = Buffer::with_capacity(0);
        assert_eq!(buffer.capacity_in(), 0);
        buffer.input_slice_mut(64);
        assert!(buffer.capacity_in() >= 64);
    }
    #[cfg(feature = "std")]
    fn io_test(in_rate: usize, out_rate: usize) {
        use std::io::Cursor;
        let byte_count = 5000usize;
        let bytes: Vec<u8> =
            core::iter::successors(Some(1u8), |byte| Some(byte.overflowing_add(3u8).0))
                .take(byte_count)
                .collect();
        let mut buffer = Buffer::with_capacity(1024);
        let mut read = Cursor::new(bytes);
        let output = vec![0u8; byte_count];
        let mut write = Cursor::new(output);
        let mut should_loop = true;
        while should_loop {
            use std::io::{Read, Write};
            should_loop = false;
            // Input.
            let mut slice = buffer.input_slice_mut(in_rate);
            let len = core::cmp::min(slice.len(), in_rate);
            slice = &mut slice[..len];
            let byte_count = read.read(slice).unwrap();
            buffer.advance(byte_count);
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
    fn single_input() {
        io_test(6000, 1000);
    }
}
