//! I/O abstractions.
//!
//! A collection of traits and implementations thereof for the purposes of
//! abstracting over asynchronous byte streams.
//! These traits distinguish between two types of streams:
//! those with built-in message framing (e.g. WebSocket, ZeroMQ)
//! and those that don't (e.g. TCP, Unix streams).

// These are meant to be able to work in `no_std`,
// so we can't use `std::io::Error` as an error type.
// Besides, `!`/`core::convert::Infallible` is a valid error type for some of these.

use crate::buffer::{BufferReader, BufferWriter};
use core::{
    num::NonZeroUsize,
    pin::Pin,
    task::{Context, Poll},
};

/// Trait for the read halves of streams with built-in message framing.
pub trait FramedRead {
    type Error: 'static;
    /// Reads into the provided buffer `buf`.
    /// Returns the length of the first message available to parse from the buffer.
    ///
    /// This function may make partial updates to the buffer,
    /// or write more than one message to it.
    /// However, it will not return [`Poll::Ready`] until there is a full message available,
    /// and if multiple messages are written,
    fn read_msg(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut BufferWriter,
        max_len: NonZeroUsize,
    ) -> Poll<Result<usize, Self::Error>>;
}
/// Trait for the read halves of streams with no message framing.
pub trait UnframedRead {
    type Error: 'static;
    fn read_some(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut BufferWriter,
        max_len: NonZeroUsize,
    ) -> Poll<Result<(), Self::Error>>;
}

/// Trait for the write halves of streams with built-in message framing.
pub trait FramedWrite {
    type Error: 'static;
    fn write_msg(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut BufferReader,
        msg_len: usize,
    ) -> Poll<Result<(), Self::Error>>;
}

/// Trait for the write halves of streams with no message framing.
pub trait UnframedWrite {
    type Error: 'static;
    fn write_some(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut BufferReader,
    ) -> Poll<Result<(), Self::Error>>;
}

/// Trait for message decoders that operate on [`FramedRead`]s.
pub trait FramedDecoder {
    type Error: 'static;
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
        self: Pin<&mut Self>,
        buf: &'a mut BufferReader,
        msg_len: Option<usize>,
    ) -> Poll<Result<Self::Message<'a>, Self::Error>>;
}

/// Trait for message decoders that operate on [`UnframedRead`]s.
///
/// Implementing this trait asserts that [`FramedDecoder::decode`]
/// does not need to ever be given the message length in order to parse a message.
pub trait UnframedDecoder: FramedDecoder {}

/// Trait for message encoders that operate on [`FramedWrite`]s.
pub trait FramedEncoder {
    /// The type of messages this type can encode.
    type Message<'a>;
    /// Encodes `value` to the provided buffer.
    ///
    /// This function may not immediately write encoded messages to `buf`.
    /// [`MsgEncoder::flush`] should be called to ensure that encoded messages are written.
    fn encode(self: Pin<&mut Self>, value: &Self::Message<'_>, buf: &mut BufferWriter);
    /// Flushes any internal buffering.
    fn flush(self: Pin<&mut Self>, buf: &mut BufferWriter);
}

/// Trait for message encoders that operate on [`UnframedWrite`]s.
///
/// Implementing this trait asserts that messages written by this encoder
/// can be decoded from a plain stream of bytes.
pub trait UnframedEncoder: FramedEncoder {}
