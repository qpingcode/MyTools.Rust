/// Evaluate the complete prefix while the user is typing trailing operators/open brackets.
/// Keep strict evaluation separate so invalid syntax and division by zero never fall back.
pub fn preview(input: &str) -> Option<(&str, f64)> {
    if input.len() > 512 {
        return None;
    }
    let input = input.trim();
    if let Some(value) = evaluate(input) {
        return Some((input, value));
    }
    let prefix = input.trim_end_matches(|ch: char| {
        ch.is_ascii_whitespace() || matches!(ch, '+' | '-' | '*' | '/' | '(')
    });
    if prefix == input {
        return None;
    }
    evaluate(prefix).map(|value| (prefix, value))
}

/// Small arithmetic parser: + - * /, parentheses, signs and decimal numbers.
pub fn evaluate(input: &str) -> Option<f64> {
    // Bound recursion and work for malformed or very long search input.
    if input.trim().is_empty() || input.len() > 512 {
        return None;
    }
    let mut parser = Parser {
        input: input.as_bytes(),
        position: 0,
    };
    let result = parser.sum()?;
    parser.skip_spaces();
    (parser.position == parser.input.len() && result.is_finite()).then_some(result)
}

struct Parser<'a> {
    input: &'a [u8],
    position: usize,
}

impl Parser<'_> {
    fn skip_spaces(&mut self) {
        while self
            .input
            .get(self.position)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.position += 1;
        }
    }

    fn consume(&mut self, byte: u8) -> bool {
        self.skip_spaces();
        if self.input.get(self.position) == Some(&byte) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn sum(&mut self) -> Option<f64> {
        let mut value = self.product()?;
        loop {
            if self.consume(b'+') {
                value += self.product()?;
            } else if self.consume(b'-') {
                value -= self.product()?;
            } else {
                return Some(value);
            }
        }
    }

    fn product(&mut self) -> Option<f64> {
        let mut value = self.factor()?;
        loop {
            if self.consume(b'*') {
                value *= self.factor()?;
            } else if self.consume(b'/') {
                let divisor = self.factor()?;
                if divisor == 0.0 {
                    return None;
                }
                value /= divisor;
            } else {
                return Some(value);
            }
        }
    }

    fn factor(&mut self) -> Option<f64> {
        if self.consume(b'+') {
            return self.factor();
        }
        if self.consume(b'-') {
            return Some(-self.factor()?);
        }
        if self.consume(b'(') {
            let value = self.sum()?;
            return self.consume(b')').then_some(value);
        }
        self.skip_spaces();
        let start = self.position;
        while self
            .input
            .get(self.position)
            .is_some_and(|byte| byte.is_ascii_digit() || *byte == b'.')
        {
            self.position += 1;
        }
        std::str::from_utf8(&self.input[start..self.position])
            .ok()?
            .parse()
            .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_complete_prefixes_without_hiding_invalid_expressions() {
        for (query, prefix, expected) in [
            ("32-1-", "32-1", 31.0),
            ("10+", "10", 10.0),
            ("5*5*", "5*5", 25.0),
            ("5+6*(", "5+6", 11.0),
            (" 5+6 * ( ", "5+6", 11.0),
            ("(2+3)*", "(2+3)", 5.0),
        ] {
            assert_eq!(preview(query), Some((prefix.trim(), expected)), "{query}");
        }
        for query in [
            "", "(", "+-", "1/0+", "2**3*", "hello+", "1..2+", "1 2+", "1+2)",
        ] {
            assert!(preview(query).is_none(), "{query}");
        }
        assert!(preview(&"(".repeat(513)).is_none());
    }

    #[test]
    fn evaluates_arithmetic() {
        for (query, expected) in [
            ("1+1", 2.0),
            ("2+3*4", 14.0),
            ("(2+3)*4", 20.0),
            ("-2 * -3", 6.0),
            ("1.5 / .5", 3.0),
            (" 42 ", 42.0),
        ] {
            assert_eq!(evaluate(query), Some(expected), "{query}");
        }
    }

    #[test]
    fn ignores_invalid_or_incomplete_expressions() {
        for query in ["", "1+", "hello", "1/0", "(1+2", "1..2", "1 2", "2**3"] {
            assert_eq!(evaluate(query), None, "{query}");
        }
        assert_eq!(evaluate(&"(".repeat(513)), None);
    }
}
