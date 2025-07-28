use crate::buf::{BufRead, BufWrite, LinearBufReader};
use core::task::Poll;

/// Trait for message decoders that operate on [`FramedRead`][crate::io::FramedRead]s.
pub trait FramedDecoder {
    type Error;
    type Message<'a>;
    /// Decodes one message from the buffer, where `msg_len` is equal to the length of the message.
    /// Returns [`Poll::Pending`] if additional data is needed to return a message.
    ///
    /// `msg_len` is the length of the first message in `buf`.
    /// If `None`, the message length is unknown.
    /// If this function is called with `msg_len = None` and further parsing isn't possible
    /// without knowing the message length, `Poll::Pending` shall be returned.
    ///
    /// `msg_len` may be larger than the length of `buf`,
    /// indicating that `buf` contains a partial message.
    /// If `msg_len` is `usize::MAX`, indicates that the remaining message length is
    /// greater than or equal to `usize::MAX`.
    fn decode<'a>(
        &mut self,
        buf: &'a mut LinearBufReader,
        msg_len: Option<usize>,
    ) -> Poll<Result<Self::Message<'a>, Self::Error>>;
}

/// Trait for message decoders that operate on [`UnframedRead`][crate::io::UnframedRead]s.
///
/// Implementing this trait asserts that [`FramedDecoder::decode`]
/// does not need to ever be given the message length in order to parse a message.
pub trait UnframedDecoder: FramedDecoder {}

/// Trait for message encoders that operate on [`FramedWrite`][crate::io::FramedWrite]s.
pub trait FramedEncoder {
    /// The type of messages this type can encode.
    type Message<'a>;
    /// Encodes `value` to the provided buffer.
    ///
    /// This function may not immediately write encoded messages to `buf`.
    /// [`FramedEncoder::flush`] should be called to ensure that encoded messages are written.
    fn encode(&mut self, value: &Self::Message<'_>, buf: &mut dyn BufWrite);
    /// Flushes any internal buffering.
    #[allow(unused)]
    fn flush(&mut self, buf: &mut dyn BufWrite) {}
    /// Emits any data necessary for the end of a closing stream.
    #[allow(unused)]
    fn finish(&mut self, output: &mut dyn BufWrite) {}
}

/// Trait for message encoders that operate on [`UnframedWrite`][crate::io::UnframedWrite]s.
///
/// Implementing this trait asserts that messages written by this encoder
/// can be decoded from a plain stream of bytes.
pub trait UnframedEncoder: FramedEncoder {}

/// Dyn-compatible trait for byte stream decoders.
pub trait ByteDecoder {
    type Error;
    fn decode(&mut self, data: &mut dyn BufRead, buf: &mut dyn BufWrite)
    -> Result<(), Self::Error>;
}

/// Dyn-compatible trait for byte stream encoders.
pub trait ByteEncoder {
    /// Encodes data to the provided buffer.
    ///
    /// This function may not immediately write data to `buf`.
    /// [`ByteEncoder::flush`] should be called to ensure that encoded data is written.
    fn encode(&mut self, data: &mut dyn BufRead, buf: &mut dyn BufWrite);
    /// Flushes any internal buffering.
    #[allow(unused)]
    fn flush(&mut self, output: &mut dyn BufWrite) {}
    /// Emits any data necessary for the end of a closing stream.
    #[allow(unused)]
    fn finish(&mut self, output: &mut dyn BufWrite) {}
}
