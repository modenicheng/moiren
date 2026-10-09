//! Runtime integration tests share one allocator so allocation accounting stays
//! thread-local while graph, boundary IO, and automation coverage can evolve apart.
mod allocation;
mod automation;
mod graph;
mod io;
mod support;
mod swap;
