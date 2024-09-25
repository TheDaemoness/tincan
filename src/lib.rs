#![cfg_attr(not(feature = "std"), no_std)]
#![doc = include_str!("../doc/lib.md")]
#![cfg_attr(doc_unstable, feature(doc_auto_cfg))]

extern crate alloc;

pub mod buffer;
