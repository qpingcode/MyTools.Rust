//! `config.json` may carry `//` and `/* */` comments, so the example can document every
//! option. Comments are blanked out (keeping line numbers for parse errors); strings are
//! left untouched, including URLs such as `https://`.
pub fn strip_comments(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    while let Some(ch) = chars.next() {
        if in_string {
            output.push(ch);
            match ch {
                '\\' => output.extend(chars.next()),
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match (ch, chars.peek()) {
            ('"', _) => {
                in_string = true;
                output.push(ch);
            }
            ('/', Some('/')) => {
                for next in chars.by_ref() {
                    if next == '\n' {
                        output.push('\n');
                        break;
                    }
                }
            }
            ('/', Some('*')) => {
                chars.next();
                let mut previous = '\0';
                for next in chars.by_ref() {
                    if next == '\n' {
                        output.push('\n');
                    }
                    if previous == '*' && next == '/' {
                        break;
                    }
                    previous = next;
                }
            }
            _ => output.push(ch),
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_comments_but_not_string_contents() {
        let text = "{\n  // line\n  \"url\": \"https://a//b\", /* block\n */ \"q\": \"say \\\"/*hi*/\\\"\"\n}";
        let value: serde_json::Value = serde_json::from_str(&strip_comments(text)).unwrap();
        assert_eq!(value["url"], "https://a//b");
        assert_eq!(value["q"], "say \"/*hi*/\"");
    }
}
