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

use crate::buf::{BufRead, BufWrite};
use core::{
    pin::Pin,
    task::{Context, Poll},
};

/// Trait for the read halves of streams with no message framing.
pub trait UnframedRead {
    type Error;
    /// Reads up to `max_len` bytes into the provided buffer `buf`.
    ///
    /// Returns a value that is no less than the length of the next message in the buffer.
    /// A value of `0` may be interpreted as "unknown".
    /// The lower bound of `len` indicates how many bytes are wanted by the caller,
    /// while the upper bound indicates the maxmium amount that may be readed.
    ///
    /// If this function returns `Poll::Pending`, subsequent calls must use the same value
    /// for `buf` and `len`.
    fn read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut dyn BufWrite,
        len: core::ops::Range<usize>,
    ) -> Poll<Result<usize, Self::Error>>;
}

/// Trait for the read halves of streams with built-in message framing.
///
/// This trait imposes additional conditions on [`UnframedRead::read`].
/// Upon returning `Poll::Ready(Ok(len))`, precisely one message that is exactly `len` bytes
/// long shall have been written to the provided buffer.
pub trait FramedRead: UnframedRead {}

/// Trait for the write halves of streams with no message framing.
pub trait UnframedWrite {
    type Error;
    /// Writes bytes from the provided buffer `buf`.
    ///
    /// msg_len is a hint indicating how many bytes are in the next message.
    /// This function should attempt to write no fewer than that many bytes.
    ///
    /// If this function returns `Poll::Pending`, subsequent calls must use the same value
    /// for `buf` and `msg_len`.
    fn write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut dyn BufRead,
        msg_len: usize,
    ) -> Poll<Result<(), Self::Error>>;

    /// Flushes any internal buffering.
    fn flush(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut dyn BufRead,
    ) -> Poll<Result<(), Self::Error>>;
}

/// Trait for the write halves of streams with built-in message framing.
///
/// This trait imposes additional conditions on [`UnframedWrite::write`].
/// Upon returning `Poll::Ready(Ok(()))`, exactly `msg_len` bytes must have been written.
pub trait FramedWrite: UnframedWrite {}
