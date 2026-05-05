//! ZIP archive assembly for EPUB 3 publications.
//!
//! The OCF (EPUB Open Container Format) specification mandates that:
//!
//! - The first file in the archive **must** be `mimetype`.
//! - `mimetype` **must** be stored uncompressed (`Stored`, not `Deflated`).
//! - `mimetype` **must not** have an extra field.
//! - The contents are exactly the ASCII bytes `application/epub+zip`,
//!   without a trailing newline.
//!
//! Other entries should be deflate-compressed (smaller archive) — except
//! audio files, which are already lossily compressed and gain nothing from
//! a second deflate pass.

use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use zip::CompressionMethod;
use zip::write::{SimpleFileOptions, ZipWriter};

use crate::error::{Error, Result};
use crate::model::Publication;
use crate::writers::{
    write_container_xml, write_content_xhtml, write_nav_xhtml, write_overlay_smil,
    write_package_opf,
};

const OPF_PATH_IN_ZIP: &str = "EPUB/package.opf";
const NAV_HREF: &str = "nav.xhtml";

impl Publication {
    /// Write this publication as an `.epub` ZIP archive to `writer`.
    ///
    /// `writer` must support seeking — the underlying ZIP format wants to
    /// patch the central directory at the end. Pass `&mut std::fs::File`
    /// or `&mut std::io::Cursor<Vec<u8>>` for in-memory output.
    ///
    /// If `metadata.modified` is empty, it is filled with the current UTC
    /// timestamp (`YYYY-MM-DDTHH:MM:SSZ`). Everything else is taken as-is.
    pub fn write_zip<W: Write + Seek>(&self, writer: W) -> Result<()> {
        self.validate()?;

        // Materialise a copy whose `modified` timestamp is guaranteed present.
        let mut publication = self.clone();
        if publication.metadata.modified.is_empty() {
            let now: DateTime<Utc> = Utc::now();
            publication.metadata.modified = now.format("%Y-%m-%dT%H:%M:%SZ").to_string();
        }

        let mut zip = ZipWriter::new(writer);

        // 1. mimetype — first, uncompressed, no extra field.
        let mimetype_opts = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .last_modified_time(zip::DateTime::default());
        write_entry(&mut zip, "mimetype", b"application/epub+zip", mimetype_opts)?;

        let deflate_opts = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .compression_level(Some(6));

        // 2. META-INF/container.xml
        write_entry(
            &mut zip,
            "META-INF/container.xml",
            write_container_xml(OPF_PATH_IN_ZIP).as_bytes(),
            deflate_opts,
        )?;

        // 3. EPUB/package.opf
        write_entry(
            &mut zip,
            OPF_PATH_IN_ZIP,
            write_package_opf(&publication).as_bytes(),
            deflate_opts,
        )?;

        // 4. EPUB/nav.xhtml
        let nav_path = format!("EPUB/{NAV_HREF}");
        let nav_xhtml = write_nav_xhtml(
            &publication.nav,
            &publication.metadata.language,
            &publication.metadata.title,
        );
        write_entry(&mut zip, &nav_path, nav_xhtml.as_bytes(), deflate_opts)?;

        // 5. EPUB/<content xhtml> for each section
        for section in &publication.sections {
            let path = format!("EPUB/{}", section.content.href);
            write_entry(
                &mut zip,
                &path,
                write_content_xhtml(&section.content).as_bytes(),
                deflate_opts,
            )?;
        }

        // 6. EPUB/<overlay smil> for each section that has one
        for section in &publication.sections {
            if let Some(overlay) = &section.overlay {
                let path = format!("EPUB/{}", overlay.href);
                write_entry(
                    &mut zip,
                    &path,
                    write_overlay_smil(overlay).as_bytes(),
                    deflate_opts,
                )?;
            }
        }

        // 7. EPUB/audio/* — Stored (no extra deflate over already-compressed audio)
        let audio_opts = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for audio in &publication.audio_files {
            let path = format!("EPUB/{}", audio.href);
            zip.start_file(&path, audio_opts)?;
            stream_into_zip(&mut zip, &audio.source_path, &path)?;
        }

        // 8. EPUB/<cover.href> — Stored (image is already compressed).
        if let Some(cover) = &publication.cover {
            let path = format!("EPUB/{}", cover.href);
            write_entry(&mut zip, &path, &cover.bytes, audio_opts)?;
        }

        zip.finish()?;
        Ok(())
    }
}

/// Start a ZIP entry and write `data` into it, mapping `io::Error` to
/// [`Error::Io`] with the entry's ZIP-internal path.
///
/// Centralising this means callers don't accidentally lose the path
/// context when writing fails — the previous design had a separate
/// `ZipIo(#[from] io::Error)` variant that erased the path because
/// `?` always picked the bare-`io::Error` conversion over the
/// contextualised one.
fn write_entry<W: Write + Seek>(
    zip: &mut ZipWriter<W>,
    zip_path: &str,
    data: &[u8],
    opts: SimpleFileOptions,
) -> Result<()> {
    zip.start_file(zip_path, opts)?;
    zip.write_all(data).map_err(|source| Error::Io {
        path: PathBuf::from(zip_path),
        source,
    })
}

fn stream_into_zip<W: Write + Seek>(
    zip: &mut ZipWriter<W>,
    src: &Path,
    zip_path: &str,
) -> Result<()> {
    let mut f = std::fs::File::open(src).map_err(|source| Error::Io {
        path: src.to_path_buf(),
        source,
    })?;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf).map_err(|source| Error::Io {
            path: src.to_path_buf(),
            source,
        })?;
        if n == 0 {
            break;
        }
        zip.write_all(&buf[..n]).map_err(|source| Error::Io {
            path: PathBuf::from(zip_path),
            source,
        })?;
    }
    Ok(())
}
