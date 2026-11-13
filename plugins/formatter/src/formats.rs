//! JSON and XML pretty-printing; the format is recognized from the text itself.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Json,
    Xml,
}

/// The format a document is written in, from its first character: JSON objects and arrays
/// start with `{` or `[`, XML with `<`. Anything else (including bare JSON values, which
/// have nothing to indent) is not recognized.
pub fn detect(input: &str) -> Option<Format> {
    match input
        .trim_start_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
        .chars()
        .next()?
    {
        '{' | '[' => Some(Format::Json),
        '<' => Some(Format::Xml),
        _ => None,
    }
}

pub fn format(format: Format, input: &str) -> Result<String, String> {
    let result = match format {
        Format::Json => {
            serde_json::from_str::<serde_json::Value>(input.trim_start_matches('\u{feff}'))
                .and_then(|value| serde_json::to_string_pretty(&value))
                .map_err(|e| e.to_string())
        }
        Format::Xml => xml(input.trim_start_matches('\u{feff}')),
    }?;
    if result.len() > 128 * 1024 {
        return Err("128 KiB".into());
    }
    Ok(result)
}

fn xml(input: &str) -> Result<String, String> {
    use quick_xml::{Reader, Writer, events::Event};
    let document = roxmltree::Document::parse(input).map_err(|e| e.to_string())?;
    // Mixed content and explicit whitespace preservation must keep their exact text.
    if document.descendants().any(|node| {
        node.attribute(("http://www.w3.org/XML/1998/namespace", "space")) == Some("preserve")
            || (node.children().any(|child| child.is_element())
                && node
                    .children()
                    .any(|child| child.is_text() && !child.text().unwrap_or("").trim().is_empty()))
    }) {
        return Ok(input.into());
    }
    let mut reader = Reader::from_str(input);
    let mut writer = Writer::new_with_indent(Vec::new(), b' ', 2);
    // Whitespace-only leaf text is meaningful too; only skip whitespace between elements.
    let structural_whitespace: std::collections::HashSet<_> = document
        .descendants()
        .filter(|node| {
            node.is_text()
                && node.text().unwrap_or("").trim().is_empty()
                && node
                    .parent()
                    .is_some_and(|parent| parent.children().any(|child| child.is_element()))
        })
        .map(|node| node.range().start)
        .collect();
    loop {
        let position = reader.buffer_position() as usize;
        let event = reader.read_event().map_err(|e| e.to_string())?;
        match event {
            Event::Eof => break,
            Event::Text(_) if structural_whitespace.contains(&position) => continue,
            event => writer.write_event(event).map_err(|e| e.to_string())?,
        }
    }
    String::from_utf8(writer.into_inner()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_the_format_from_the_first_character() {
        assert_eq!(detect("  \n{\"a\":1}"), Some(Format::Json));
        assert_eq!(detect("[1,2]"), Some(Format::Json));
        assert_eq!(detect("\u{feff}<a/>"), Some(Format::Xml));
        assert_eq!(detect("<?xml version=\"1.0\"?><a/>"), Some(Format::Xml));
        for other in ["", "   ", "json {}", "123", "\"text\"", "a: b"] {
            assert_eq!(detect(other), None, "{other}");
        }
    }

    #[test]
    fn formats_and_rejects_invalid_documents() {
        let json = format(Format::Json, r#"{"a":[1,2]}"#).unwrap();
        assert!(json.contains('\n'));
        assert_eq!(format(Format::Json, &json).unwrap(), json);
        assert_eq!(
            format(
                Format::Json,
                r#"{"z":123456789012345678901234567890,"a":0.12345678901234567890}"#
            )
            .unwrap(),
            "{\n  \"z\": 123456789012345678901234567890,\n  \"a\": 0.12345678901234567890\n}"
        );
        assert!(format(Format::Json, "\u{feff}{}").is_ok());
        assert!(format(Format::Json, "{broken}").is_err());
        assert!(format(Format::Xml, "<a><b></a>").is_err());
        assert!(format(Format::Xml, "<a/><b/>").is_err());
        assert!(
            format(Format::Xml, "<a><b>1</b><c/></a>")
                .unwrap()
                .contains("\n  <b>")
        );
        assert_eq!(
            format(Format::Xml, "<p>Hello <b>world</b> !</p>").unwrap(),
            "<p>Hello <b>world</b> !</p>"
        );
        assert!(
            format(Format::Xml, "<a><b> </b></a>")
                .unwrap()
                .contains("<b> </b>")
        );
    }
}
