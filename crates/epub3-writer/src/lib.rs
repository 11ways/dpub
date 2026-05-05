//! Builder and serialiser for accessible EPUB 3 publications, with first-class
//! support for **Media Overlays** (synchronised text-and-audio).
//!
//! Three layers, top-down:
//!
//! 1. [`Publication`] is a small typed model of an EPUB 3 package.
//! 2. The free functions in [`writers`] turn parts of a [`Publication`] into
//!    UTF-8 XML strings (container, OPF, nav, content XHTML, SMIL Media
//!    Overlays).
//! 3. [`Publication::write_zip`] assembles those parts into an `.epub`
//!    archive with the mandatory ordering (`mimetype` first, uncompressed)
//!    and pulls audio bytes from disk on demand.

mod error;
mod model;
mod writers;
mod zip_assembly;

pub use error::{Error, Result};
pub use model::{
    AccessMode, AudioFile, ContentDocument, CoverImage, MediaOverlay, Nav, NavListItem,
    OverlayItem, OverlayPar, OverlaySeq, PackageMetadata, Publication, SectionPart,
};
pub use writers::{
    write_container_xml, write_content_xhtml, write_nav_xhtml, write_overlay_smil,
    write_package_opf,
};
