//! Server-sent events, line by line (the WHATWG event-stream format, as far as the Inbox
//! uses it): `event`, `data`, `id` and `retry` fields, `:` comments (its `: ping`), and a
//! blank line that ends a frame.

/// One frame: what one blank line dispatches.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Frame {
    /// `message` when the frame names none.
    pub(crate) event: String,
    pub(crate) data: String,
    pub(crate) id: Option<String>,
    /// The reconnection delay the server asks for, in milliseconds.
    pub(crate) retry: Option<u64>,
}

/// Fields gathered until the next blank line.
#[derive(Debug, Default)]
pub(crate) struct Parser {
    event: Option<String>,
    data: Option<String>,
    id: Option<String>,
    retry: Option<u64>,
}

impl Parser {
    /// Feed one line (with or without its line ending); a frame when the line ends one.
    pub(crate) fn line(&mut self, line: &str) -> Option<Frame> {
        let line = line.strip_suffix('\n').unwrap_or(line);
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            return self.dispatch();
        }
        if line.starts_with(':') {
            return None;
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "event" => self.event = Some(value.to_owned()),
            "data" => match &mut self.data {
                Some(data) => {
                    data.push('\n');
                    data.push_str(value);
                }
                None => self.data = Some(value.to_owned()),
            },
            "id" if !value.contains('\0') => self.id = Some(value.to_owned()),
            "retry" => self.retry = value.parse().ok().or(self.retry),
            _ => {}
        }
        None
    }

    fn dispatch(&mut self) -> Option<Frame> {
        let taken = std::mem::take(self);
        let retry = taken.retry;
        match taken.data {
            Some(data) => Some(Frame {
                event: taken.event.filter(|e| !e.is_empty()).unwrap_or_else(|| "message".to_owned()),
                data,
                id: taken.id,
                retry,
            }),
            // A frame with no data dispatches nothing, but a `retry` alone still counts.
            None => retry.map(|retry| Frame { retry: Some(retry), ..Frame::default() }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(text: &str) -> Vec<Frame> {
        let mut parser = Parser::default();
        text.split_inclusive('\n').filter_map(|line| parser.line(line)).collect()
    }

    #[test]
    fn the_inboxs_frames_parse_whatever_the_line_endings() {
        let text = "retry: 2000\nevent: hello\ndata: {\"cursor\":3}\n\n: ping\n\nid: 4\r\nevent: record\r\ndata: {\"seq\":4}\r\n\r\n";
        assert_eq!(
            frames(text),
            vec![
                Frame { event: "hello".into(), data: "{\"cursor\":3}".into(), id: None, retry: Some(2000) },
                Frame {
                    event: "record".into(),
                    data: "{\"seq\":4}".into(),
                    id: Some("4".into()),
                    retry: None
                },
            ]
        );
    }

    #[test]
    fn data_lines_join_and_a_frame_without_data_is_dropped() {
        assert_eq!(
            frames("event: x\n\ndata: a\ndata:b\n\n"),
            vec![Frame { event: "message".into(), data: "a\nb".into(), ..Frame::default() }]
        );
    }
}
