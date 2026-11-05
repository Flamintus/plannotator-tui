//! What a note's bubble in the rail says: the note, as it always did. Over a document whose
//! comments the review loop keeps, the replies are under the passage, in the note's thread
//! (`threads`): the bubble counts them rather than repeat them, and it is dimmed once the note
//! is resolved. Without the loop a record's replies and resolved state change nothing here.

use plannotator_tui_schema::{Annotation, State};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;

use crate::wrap::wrap_line;

/// The bubble's text wrapped to `width` columns; `label` stands in for an empty body.
/// `in_loop`: the review loop keeps the note, its replies are counted and its resolution dims it.
pub(super) fn lines(annotation: &Annotation, label: &str, width: usize, in_loop: bool) -> Vec<Line<'static>> {
    let (body, style) = if annotation.body.is_empty() {
        (label, Style::new().dim().italic())
    } else {
        (annotation.body.as_str(), Style::new())
    };
    let mut lines = wrapped(body, width, style);
    if !in_loop {
        return lines;
    }
    let replies = annotation.replies.len();
    if replies > 0 {
        let count = if replies == 1 { "1 reply".to_owned() } else { format!("{replies} replies") };
        lines.extend(wrapped(&format!("\u{21b3} {count} under the passage"), width, Style::new().dim()));
    }
    if annotation.state == State::Resolved {
        lines = lines.into_iter().map(|line| line.patch_style(Modifier::DIM)).collect();
    }
    lines
}

fn wrapped(text: &str, width: usize, style: Style) -> Vec<Line<'static>> {
    wrap_line(&Line::from(text), &[], width).into_iter().map(|row| row.line.style(style)).collect()
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use plannotator_tui_schema::Kind;

    use super::super::review_test_support::{answer_on_disk, draw, file_app, reply_json};

    /// A record another client wrote replies and a resolution into, without the review loop:
    /// the bubble is the note alone, as before the loop.
    #[test]
    fn without_the_review_loop_a_bubble_is_the_note_alone() {
        let (root, mut app, _) = file_app("bubble-alone");
        app.add_quote_annotation("two", Kind::Comment, "Why two?".into()).expect("note");
        let reply = serde_json::json!([reply_json("Two keeps it short.", "claude")]);
        answer_on_disk(&mut app, 0, reply, "resolved");
        let screen = draw(&mut app, 100, 14);
        assert!(screen.contains("Why two?"), "{screen}");
        for gone in ["claude", "Two keeps it short.", "\u{21b3}", "Resolved", "resolved"] {
            assert!(!screen.contains(gone), "{gone}: {screen}");
        }
        std::fs::remove_dir_all(root).expect("cleanup");
    }
}
