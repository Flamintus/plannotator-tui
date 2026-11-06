//! Where a comment of the journal is in its document: which comments of the review's view
//! are on the document's words, and which sub-threads under them on a box's passage, where
//! each one's quote is in them, and a note's images in the words the journal keeps.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use plannotator_tui_journal::{
    Anchor as On, CommentKind, NewComment, Placement, State as ThreadState, ThreadNode, ThreadView,
};
use plannotator_tui_schema::{Anchor, Annotation, Kind, NoteExtras, Reply, SourceRange, State, blob_sha};

use super::Kept;
use crate::doc::Document;
use crate::journal::{Target, on_file, passage, tree};

/// How an attached image is named in a note's words, as the feedback names it.
const IMAGE: &str = "Image: ";

/// The words of a looks-good, as Plannotator's screens write it and read it back: the journal
/// has no kind for it, these exact words are it.
pub(super) const LOOKS_GOOD: &str = "\u{1f44d} Looks good";

/// `words` then one `Image: <path>` line per image, after a blank line.
pub(super) fn with_images(words: &str, images: &[String]) -> String {
    let lines = images.iter().map(|image| format!("{IMAGE}{image}")).collect::<Vec<_>>().join("\n");
    match (words.trim().is_empty(), lines.is_empty()) {
        (_, true) => words.trim().to_owned(),
        (true, false) => lines,
        (false, false) => format!("{}\n\n{lines}", words.trim()),
    }
}

/// `with_images` undone: the words and the images of a note's text, when its text ends as
/// `with_images` writes it; else the text whole, no image.
fn split_images(text: &str) -> (String, Vec<String>) {
    let lines: Vec<&str> = text.split('\n').collect();
    let images: Vec<String> = lines
        .iter()
        .rev()
        .take_while(|line| line.len() > IMAGE.len() && line.starts_with(IMAGE))
        .map(|line| line.get(IMAGE.len()..).unwrap_or_default().to_owned())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let words = lines.get(..lines.len() - images.len()).unwrap_or_default().join("\n");
    let words = words.trim().to_owned();
    if images.is_empty() || with_images(&words, &images) != text {
        return (text.to_owned(), Vec::new());
    }
    (words, images)
}

impl Kept {
    /// Write `annotation` as a new comment, or as an edit of its draft, its words `text`: on
    /// the document's words, or a sub-thread on its box's passage, of its kind either way.
    pub(super) fn comment(&self, annotation: &Annotation, text: &str) -> Result<()> {
        let quote = annotation.anchor.rendered().to_owned();
        let deletion = annotation.anchor.kind() == Kind::Delete;
        let placement = match (&self.target, self.passages.get(&annotation.id)) {
            (_, Some(passage)) => Placement::Passage {
                parent: passage.parent.clone(),
                quote: passage.quote.clone(),
                quote_offset: passage.offset,
                deletion,
            },
            (Target::Message { message_id }, None) => {
                Placement::On { on: On::Message { message_id: message_id.clone() }, quote, deletion }
            }
            (Target::File { path }, None) => {
                let on = On::File { path: path.display().to_string(), lines: self.lines(annotation) };
                Placement::On { on, quote, deletion }
            }
        };
        let comment = NewComment { id: annotation.id.clone(), text: text.to_owned(), placement };
        let review = &self.review;
        // `now` is a reply's, which no note is.
        review
            .journal
            .comment(&review.target, &comment)
            .map(drop)
            .with_context(|| format!("comment {}", annotation.id))
    }

    /// The lines a note is on in the file, as the agent reads them, from where it was made.
    fn lines(&self, annotation: &Annotation) -> Option<[u64; 2]> {
        let range = annotation.anchor.plannotator_tui.as_ref()?.source.as_ref()?;
        let (first, last) = crate::export::line_span(&self.source, &(range.start..range.end));
        Some([first as u64, last as u64])
    }

    /// The comments of `view` this document's notes are: on a message or a file, its
    /// comments, each followed by the sub-threads under it at any depth, on the passages of its
    /// box.
    pub(super) fn comments<'v>(&self, view: &'v ThreadView) -> Vec<&'v ThreadNode> {
        let with_sub_threads = |node: &'v ThreadNode| std::iter::once(node).chain(tree::under(node));
        match &self.target {
            Target::Message { message_id } => view
                .threads
                .iter()
                .filter(|node| {
                    node.place.parent.is_none()
                        && matches!(&node.place.on, On::Message { message_id: on } if on == message_id)
                })
                .flat_map(with_sub_threads)
                .collect(),
            Target::File { path } => view
                .threads
                .iter()
                .filter(|node| node.place.parent.is_none() && on_file(&node.place.on, path))
                .flat_map(with_sub_threads)
                .collect(),
        }
    }

    /// Where `node`'s quote is in the document: where this review put it, else the first
    /// place the words as drawn hold it, else an anchor that resolves to nothing, as a
    /// sub-thread on a box's passage has, of its kind.
    pub(super) fn anchor(&self, node: &ThreadNode, doc: &Document) -> Anchor {
        if let Some(anchor) = self.anchors.get(&node.id) {
            return anchor.clone();
        }
        if self.passages.contains_key(&node.id) {
            return quoted(&node.place.quote, kind_of(node));
        }
        let quote = node.place.quote.clone();
        match passage::find(&self.chars, &doc.source, &quote, 0) {
            Some(range) => {
                let block = doc.block_containing(range.start);
                let range = SourceRange {
                    start: range.start,
                    end: range.end,
                    version: blob_sha(doc.source.as_bytes()),
                };
                Anchor::new(quote, &doc.source, range, kind_of(node), block)
            }
            None => quoted(&quote, kind_of(node)),
        }
    }
}

/// The kind of note a comment of the view is, at any depth: a deletion stays one, so writing
/// its draft again keeps it, and a looks-good is its exact words. Its note's kind, and its box's.
pub(crate) fn kind_of(node: &ThreadNode) -> Kind {
    kind_of_words(&node.text, node.kind == CommentKind::Deletion)
}

/// The kind of note words the journal keeps are, `deletion` when it says so.
fn kind_of_words(text: &str, deletion: bool) -> Kind {
    match (deletion, text == LOOKS_GOOD) {
        (true, _) => Kind::Delete,
        (false, true) => Kind::LooksGood,
        (false, false) => Kind::Comment,
    }
}

/// An anchor on `quote` with no place in the document's words, that keeps the note's kind: a
/// note whose words are not found there, or one on a passage of a thread's box.
pub(super) fn quoted(quote: &str, kind: Kind) -> Anchor {
    let extras =
        (kind != Kind::Comment).then(|| plannotator_tui_schema::Extras { kind, ..Default::default() });
    Anchor {
        original_text: quote.to_owned(),
        quote: quote.to_owned(),
        plannotator_tui: extras,
        other: BTreeMap::new(),
    }
}

/// A draft of the review's view as a note on `quote`, with no place in a document: its images
/// taken from its words; a deletion's anchor says so, and a looks-good's.
pub(crate) fn draft_note(id: &str, quote: &str, text: &str, deletion: bool) -> Annotation {
    let anchor = quoted(quote, kind_of_words(text, deletion));
    let (body, images) = split_images(text);
    Annotation {
        id: id.to_owned(),
        document_id: String::new(),
        anchor,
        body,
        author: None,
        author_name: None,
        state: State::Open,
        attachments: Vec::new(),
        plannotator_tui: NoteExtras { images, ..NoteExtras::default() },
        created_at: String::new(),
        updated_at: String::new(),
        replies: Vec::new(),
        other: BTreeMap::new(),
    }
}

/// A comment of the review's view as a note: its images taken from its words, its replies
/// flat under it, by who wrote them.
pub(super) fn note(node: &ThreadNode, anchor: Anchor) -> Annotation {
    let replies = node.replies.iter().map(|reply| {
        let author = tree::author(reply);
        Reply {
            id: reply.id.clone(),
            annotation_id: node.id.clone(),
            body: reply.text.clone(),
            author: Some(author),
            author_name: reply.author_name.clone(),
            created_at: reply.at.clone(),
            updated_at: reply.at.clone(),
            other: BTreeMap::new(),
        }
    });
    let (body, images) = split_images(&node.text);
    Annotation {
        id: node.id.clone(),
        document_id: String::new(),
        anchor,
        body,
        author: None,
        author_name: None,
        state: if node.state == ThreadState::Resolved { State::Resolved } else { State::Open },
        attachments: Vec::new(),
        plannotator_tui: NoteExtras { images, ..NoteExtras::default() },
        created_at: String::new(),
        updated_at: String::new(),
        replies: replies.collect(),
        other: BTreeMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::{split_images, with_images};

    #[test]
    fn a_notes_images_are_lines_of_its_words_and_come_back_out_of_them() {
        let images = vec!["/tmp/a.png".to_owned(), "/tmp/b c.png".to_owned()];
        let text = with_images("Look here.", &images);
        assert_eq!(text, "Look here.\n\nImage: /tmp/a.png\nImage: /tmp/b c.png");
        assert_eq!(split_images(&text), ("Look here.".to_owned(), images.clone()));
        assert_eq!(split_images(&with_images("", &images)), (String::new(), images));
        assert_eq!(with_images(" Plain. ", &[]), "Plain.");
    }

    #[test]
    fn words_that_only_look_like_images_stay_whole() {
        for text in ["See below.\nImage: /tmp/a.png", "Image: ", "Plain words."] {
            assert_eq!(split_images(text), (text.to_owned(), Vec::new()), "{text:?}");
        }
    }
}
