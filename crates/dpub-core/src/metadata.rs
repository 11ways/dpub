use std::collections::BTreeMap;

/// Dublin Core + DAISY-specific metadata extracted from the NCC's `<head>`.
///
/// Known DAISY 2.02 metadata names follow either the `dc:*` (Dublin Core) or
/// `ncc:*` (DAISY-specific) prefix and are typed where helpful. Unknown names
/// are preserved verbatim in [`Metadata::other`] so nothing is lost.
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct Metadata {
    pub title: Option<String>,
    pub creator: Option<String>,
    pub publisher: Option<String>,
    pub date: Option<String>,
    pub language: Option<String>,
    pub identifier: Option<String>,
    pub format: Option<String>,
    pub source: Option<String>,
    pub subject: Option<String>,

    pub charset: Option<String>,
    pub multimedia_type: Option<String>,
    pub narrator: Option<String>,
    pub total_time: Option<String>,
    pub depth: Option<u32>,
    pub toc_items: Option<u32>,
    pub page_normal: Option<u32>,
    pub files: Option<u32>,
    pub kbyte_size: Option<u32>,

    /// Any metadata `<meta>` tags whose `name` did not match a recognised field.
    pub other: BTreeMap<String, String>,
}

impl Metadata {
    /// Apply a single `<meta name="..." content="...">` tag to this metadata struct.
    pub(crate) fn set(&mut self, name: &str, content: String) {
        match name {
            "dc:title" | "dc:Title" => self.title = Some(content),
            "dc:creator" | "dc:Creator" => self.creator = Some(content),
            "dc:publisher" | "dc:Publisher" => self.publisher = Some(content),
            "dc:date" | "dc:Date" => self.date = Some(content),
            "dc:language" | "dc:Language" => self.language = Some(content),
            "dc:identifier" | "dc:Identifier" => self.identifier = Some(content),
            "dc:format" | "dc:Format" => self.format = Some(content),
            "dc:source" | "dc:Source" => self.source = Some(content),
            "dc:subject" | "dc:Subject" => self.subject = Some(content),

            "ncc:charset" => self.charset = Some(content),
            "ncc:multimediaType" => self.multimedia_type = Some(content),
            "ncc:narrator" => self.narrator = Some(content),
            "ncc:totalTime" => self.total_time = Some(content),
            "ncc:depth" => self.depth = content.parse().ok(),
            "ncc:tocItems" => self.toc_items = content.parse().ok(),
            "ncc:pageNormal" => self.page_normal = content.parse().ok(),
            "ncc:files" => self.files = content.parse().ok(),
            "ncc:kByteSize" => self.kbyte_size = content.parse().ok(),

            _ => {
                self.other.insert(name.to_owned(), content);
            }
        }
    }
}
