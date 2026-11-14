//! Marked-content recording while pages are painted.
//!
//! The recorder lives on the writer so every painter already reaches it. When
//! disabled it emits nothing and allocates nothing, which keeps untagged
//! output byte-identical.

use super::super::geometry::PdfRect;
use super::super::pdf_text::PdfTextString;
use super::artifacts::artifact_unmarked_paint;
use crate::accessibility::{
    ContentRole, DocumentMetadata, DocumentStructure, ElementOrdinal, ListNumbering,
    MissingAltPolicy, NodeFacts, StructureRef, StructureRole, StructureSkeleton,
};

/// Per-document tagging state, present only for accessible output.
#[derive(Default)]
pub(crate) struct StructureRecorder(Option<Box<Tagging>>);

pub(crate) struct Tagging {
    pub(super) metadata: DocumentMetadata,
    pub(super) skeleton: StructureSkeleton,
    pub(super) missing_alt: MissingAltPolicy,
    /// Pages already handed to the writer, in page order.
    pub(super) pages: Vec<PageRecord>,
    /// Form XObjects that carry marks of their own, with their parent-tree key.
    pub(super) forms: Vec<FormRecord>,
    /// Marks of the page being painted, indexed by MCID.
    current: Vec<MarkRecord>,
    /// Groups being painted that will become form XObjects: their marks get
    /// a namespace of their own (ISO 32000-1 14.7.4.4, `/StructParents` on
    /// the form), so the saved page state is restored when they close.
    form_scopes: Vec<FormScope>,
    /// Depth of enclosing artifacts and isolated groups: nothing inside them
    /// may allocate an MCID.
    suppression: u32,
    /// Depth of enclosing content marks: nested painters join the open mark.
    open: u32,
    /// Next `/StructParents` or `/StructParent` key.
    next_key: u32,
}

/// What one MCID stands for.
#[derive(Debug, Clone)]
pub(super) struct MarkRecord {
    pub(super) owner: StructureRef,
    pub(super) kind: MarkKind,
}

#[derive(Debug, Clone)]
pub(crate) enum MarkKind {
    /// Text; `actual_text` is what a reader should see instead of the shown
    /// glyphs (ISO 32000-1 14.9.4), for example a word whose line-end hyphen
    /// the line breaker inserted.
    Text { actual_text: Option<String> },
    /// A list item's geometric bullet.
    Label,
    /// A replaced image or inline SVG, with its painted box in default user
    /// space for the `Layout` `/BBox` attribute.
    Figure { bbox: PdfRect },
    /// A composited paint group cut into a form XObject.
    Group,
}

impl MarkKind {
    /// Text shown as is.
    pub(crate) const TEXT: Self = Self::Text { actual_text: None };
}

/// Typed artifact regions painters open explicitly (ISO 32000-1 14.8.2.2);
/// `Layout` artifacts come from the normalizer instead.
#[derive(Debug, Clone, Copy)]
pub(crate) enum ArtifactKind {
    /// Page furniture: headers, footers, repeated rows, continuation paint.
    Pagination,
}

impl ArtifactKind {
    fn operator(self) -> &'static str {
        match self {
            Self::Pagination => "/Artifact << /Type /Pagination >> BDC\n",
        }
    }
}

/// A marked-content sequence a painter has opened and must close.
#[must_use = "an open mark must be closed with StructureRecorder::end"]
pub(crate) enum OpenMark {
    /// Nothing was written (recorder disabled, or nested inside another mark).
    Skipped,
    /// A content mark; nested painters join it.
    Content,
    /// An artifact; nothing inside allocates an MCID.
    Suppressing,
    /// The owner mark around a group cut into a form XObject; marks inside
    /// belong to the form namespace.
    Form,
}

pub(super) struct PageRecord {
    pub(super) key: u32,
    pub(super) marks: Vec<MarkRecord>,
    pub(super) annotations: Vec<AnnotationRecord>,
}

/// Marks painted into a form XObject rather than directly onto the page.
pub(super) struct FormRecord {
    pub(super) key: u32,
    pub(super) object_id: usize,
    /// Index of the page that invokes the form.
    pub(super) page: usize,
    pub(super) marks: Vec<MarkRecord>,
}

struct FormScope {
    saved_marks: Vec<MarkRecord>,
    saved_open: u32,
    saved_suppression: u32,
    /// Where the group content starts, for renumbering when it ends up
    /// inline after all.
    content_start: usize,
}

pub(crate) struct AnnotationRecord {
    pub(crate) key: u32,
    pub(crate) object_id: usize,
    pub(crate) owner: StructureRef,
}

/// Parent-tree keys reserved for one page and its annotations.
pub(crate) struct PageKeys {
    pub(crate) page: u32,
    pub(crate) annotations: Vec<u32>,
}

impl StructureRecorder {
    pub(crate) fn for_document(structure: &DocumentStructure) -> Self {
        Self(Some(Box::new(Tagging {
            metadata: structure.metadata.clone(),
            skeleton: structure.skeleton.clone(),
            missing_alt: structure.missing_alt,
            pages: Vec::new(),
            forms: Vec::new(),
            current: Vec::new(),
            form_scopes: Vec::new(),
            suppression: 0,
            open: 0,
            next_key: 0,
        })))
    }

    /// Accessible output embeds every shown glyph (ISO 14289-1 7.21.4.1).
    pub(crate) fn font_policy(&self) -> crate::render::pdf_fonts::FontPolicy {
        if self.0.is_some() {
            crate::render::pdf_fonts::FontPolicy::Embedded
        } else {
            crate::render::pdf_fonts::FontPolicy::Historical
        }
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.0.is_some()
    }

    /// Open the marked content of `owner` around content about to be painted.
    pub(crate) fn begin_content(
        &mut self,
        content: &mut String,
        owner: StructureRef,
        kind: MarkKind,
    ) -> OpenMark {
        let Some(tagging) = self.0.as_mut() else {
            return OpenMark::Skipped;
        };
        if tagging.suppression > 0 || tagging.open > 0 {
            return OpenMark::Skipped;
        }
        if tagging.paints_as_artifact(owner, &kind) {
            return tagging.begin_suppressing(content, "/Artifact BMC\n");
        }
        tagging.open += 1;
        tagging.begin_mark(content, owner, kind);
        OpenMark::Content
    }

    /// Open a `Lbl` mark for a list item's geometric bullet. Inline boxes with
    /// any other identity stay unmarked: their text lines mark themselves.
    pub(crate) fn begin_label(&mut self, content: &mut String, owner: StructureRef) -> OpenMark {
        match owner {
            StructureRef::Content {
                role: ContentRole::Label(_),
                ..
            } => self.begin_content(content, owner, MarkKind::Label),
            _ => OpenMark::Skipped,
        }
    }

    /// Open the owner mark around a paint group that will be cut into a form
    /// XObject. Content inside keeps its own structure: the form gets a
    /// `/StructParents` entry and its marks are numbered from zero.
    pub(crate) fn begin_group(&mut self, content: &mut String, owner: StructureRef) -> OpenMark {
        let Some(tagging) = self.0.as_mut() else {
            return OpenMark::Skipped;
        };
        if tagging.suppression > 0 || tagging.open > 0 {
            return OpenMark::Skipped;
        }
        if tagging.paints_as_artifact(owner, &MarkKind::Group) {
            return tagging.begin_suppressing(content, "/Artifact BMC\n");
        }
        tagging.begin_mark(content, owner, MarkKind::Group);
        tagging.form_scopes.push(FormScope {
            saved_marks: std::mem::take(&mut tagging.current),
            saved_open: tagging.open,
            saved_suppression: tagging.suppression,
            content_start: content.len(),
        });
        tagging.open = 0;
        tagging.suppression = 0;
        OpenMark::Form
    }

    /// Close a group mark. `form` is the object id of the form XObject that
    /// received the group content, or `None` when it stayed inline.
    pub(crate) fn end_group(&mut self, content: &mut String, mark: OpenMark, form: Option<usize>) {
        let Some(tagging) = self.0.as_mut() else {
            return;
        };
        if !matches!(mark, OpenMark::Form) {
            self.end(content, mark);
            return;
        }
        let Some(scope) = tagging.form_scopes.pop() else {
            return;
        };
        let inner = std::mem::replace(&mut tagging.current, scope.saved_marks);
        tagging.open = scope.saved_open;
        tagging.suppression = scope.saved_suppression;
        match form {
            Some(object_id) => {
                let key = tagging.next_key;
                tagging.next_key += 1;
                tagging.forms.push(FormRecord {
                    key,
                    object_id,
                    page: tagging.pages.len(),
                    marks: inner,
                });
            }
            None => {
                // Nothing was cut: the marks are on the page after all, so
                // they join its namespace after the ones already allocated.
                let offset = tagging.current.len();
                renumber_mcids(content, scope.content_start, offset);
                tagging.current.extend(inner);
            }
        }
        content.push_str("EMC\n");
    }

    /// Open a typed artifact region (page furniture, repeated rows).
    pub(crate) fn begin_artifact(&mut self, content: &mut String, kind: ArtifactKind) -> OpenMark {
        let Some(tagging) = self.0.as_mut() else {
            return OpenMark::Skipped;
        };
        if tagging.suppression > 0 || tagging.open > 0 {
            return OpenMark::Skipped;
        }
        tagging.begin_suppressing(content, kind.operator())
    }

    pub(crate) fn end(&mut self, content: &mut String, mark: OpenMark) {
        let Some(tagging) = self.0.as_mut() else {
            return;
        };
        match mark {
            OpenMark::Skipped => {}
            OpenMark::Content => {
                content.push_str("EMC\n");
                tagging.open = tagging.open.saturating_sub(1);
            }
            OpenMark::Suppressing => {
                content.push_str("EMC\n");
                tagging.suppression = tagging.suppression.saturating_sub(1);
            }
            OpenMark::Form => self.end_group(content, OpenMark::Form, None),
        }
    }

    /// Wrap every painting operator outside marked content in a `Layout`
    /// artifact. Runs only in tagged mode and only on page-level streams.
    pub(crate) fn normalize_page_stream(&self, stream: &mut String) {
        if self.0.is_some() {
            *stream = artifact_unmarked_paint(stream);
        }
    }

    /// Reserve parent-tree keys for the page being added and its annotations.
    pub(crate) fn page_keys(&mut self, annotation_count: usize) -> Option<PageKeys> {
        let tagging = self.0.as_mut()?;
        let page = tagging.next_key;
        let annotations = (1..=annotation_count as u32)
            .map(|offset| page + offset)
            .collect();
        tagging.next_key = page + 1 + annotation_count as u32;
        Some(PageKeys { page, annotations })
    }

    /// Record the finished page's marks and annotations.
    pub(crate) fn finish_page(&mut self, keys: PageKeys, annotations: Vec<AnnotationRecord>) {
        if let Some(tagging) = self.0.as_mut() {
            tagging.pages.push(PageRecord {
                key: keys.page,
                marks: std::mem::take(&mut tagging.current),
                annotations,
            });
        }
    }

    /// The accessible name of a link, in HTML order: link text, then
    /// `aria-label`, `title`, and finally the URL.
    pub(crate) fn link_description(
        &self,
        owner: StructureRef,
        text: &str,
        url: &str,
    ) -> PdfTextString {
        let facts = self.0.as_deref().and_then(|tagging| tagging.facts(owner));
        let text = text.trim();
        let name = if !text.is_empty() {
            text
        } else {
            facts
                .and_then(|facts| facts.aria_label.as_deref().or(facts.title.as_deref()))
                .unwrap_or(url)
        };
        PdfTextString::from(name)
    }

    pub(crate) fn into_tagging(self) -> Option<Box<Tagging>> {
        self.0
    }
}

impl Tagging {
    /// `/ParentTreeNextKey`: above every page and annotation key.
    pub(super) fn next_parent_key(&self) -> u32 {
        self.next_key
    }
}

/// Shift every `/MCID n` the recorder wrote after `start` by `offset`.
fn renumber_mcids(content: &mut String, start: usize, offset: usize) {
    if offset == 0 || start >= content.len() {
        return;
    }
    const KEY: &str = "<< /MCID ";
    let mut renumbered = String::with_capacity(content.len() - start);
    let mut rest = &content[start..];
    while let Some(at) = rest.find(KEY) {
        let digits_start = at + KEY.len();
        let digits_end = digits_start
            + rest[digits_start..]
                .find(|character: char| !character.is_ascii_digit())
                .unwrap_or(rest.len() - digits_start);
        renumbered.push_str(&rest[..digits_start]);
        match rest[digits_start..digits_end].parse::<usize>() {
            Ok(mcid) => renumbered.push_str(&(mcid + offset).to_string()),
            Err(_) => renumbered.push_str(&rest[digits_start..digits_end]),
        }
        rest = &rest[digits_end..];
    }
    renumbered.push_str(rest);
    content.truncate(start);
    content.push_str(&renumbered);
}

impl Tagging {
    fn begin_suppressing(&mut self, content: &mut String, operator: &str) -> OpenMark {
        content.push_str(operator);
        self.suppression += 1;
        OpenMark::Suppressing
    }

    fn begin_mark(&mut self, content: &mut String, owner: StructureRef, kind: MarkKind) {
        let mcid = self.current.len();
        let footnote_role = match owner {
            StructureRef::Content { role, .. } => Some(role),
            _ => None,
        };
        let tag = match (&kind, footnote_role) {
            (MarkKind::Label, _) => "Lbl",
            (MarkKind::Figure { .. }, _) => "Figure",
            (_, Some(ContentRole::FootnoteCall)) => "Reference",
            (_, Some(ContentRole::Note)) => "Note",
            (_, Some(ContentRole::NoteLabel)) => "Lbl",
            _ => self.emitting_role(owner).pdf_name(),
        };
        let actual_text = match (&kind, owner) {
            (
                MarkKind::Label,
                StructureRef::Content {
                    role: ContentRole::Label(numbering),
                    ..
                },
            ) => numbering.bullet().map(PdfTextString::from),
            (
                MarkKind::Text {
                    actual_text: Some(text),
                },
                _,
            ) => Some(PdfTextString::from(text.as_str())),
            _ => None,
        }
        .map(|text| format!(" /ActualText {text}"))
        .unwrap_or_default();
        content.push_str(&format!("/{tag} << /MCID {mcid}{actual_text} >> BDC\n"));
        self.current.push(MarkRecord { owner, kind });
    }

    /// Hidden content, and figures without a name under the `Artifact`
    /// policy, are painted as artifacts instead of being tagged.
    fn paints_as_artifact(&self, owner: StructureRef, kind: &MarkKind) -> bool {
        if owner == StructureRef::Decorative {
            return true;
        }
        matches!(kind, MarkKind::Figure { .. })
            && self.missing_alt == MissingAltPolicy::Artifact
            && self.emitting_role(owner) == StructureRole::Figure
            && self
                .facts(owner)
                .is_none_or(|facts| facts.accessible_name().is_none())
    }

    /// The nearest element that emits a structure element for `owner`'s
    /// content; the `Document` for content without a known element.
    pub(super) fn emitting_element(&self, owner: StructureRef) -> ElementOrdinal {
        let StructureRef::Content { element, .. } = owner else {
            return ElementOrdinal::DOCUMENT;
        };
        let mut ordinal = element;
        while let Some(node) = self.skeleton.node(ordinal) {
            if node.role.emits_element() {
                return ordinal;
            }
            ordinal = node.parent;
        }
        ElementOrdinal::DOCUMENT
    }

    fn emitting_role(&self, owner: StructureRef) -> StructureRole {
        self.skeleton
            .node(self.emitting_element(owner))
            .map_or(StructureRole::Document, |node| node.role)
    }

    fn facts(&self, owner: StructureRef) -> Option<&NodeFacts> {
        self.skeleton
            .node(self.emitting_element(owner))
            .map(|node| &node.facts)
    }
}

impl ListNumbering {
    /// The character a geometric bullet stands for, for `/ActualText`.
    fn bullet(self) -> Option<&'static str> {
        match self {
            Self::Disc => Some("\u{2022}"),
            Self::Circle => Some("\u{25E6}"),
            Self::Square => Some("\u{25AA}"),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accessibility::{DocumentLanguage, DocumentTitle, TaggedPdf};
    use crate::parser::html::parse_html;

    fn recorder(html: &str) -> (StructureRecorder, Vec<crate::parser::dom::DomNode>) {
        let mut nodes = parse_html(html).unwrap();
        let skeleton = StructureSkeleton::assign(&mut nodes);
        let structure = DocumentStructure {
            metadata: DocumentMetadata {
                title: "T".parse::<DocumentTitle>().unwrap(),
                language: "en".parse::<DocumentLanguage>().unwrap(),
            },
            skeleton,
            missing_alt: TaggedPdf::default().missing_alt,
        };
        (StructureRecorder::for_document(&structure), nodes)
    }

    fn content_ref(element: u32) -> StructureRef {
        StructureRef::Content {
            element: ElementOrdinal::from_skeleton_index(element as usize - 1),
            role: ContentRole::Body,
        }
    }

    #[test]
    fn disabled_recorder_writes_nothing() {
        let mut recorder = StructureRecorder::default();
        let mut content = String::new();
        let mark = recorder.begin_content(&mut content, content_ref(2), MarkKind::TEXT);
        recorder.end(&mut content, mark);
        let region = recorder.begin_artifact(&mut content, ArtifactKind::Pagination);
        recorder.end(&mut content, region);
        assert_eq!(content, "");
        assert!(recorder.page_keys(3).is_none());
    }

    #[test]
    fn marks_allocate_dense_mcids_and_nested_painters_join_the_open_mark() {
        // ISO 32000-1 14.7.4.4: MCIDs index the page's parent-tree entry, so
        // they are allocated densely from 0 per page.
        let (mut recorder, _nodes) = recorder("<p>a <em>b</em></p>");
        let mut content = String::new();
        let outer = recorder.begin_content(&mut content, content_ref(2), MarkKind::TEXT);
        let inner = recorder.begin_content(&mut content, content_ref(3), MarkKind::TEXT);
        assert!(matches!(inner, OpenMark::Skipped));
        recorder.end(&mut content, inner);
        recorder.end(&mut content, outer);
        let next = recorder.begin_content(&mut content, content_ref(2), MarkKind::TEXT);
        recorder.end(&mut content, next);
        assert_eq!(
            content,
            "/P << /MCID 0 >> BDC\nEMC\n/P << /MCID 1 >> BDC\nEMC\n"
        );
    }

    #[test]
    fn hidden_content_and_artifact_regions_suppress_allocation() {
        // ISO 14289-1 7.1: artifacts carry no MCID; nothing inside one may be
        // tagged (veraPDF 7.1-1).
        let (mut recorder, _nodes) = recorder("<p>a</p>");
        let mut content = String::new();
        let region = recorder.begin_artifact(&mut content, ArtifactKind::Pagination);
        let inside = recorder.begin_content(&mut content, content_ref(2), MarkKind::TEXT);
        assert!(matches!(inside, OpenMark::Skipped));
        recorder.end(&mut content, inside);
        recorder.end(&mut content, region);
        let hidden = recorder.begin_content(&mut content, StructureRef::Decorative, MarkKind::TEXT);
        recorder.end(&mut content, hidden);
        assert_eq!(
            content,
            "/Artifact << /Type /Pagination >> BDC\nEMC\n/Artifact BMC\nEMC\n"
        );
        let keys = recorder.page_keys(0).unwrap();
        recorder.finish_page(keys, Vec::new());
        assert!(recorder.0.as_ref().unwrap().pages[0].marks.is_empty());
    }

    #[test]
    fn groups_cut_into_forms_keep_their_own_mcid_namespace() {
        // ISO 32000-1 14.7.4.4: marked content inside a form XObject is keyed
        // by the form /StructParents entry and numbered from zero.
        let (mut recorder, _nodes) = recorder("<div><p>a</p></div>");
        let mut content = String::new();
        let first = recorder.begin_content(&mut content, content_ref(2), MarkKind::TEXT);
        recorder.end(&mut content, first);
        let group = recorder.begin_group(&mut content, content_ref(2));
        let inner = recorder.begin_content(&mut content, content_ref(3), MarkKind::TEXT);
        recorder.end(&mut content, inner);
        recorder.end_group(&mut content, group, Some(42));
        let after = recorder.begin_content(&mut content, content_ref(2), MarkKind::TEXT);
        recorder.end(&mut content, after);
        assert_eq!(
            content,
            "/Div << /MCID 0 >> BDC\nEMC\n/Div << /MCID 1 >> BDC\n/P << /MCID 0 >> BDC\nEMC\nEMC\n/Div << /MCID 2 >> BDC\nEMC\n"
        );
        let tagging = recorder.0.as_ref().unwrap();
        assert_eq!(tagging.forms.len(), 1);
        assert_eq!((tagging.forms[0].key, tagging.forms[0].object_id), (0, 42));
        assert_eq!(tagging.forms[0].marks.len(), 1);
        assert_eq!(tagging.current.len(), 3);
    }

    #[test]
    fn groups_left_inline_renumber_their_marks_into_the_page() {
        let (mut recorder, _nodes) = recorder("<div><p>a</p></div>");
        let mut content = String::new();
        let first = recorder.begin_content(&mut content, content_ref(2), MarkKind::TEXT);
        recorder.end(&mut content, first);
        let group = recorder.begin_group(&mut content, content_ref(2));
        let inner = recorder.begin_content(&mut content, content_ref(3), MarkKind::TEXT);
        recorder.end(&mut content, inner);
        recorder.end_group(&mut content, group, None);
        assert_eq!(
            content,
            "/Div << /MCID 0 >> BDC\nEMC\n/Div << /MCID 1 >> BDC\n/P << /MCID 2 >> BDC\nEMC\nEMC\n"
        );
        assert_eq!(recorder.0.as_ref().unwrap().current.len(), 3);
        assert!(recorder.0.as_ref().unwrap().forms.is_empty());
    }

    #[test]
    fn labels_carry_the_bullet_as_actual_text() {
        // ISO 32000-1 14.9.4: a path-drawn bullet needs /ActualText to be
        // read; ISO 14289-1 7.2 requires LI content under Lbl or LBody.
        let (mut recorder, _nodes) = recorder("<ul><li>x</li></ul>");
        let mut content = String::new();
        let label = StructureRef::Content {
            element: ElementOrdinal::from_skeleton_index(2),
            role: ContentRole::Label(ListNumbering::Disc),
        };
        let mark = recorder.begin_label(&mut content, label);
        recorder.end(&mut content, mark);
        assert_eq!(
            content,
            "/Lbl << /MCID 0 /ActualText <FEFF2022> >> BDC\nEMC\n"
        );
        let not_a_label = recorder.begin_label(&mut content, content_ref(3));
        assert!(matches!(not_a_label, OpenMark::Skipped));
    }

    #[test]
    fn synthesized_hyphens_are_replaced_by_a_soft_hyphen_in_actual_text() {
        // ISO 32000-1 14.9.4: a hyphen inserted at a line break is not part
        // of the text; /ActualText carries the word with U+00AD instead.
        let (mut recorder, _nodes) = recorder("<p>hyphenation</p>");
        let mut content = String::new();
        let kind = MarkKind::Text {
            actual_text: crate::layout::engine::RunHyphenation::SynthesizedHyphen
                .actual_text("hy-"),
        };
        let mark = recorder.begin_content(&mut content, content_ref(2), kind);
        recorder.end(&mut content, mark);
        assert_eq!(
            content,
            "/P << /MCID 0 /ActualText <FEFF0068007900AD> >> BDC\nEMC\n"
        );
    }

    #[test]
    fn parent_tree_keys_are_shared_between_pages_and_annotations() {
        // ISO 32000-1 14.7.4.4: page /StructParents and annotation
        // /StructParent keys index one number tree.
        let (mut recorder, _nodes) = recorder("<p>a</p>");
        let first = recorder.page_keys(2).unwrap();
        assert_eq!((first.page, first.annotations.clone()), (0, vec![1, 2]));
        recorder.finish_page(first, Vec::new());
        let second = recorder.page_keys(0).unwrap();
        assert_eq!(second.page, 3);
        assert!(second.annotations.is_empty());
    }

    #[test]
    fn link_descriptions_follow_the_accessible_name_order() {
        // ISO 14289-1 7.18.5: link annotations need a /Contents description.
        let (recorder, _nodes) =
            recorder(r#"<p><a href="https://x" title="Site" aria-label="Example site">go</a></p>"#);
        let link = content_ref(3);
        assert_eq!(
            recorder
                .link_description(link, " go ", "https://x")
                .to_string(),
            "(go)"
        );
        assert_eq!(
            recorder.link_description(link, "", "https://x").to_string(),
            "(Example site)"
        );
        assert_eq!(
            StructureRecorder::default()
                .link_description(link, "", "https://x")
                .to_string(),
            "(https://x)"
        );
    }
}
