//! What a note's bubble in the rail says: the note, then each reply under it with who wrote
//! it, all of it dimmed once the note is resolved. The record already carries replies and
//! the resolved state (another client may write them); this only shows them.

use plannotator_tui_schema::{Annotation, Reply, State};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;

use crate::wrap::wrap_line;

/// Columns a reply's text sits in from the bubble's edge, under its `↳ author` line.
const REPLY_INDENT: usize = 2;

/// The bubble's text wrapped to `width` columns; `label` stands in for an empty body.
pub(super) fn lines(annotation: &Annotation, label: &str, width: usize) -> Vec<Line<'static>> {
    let (body, style) = if annotation.body.is_empty() {
        (label, Style::new().dim().italic())
    } else {
        (annotation.body.as_str(), Style::new())
    };
    let mut lines = wrapped(body, width, style);
    for reply in &annotation.replies {
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
fn author(reply: &Reply) -> &str {
    reply.author_name.as_deref().or(reply.author.as_deref()).unwrap_or("anonymous")
}

fn wrapped(text: &str, width: usize, style: Style) -> Vec<Line<'static>> {
    wrap_line(&Line::from(text), &[], width).into_iter().map(|row| row.line.style(style)).collect()
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]
mod tests {
    use plannotator_tui_schema::Kind;

    use super::super::review_test_support::{draw, file_app, reopen};
    use crate::app::App;
    use crate::store::Location;

    /// Give the open file's one note `replies` and `state` in its record on disk, as another
    /// client would, and read the record back.
    fn answer_on_disk(app: &mut App, replies: serde_json::Value, state: &str) {
        let path = match &app.open.source.provenance {
            plannotator_tui_schema::Provenance::File { path } => Some(path.clone()),
            _ => None,
        };
        let record = Location::for_file(&app.data_dir, &app.project, &path.expect("a file review")).record;
        let text = std::fs::read_to_string(&record).expect("record");
        let mut json: serde_json::Value = serde_json::from_str(&text).expect("json");
        let note = &mut json["annotations"][0];
        note["replies"] = replies;
        note["state"] = state.into();
        std::fs::write(&record, json.to_string()).expect("write record");
        reopen(app);
    }

    fn reply(body: &str, author: &str) -> serde_json::Value {
        serde_json::json!({
            "id": format!("reply-{author}"), "annotation_id": "x", "body": body, "author": author,
            "created_at": "2026-11-05T10:00:00.000Z", "updated_at": "2026-11-05T10:00:00.000Z"
        })
    }

    #[test]
    fn a_notes_replies_are_drawn_under_it_with_their_authors() {
        let (root, mut app, _) = file_app("bubble-replies");
        app.add_quote_annotation("two", Kind::Comment, "Why two?".into()).expect("note");
        answer_on_disk(&mut app, serde_json::json!([reply("Two keeps it short.", "claude")]), "open");
        let screen = draw(&mut app, 100, 14);
        let rows: Vec<&str> = screen.lines().collect();
        let at = |needle: &str| rows.iter().position(|row| row.contains(needle)).expect(needle);
        assert!(at("Why two?") < at("\u{21b3} claude"), "{screen}");
        assert!(at("\u{21b3} claude") < at("Two keeps it short."), "{screen}");
        assert!(!screen.contains("resolved"), "{screen}");
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn a_resolved_note_says_so_in_its_title() {
        let (root, mut app, _) = file_app("bubble-resolved");
        app.add_quote_annotation("two", Kind::Comment, "Why two?".into()).expect("note");
        answer_on_disk(&mut app, serde_json::json!([]), "resolved");
        let screen = draw(&mut app, 100, 14);
        let title = screen.lines().find(|row| row.contains("\u{1f4ac}")).expect("the bubble's title");
        assert!(title.contains("\u{b7} resolved"), "{screen}");
        std::fs::remove_dir_all(root).expect("cleanup");
    }
}
