#![deny(unsafe_op_in_unsafe_fn)]
//! Realtime foundation and non-RT graph compilation; platform transports live outside.
pub mod boundary;
pub mod buffer;
pub mod compiler;
pub mod control;
pub mod meter;
pub mod node;
pub mod processor;
pub mod runtime;
pub mod sample;
