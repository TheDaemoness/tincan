use alloc::alloc::Layout;

fn copy_partial(output: &mut [u8], input: &[u8]) -> usize {
    let len = core::cmp::min(input.len(), output.len());
    let output = &mut output[..len];
    let input = &input[..len];
    output.copy_from_slice(input);
    len
}

/// Linear resizeable byte buffer.
///
/// Used for zero-copy parsing of messages out of single slices.
#[repr(C)]
pub struct Buffer {
    bytes: core::ptr::NonNull<u8>,
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
        let dest = b.input_slice(src.len());
        dest.copy_from_slice(src);
        b
    }
}

impl Default for Buffer {
    fn default() -> Self {
        Self::new()
    }
}

impl Buffer {
    pub const fn new() -> Self {
        Buffer { bytes: core::ptr::NonNull::dangling(), capacity: 0, input_idx: 0, output_idx: 0 }
    }
    /// Allocates a `Buffer` with a starting capacity that is at least `size` bytes.
    ///
    /// # Panics
    /// Calls [`alloc::alloc::handle_alloc_error`] on allocation failure.
    /// Panics if more bytes are requested than `isize::MAX`.
    pub fn with_capacity(capacity: usize) -> Buffer {
        let layout = Layout::array::<u8>(capacity).unwrap();
        if layout.size() > 0 {
            let bytes = unsafe {
                use alloc::alloc::alloc_zeroed;
                let Some(bytes) = core::ptr::NonNull::new(alloc_zeroed(layout)) else {
                    alloc::alloc::handle_alloc_error(layout);
                };
                bytes
            };
            Buffer { bytes, capacity, input_idx: 0, output_idx: 0 }
        } else {
            Self::new()
        }
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
            return self.input_available();
        }
        let range = self.output_idx..self.input_idx;
        let slice = self.full_slice_mut();
        slice.copy_within(range, 0);
        let retval = self.capacity + self.output_idx - self.input_idx;
        self.input_idx -= self.output_idx;
        self.output_idx = 0;
        retval
    }
    /// Returns true if there is no output available.
    pub fn is_empty(&self) -> bool {
        self.input_idx == self.output_idx
    }
    /// Returns how many bytes of space are available to read into.
    pub fn input_available(&self) -> usize {
        self.capacity - self.input_idx
    }
    /// Returns how many bytes of space are available to read out of.
    pub fn output_available(&self) -> usize {
        self.input_idx - self.output_idx
    }
    fn capacity_min(&self) -> usize {
        self.capacity - self.output_idx
    }
    fn reserve(&mut self, bytes: usize) {
        if self.capacity == 0 {
            *self = Self::with_capacity(bytes);
        } else if self.input_available() < bytes && self.shift_to_start() < bytes {
            use alloc::alloc::realloc;
            let new_capacity = self.capacity + self.input_idx + bytes;
            let layout = Layout::array::<u8>(self.capacity).unwrap();
            unsafe {
                let bytes = realloc(self.bytes.as_mut(), layout, new_capacity);
                if let Some(bytes) = core::ptr::NonNull::new(bytes) {
                    // TODO: Extra bytes should really be zeroed.
                    self.capacity = new_capacity;
                    self.bytes = bytes;
                }
            }
        }
    }
    /// Returns a reference to a slice for writing to.
    /// The slice will be at least `min` bytes long unless an allocation failure occurs.
    ///
    /// After writing, [`Buffer::advance`] should be called
    /// with how many bites have been written.
    ///
    /// # Panics
    /// Panics if more bytes are requested than `isize::MAX`.
    pub fn input_slice(&mut self, min: usize) -> &mut [u8] {
        self.reserve(min);
        let range = self.input_idx..;
        &mut self.full_slice_mut()[range]
    }
    /// Returns a reference to a slice for reading out of.
    ///
    /// If a read opertion conceptually consumes bytes
    /// (e.g. due to message parsing), [`Buffer::consume`]
    /// should be called afterward.
    pub fn output_slice(&self) -> &[u8] {
        &self.full_slice()[self.output_idx..self.input_idx]
    }
    /// Marks `count` bytes of the front of the input slice as having been read into,
    /// making them available at the end of the output slice.
    ///
    /// Saturates if count is greater than the number of bytes available for input.
    pub fn advance(&mut self, mut count: usize) {
        count = core::cmp::min(count, self.input_available());
        self.input_idx += count;
    }
    /// Marks `count` bytes of the front of the output slice as having been read out of.
    ///
    /// Saturates if count is greater than the number of bytes available for output.
    pub fn consume(&mut self, mut count: usize) {
        count = core::cmp::min(count, self.output_available());
        self.output_idx += count;
        if self.is_empty() {
            self.output_idx = 0;
            self.input_idx = 0;
        }
    }
    /// Marks the entire output slice as having been read out of.
    pub fn consume_all(&mut self) {
        self.output_idx = 0;
        self.input_idx = 0;
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
        let slice = unsafe { core::slice::from_raw_parts(self.bytes.as_ptr(), self.input_idx) };
        match f(&slice[self.output_idx..]) {
            Ok((retval, consume)) => {
                self.consume(consume);
                Ok(retval)
            }
            Err(e) => Err(e),
        }
    }
    #[cfg(feature = "std")]
    /// Reads data once from a provided [`Read`].
    pub fn read_from<T: std::io::Read>(
        &mut self,
        min: usize,
        read: &mut T,
    ) -> std::io::Result<usize> {
        let count = read.read(self.input_slice(min))?;
        self.advance(count);
        Ok(count)
    }
    #[cfg(feature = "std")]
    /// Writes data once to a provided [`Write`].
    pub fn write_to<T: std::io::Write>(&mut self, write: &mut T) -> std::io::Result<usize> {
        let count = write.write(self.output_slice())?;
        self.consume(count);
        Ok(count)
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
        let len = copy_partial(self.input_slice(buf.len()), buf);
        self.advance(len);
        Ok(len)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Buffer;

    #[test]
    fn zero_capacity() {
        let mut buffer = Buffer::with_capacity(0);
        assert_eq!(buffer.input_available(), 0);
        buffer.input_slice(64);
        assert!(buffer.input_available() >= 64);
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
            let mut slice = buffer.input_slice(in_rate);
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
