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

use core::{
    num::NonZeroUsize,
    pin::Pin,
    task::{Context, Poll},
};
use crate::buf::{BufRead, BufWrite};

/// Trait for the read halves of streams with no message framing.
pub trait UnframedRead {
    type Error;
    /// Reads up to `max_len` bytes into the provided buffer `buf`.
    ///
    /// Returns a value that is no less than the length of the next message in the buffer.
    /// A value of `0` may be interpreted as "unknown".
    ///
    /// If this function returns `Poll::Pending`, subsequent calls must use the same value
    /// for `buf` and `max_len`.
    fn read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut dyn BufWrite,
        max_len: NonZeroUsize,
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

/// Adapter to use buffers as unframed I/O.
///
/// [`UnframedRead`] and [`UnframedWrite`] are not blanket-implemented
/// for [`BufRead`] and [`BufWrite`] implementors respectively because
/// the blanket implementations would ignore useful functionality of the underlying types.
/// For instance, the
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
#[repr(transparent)]
pub struct AsUnframed<T>(pub T);

impl<T> AsUnframed<T> {
    /// Coerces a `&mut T` into a `&mut AsUnframed<T>`.
    pub fn from_mut(mut_ref: &mut T) -> &mut Self {
        unsafe { &mut *(mut_ref as *mut T as *mut Self) }
    }
}

impl<T: BufRead> UnframedRead for AsUnframed<T> {
    type Error = core::convert::Infallible;

    fn read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut dyn BufWrite,
        max_len: NonZeroUsize,
    ) -> Poll<Result<usize, Self::Error>> {
        todo!()
    }
}

impl<T: BufWrite> UnframedWrite for AsUnframed<T> {
    type Error = core::convert::Infallible;

    fn write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut dyn BufRead,
        msg_len: usize,
    ) -> Poll<Result<(), Self::Error>> {
        todo!()
    }

    fn flush(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut dyn BufRead,
    ) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(())
    }

}
