//! What a note's bubble in the rail says: the note, then each reply under it with who wrote
//! it, all of it dimmed once the note is resolved. The record already carries replies and
//! the resolved state (another client may write them); this only shows them. While the
//! note's thread is open in the document's flow (`threads`), its replies are read there and
//! the bubble only counts them.

use plannotator_tui_schema::{Annotation, Reply, State};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;

use crate::wrap::wrap_line;

/// Columns a reply's text sits in from the bubble's edge, under its `↳ author` line.
const REPLY_INDENT: usize = 2;

/// The bubble's text wrapped to `width` columns; `label` stands in for an empty body.
/// `in_flow`: the replies are in view under the passage, so they are counted, not repeated.
pub(super) fn lines(annotation: &Annotation, label: &str, width: usize, in_flow: bool) -> Vec<Line<'static>> {
    let (body, style) = if annotation.body.is_empty() {
        (label, Style::new().dim().italic())
    } else {
        (annotation.body.as_str(), Style::new())
    };
    let mut lines = wrapped(body, width, style);
    let replies = annotation.replies.len();
    if in_flow && replies > 0 {
        let count = if replies == 1 { "1 reply".to_owned() } else { format!("{replies} replies") };
        lines.extend(wrapped(&format!("\u{21b3} {count} under the passage"), width, Style::new().dim()));
    }
    for reply in annotation.replies.iter().filter(|_| !in_flow) {
        lines.push(Line::styled(format!("\u{21b3} {}", author(reply)), Style::new().dim().bold()));
        let indent = " ".repeat(REPLY_INDENT);
        let text = wrapped(&reply.body, width.saturating_sub(REPLY_INDENT), Style::new());
        lines.extend(text.into_iter().map(|line| {
            let mut spans = vec![indent.clone().into()];
            spans.extend(line.spans);
            Line::from(spans).style(line.style)
        }));
    }
    if annotation.state == State::Resolved {
        lines = lines.into_iter().map(|line| line.patch_style(Modifier::DIM)).collect();
    }
    lines
}

/// The name a reply shows: the author's display name, else their label.
pub(super) fn author(reply: &Reply) -> &str {
    reply.author_name.as_deref().or(reply.author.as_deref()).unwrap_or("anonymous")
}

fn wrapped(text: &str, width: usize, style: Style) -> Vec<Line<'static>> {
    wrap_line(&Line::from(text), &[], width).into_iter().map(|row| row.line.style(style)).collect()
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use plannotator_tui_schema::Kind;

    use super::super::review_test_support::{answer_on_disk, draw, file_app, reply_json};

    #[test]
    fn a_notes_replies_are_listed_under_it_with_their_authors_while_its_thread_is_folded() {
        let (root, mut app, _) = file_app("bubble-replies");
        app.add_quote_annotation("two", Kind::Comment, "Why two?".into()).expect("note");
        answer_on_disk(
            &mut app,
            0,
            serde_json::json!([reply_json("Two keeps it short.", "claude")]),
            "resolved",
        );
        let screen = draw(&mut app, 100, 14);
        let rows: Vec<&str> = screen.lines().collect();
        let at = |needle: &str| rows.iter().position(|row| row.contains(needle)).expect(needle);
        assert!(at("Why two?") < at("\u{21b3} claude"), "{screen}");
        assert!(at("\u{21b3} claude") < at("Two keeps it short."), "{screen}");
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn a_resolved_note_says_so_in_its_title() {
        let (root, mut app, _) = file_app("bubble-resolved");
        app.add_quote_annotation("two", Kind::Comment, "Why two?".into()).expect("note");
        answer_on_disk(&mut app, 0, serde_json::json!([]), "resolved");
        let screen = draw(&mut app, 100, 14);
        // ` 💬 a1b2c · resolved ` then the border: the title, not the folded line's ` · resolved · `.
        assert!(screen.lines().any(|row| row.contains("\u{b7} resolved \u{2500}")), "{screen}");
        std::fs::remove_dir_all(root).expect("cleanup");
    }
}
