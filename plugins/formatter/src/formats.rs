pub fn format(format: &str, input: &str) -> Result<String, String> {
    let result = match format {
        "json" => serde_json::from_str::<serde_json::Value>(input)
            .and_then(|value| serde_json::to_string_pretty(&value))
            .map_err(|e| e.to_string()),
        "xml" => xml(input),
        _ => Err("json / xml".into()),
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
    fn formats_and_rejects_invalid_documents() {
        let json = format("json", r#"{"a":[1,2]}"#).unwrap();
        assert!(json.contains('\n'));
        assert_eq!(format("json", &json).unwrap(), json);
        assert_eq!(
            format(
                "json",
                r#"{"z":123456789012345678901234567890,"a":0.12345678901234567890}"#
            )
            .unwrap(),
            "{\n  \"z\": 123456789012345678901234567890,\n  \"a\": 0.12345678901234567890\n}"
        );
        assert!(format("json", "{broken}").is_err());
        assert!(format("xml", "<a><b></a>").is_err());
        assert!(format("xml", "<a/><b/>").is_err());
        assert!(
            format("xml", "<a><b>1</b><c/></a>")
                .unwrap()
                .contains("\n  <b>")
        );
        assert_eq!(
            format("xml", "<p>Hello <b>world</b> !</p>").unwrap(),
            "<p>Hello <b>world</b> !</p>"
        );
        assert!(
            format("xml", "<a><b> </b></a>")
                .unwrap()
                .contains("<b> </b>")
        );
        for removed in ["json-min", "yaml", "yml", "toml"] {
            assert!(format(removed, "{}").is_err());
        }
    }
}
