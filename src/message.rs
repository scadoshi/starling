//! Turns a Teams message body, which is HTML, into plain pieces the UI can draw.
//!
//! Message bodies come from other people, so every string that leaves this module has had its control characters removed and cannot carry a terminal escape sequence.

/// One piece of a message body, in the order it appears.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Segment {
    /// HTML text, not yet cleaned. Adjacent text and emoji are merged into one segment.
    Text(String),
    /// An inline `<img>`, by its attributes.
    Image {
        src: Option<String>,
        alt: Option<String>,
    },
    /// An `<attachment>` by its `id`, which matches an entry in the message's `attachments`.
    Attachment(String),
}

#[derive(Debug, Clone, Copy)]
enum Tag {
    Img,
    Attachment,
    Emoji,
}

/// Splits a message body into text, inline images and attachments, keeping their order.
///
/// Quoted replies (`messageReference` attachments) are dropped. An unterminated tag ends the body.
pub(crate) fn parse(html: &str) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut rest = html;

    while let Some((pos, tag)) = next_tag(rest) {
        let Some((before, from_tag)) = rest.split_at_checked(pos) else {
            break;
        };
        push_text(&mut segments, before);

        let Some((tag_body, after)) = from_tag.split_once('>') else {
            rest = "";
            break;
        };

        rest = match tag {
            Tag::Img => {
                segments.push(Segment::Image {
                    src: attr(tag_body, "src").map(str::to_owned),
                    alt: attr(tag_body, "alt").map(str::to_owned),
                });
                after
            }
            Tag::Attachment => {
                if !tag_body.contains("messageReference")
                    && let Some(id) = attr(tag_body, "id")
                {
                    segments.push(Segment::Attachment(id.to_owned()));
                }
                if tag_body.ends_with('/') {
                    after
                } else {
                    after
                        .split_once("</attachment>")
                        .map_or(after, |(_, tail)| tail)
                }
            }
            Tag::Emoji => {
                if let Some(alt) = attr(tag_body, "alt") {
                    push_text(&mut segments, alt);
                }
                after
                    .strip_prefix("</emoji")
                    .and_then(|close| close.split_once('>'))
                    .map_or(after, |(_, tail)| tail)
            }
        };
    }

    push_text(&mut segments, rest);
    segments
}

fn next_tag(html: &str) -> Option<(usize, Tag)> {
    [
        (html.find("<img"), Tag::Img),
        (html.find("<attachment"), Tag::Attachment),
        (html.find("<emoji"), Tag::Emoji),
    ]
    .into_iter()
    .filter_map(|(pos, tag)| pos.map(|p| (p, tag)))
    .min_by_key(|&(pos, _)| pos)
}

fn push_text(segments: &mut Vec<Segment>, text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(Segment::Text(last)) = segments.last_mut() {
        last.push_str(text);
    } else {
        segments.push(Segment::Text(text.to_owned()));
    }
}

/// The value of attribute `name` in a tag's body, single or double quoted.
fn attr<'a>(tag_body: &'a str, name: &str) -> Option<&'a str> {
    ['"', '\''].into_iter().find_map(|quote| {
        let (_, value_on) = tag_body.split_once(&format!(" {name}={quote}"))?;
        value_on.split_once(quote).map(|(value, _)| value)
    })
}

/// Converts a fragment of message HTML to plain text.
///
/// Block tags become line breaks, other tags are removed, entities are decoded after the tags are gone, control characters are dropped, runs of blank lines collapse to one, and the ends are trimmed.
pub(crate) fn clean_text(html: &str) -> String {
    let with_breaks = [
        "</p>", "</div>", "</li>", "<br>", "<br/>", "<br />", "</br>",
    ]
    .into_iter()
    .fold(html.to_owned(), |text, tag| text.replace(tag, "\n"));

    let mut untagged = String::with_capacity(with_breaks.len());
    let mut inside_tag = false;
    for c in with_breaks.chars() {
        match c {
            '<' => inside_tag = true,
            '>' => inside_tag = false,
            _ if !inside_tag => untagged.push(c),
            _ => {}
        }
    }

    let decoded = [
        ("&nbsp;", " "),
        ("&#160;", " "),
        ("&nbsp", " "),
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", "\""),
        ("&#39;", "'"),
        ("&apos;", "'"),
        ("&amp;", "&"),
    ]
    .into_iter()
    .fold(untagged, |text, (entity, plain)| {
        text.replace(entity, plain)
    });

    let mut cleaned = String::with_capacity(decoded.len());
    let mut newlines = 0_u8;
    for c in decoded.chars() {
        if c == '\n' {
            newlines = newlines.saturating_add(1);
            if newlines <= 2 {
                cleaned.push(c);
            }
        } else if !c.is_control() {
            newlines = 0;
            cleaned.push(c);
        }
    }
    cleaned.trim().to_owned()
}

/// Removes control characters, so text from the network can be drawn on one terminal line.
pub(crate) fn sanitize(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

/// Breaks `text` into lines of at most `max_width` characters, on whitespace.
///
/// A word longer than `max_width` gets a line of its own and is left for the terminal to wrap.
pub(crate) fn wrap(text: &str, max_width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.lines() {
        let mut line = String::new();
        let mut line_width = 0_usize;
        for word in paragraph.split_whitespace() {
            let word_width = word.chars().count();
            if line_width > 0 && line_width.saturating_add(1).saturating_add(word_width) > max_width
            {
                lines.push(std::mem::take(&mut line));
                line_width = 0;
            }
            if line_width > 0 {
                line.push(' ');
                line_width = line_width.saturating_add(1);
            }
            line.push_str(word);
            line_width = line_width.saturating_add(word_width);
        }
        if !line.is_empty() {
            lines.push(line);
        }
    }
    lines
}

/// The label for an inline image: its alt text, else its file name without a query string, else "Pasted Image".
pub(crate) fn image_label(src: Option<&str>, alt: Option<&str>) -> String {
    let file_name = || {
        src?.rsplit(['/', '\\'])
            .next()
            .and_then(|name| name.split('?').next())
            .filter(|name| !name.is_empty())
    };
    sanitize(
        alt.filter(|a| !a.is_empty())
            .or_else(file_name)
            .unwrap_or("Pasted Image"),
    )
}

/// An icon for an attachment, from its content type when that is known, else from its file extension.
pub(crate) fn attachment_icon(name: &str, content_type: Option<&str>) -> &'static str {
    if let Some(ct) = content_type {
        let by_type = [
            (ct.starts_with("image/"), "🖼️"),
            (ct == "application/pdf", "📄"),
            (ct.contains("excel") || ct.contains("spreadsheet"), "📊"),
            (ct.contains("word") || ct.contains("document"), "📝"),
            (
                ct.contains("powerpoint") || ct.contains("presentation"),
                "📊",
            ),
            (ct.starts_with("video/"), "🎥"),
            (ct.starts_with("audio/"), "🎵"),
            (ct.contains("zip") || ct.contains("archive"), "📦"),
        ];
        if let Some((_, icon)) = by_type.into_iter().find(|&(matches, _)| matches) {
            return icon;
        }
    }

    let Some((_, ext)) = name.rsplit_once('.') else {
        return "📎";
    };
    match ext.to_lowercase().as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "svg" | "webp" => "🖼️",
        "pdf" | "txt" => "📄",
        "doc" | "docx" => "📝",
        "xls" | "xlsx" | "csv" | "ppt" | "pptx" => "📊",
        "mp4" | "avi" | "mov" | "mkv" | "webm" => "🎥",
        "mp3" | "wav" | "ogg" | "flac" => "🎵",
        "zip" | "rar" | "7z" | "tar" | "gz" => "📦",
        "html" | "htm" => "🌐",
        "json" | "xml" => "📋",
        _ => "📎",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Segment {
        Segment::Text(s.to_owned())
    }

    #[test]
    fn parse_keeps_plain_text_whole() {
        assert_eq!(parse("<p>hello</p>"), vec![text("<p>hello</p>")]);
    }

    #[test]
    fn parse_keeps_images_and_attachments_in_order() {
        let html = r#"before<img src="https://x/a.png?w=1" alt="cat">mid<attachment id="abc"></attachment>after"#;
        assert_eq!(
            parse(html),
            vec![
                text("before"),
                Segment::Image {
                    src: Some("https://x/a.png?w=1".to_owned()),
                    alt: Some("cat".to_owned()),
                },
                text("mid"),
                Segment::Attachment("abc".to_owned()),
                text("after"),
            ]
        );
    }

    #[test]
    fn parse_merges_emoji_into_surrounding_text() {
        let html = r#"hi <emoji id="smile" alt="😀"></emoji> there"#;
        assert_eq!(parse(html), vec![text("hi 😀 there")]);
    }

    #[test]
    fn parse_drops_quoted_replies() {
        let html = r#"<attachment id="1" type="messageReference"></attachment>reply"#;
        assert_eq!(parse(html), vec![text("reply")]);
    }

    #[test]
    fn parse_handles_self_closing_attachments() {
        assert_eq!(
            parse(r#"<attachment id="a"/>x"#),
            vec![Segment::Attachment("a".to_owned()), text("x")]
        );
    }

    #[test]
    fn parse_matches_whole_attribute_names() {
        let segments = parse(r#"<attachment itemid="wrong" id="right"></attachment>"#);
        assert_eq!(segments, vec![Segment::Attachment("right".to_owned())]);
    }

    #[test]
    fn parse_survives_unterminated_and_multibyte_input() {
        assert_eq!(parse("héllo 👋 <img src=\"x"), vec![text("héllo 👋 ")]);
        assert_eq!(parse("<emoji"), vec![]);
        assert_eq!(parse(""), vec![]);
    }

    #[test]
    fn clean_text_turns_blocks_into_lines() {
        assert_eq!(clean_text("<p>one</p><p>two</p>"), "one\ntwo");
        assert_eq!(clean_text("a<br>b<br/>c<br />d"), "a\nb\nc\nd");
    }

    #[test]
    fn clean_text_decodes_entities_after_removing_tags() {
        assert_eq!(clean_text("a &lt; b &amp;&amp; c &gt; d"), "a < b && c > d");
        assert_eq!(clean_text("&amp;lt;"), "&lt;");
        assert_eq!(clean_text("<b>x</b>&nbsp;y"), "x y");
    }

    #[test]
    fn clean_text_collapses_blank_lines() {
        assert_eq!(clean_text("a<br><br><br><br>b"), "a\n\nb");
    }

    #[test]
    fn clean_text_strips_terminal_escapes() {
        assert_eq!(
            clean_text("safe\u{1b}]0;pwned\u{7}text"),
            "safe]0;pwnedtext"
        );
        assert_eq!(sanitize("Mallory\u{1b}[2J"), "Mallory[2J");
    }

    #[test]
    fn wrap_breaks_on_whitespace() {
        assert_eq!(wrap("one two three", 7), vec!["one two", "three"]);
        assert_eq!(wrap("a\nb", 10), vec!["a", "b"]);
    }

    #[test]
    fn wrap_counts_characters_not_bytes() {
        assert_eq!(wrap("héllo wörld", 11), vec!["héllo wörld"]);
    }

    #[test]
    fn wrap_gives_a_long_word_its_own_line() {
        assert_eq!(wrap("a verylongword b", 4), vec!["a", "verylongword", "b"]);
    }

    #[test]
    fn image_label_prefers_alt_then_file_name() {
        assert_eq!(
            image_label(Some("https://x/y/pic.png?a=1"), Some("cat")),
            "cat"
        );
        assert_eq!(
            image_label(Some("https://x/y/pic.png?a=1"), None),
            "pic.png"
        );
        assert_eq!(image_label(Some(r"C:\x\pic.png"), None), "pic.png");
        assert_eq!(image_label(None, None), "Pasted Image");
    }

    #[test]
    fn attachment_icon_uses_type_then_extension() {
        assert_eq!(attachment_icon("x.bin", Some("application/pdf")), "📄");
        assert_eq!(attachment_icon("Report.DOCX", None), "📝");
        assert_eq!(attachment_icon("noext", None), "📎");
        assert_eq!(
            attachment_icon("x.unknown", Some("application/octet-stream")),
            "📎"
        );
    }
}
