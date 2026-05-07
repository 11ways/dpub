//! Tiny shared utilities used by every other crate in the dpub workspace.
//!
//! Currently just XML/XHTML attribute and text escaping. The previous
//! design had three independent copies (in `dpub-core::smil_write`,
//! `epub3_writer::writers`, `dpub_convert`) which is a recipe for bug
//! drift — fix one, forget the others.

pub mod lang;
pub mod xml;
