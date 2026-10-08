#![deny(unsafe_op_in_unsafe_fn)]
//! Realtime foundation; platform transports and graph compilation live outside.
pub mod boundary;
pub mod buffer;
pub mod control;
pub mod meter;
pub mod node;
pub mod processor;
pub mod runtime;
pub mod sample;
