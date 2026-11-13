// SPDX-License-Identifier: AGPL-3.0-or-later
//! Expressions in dimensions, such as `thickness`, `width / 2` or
//! `2 * (thickness + 1)`.
//!
//! Numbers, parameter names, `+ - * /`, unary minus and parentheses, by
//! recursive descent. Values are millimetres or degrees as the dimension
//! is; parameter units are not converted.

use std::collections::BTreeMap;

use crate::Parameter;

/// Longest expression accepted, in bytes.
const MAX_LENGTH: usize = 500;
/// Deepest nesting of parentheses accepted.
const MAX_DEPTH: usize = 50;

struct Parser<'a> {
    text: &'a [u8],
    at: usize,
    depth: usize,
    parameters: &'a BTreeMap<String, Parameter>,
}

impl Parser<'_> {
    fn skip_space(&mut self) {
        while self.text.get(self.at).is_some_and(u8::is_ascii_whitespace) {
            self.at += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip_space();
        self.text.get(self.at).copied()
    }

    fn sum(&mut self) -> Result<f64, String> {
        let mut value = self.product()?;
        while let Some(op @ (b'+' | b'-')) = self.peek() {
            self.at += 1;
            let rhs = self.product()?;
            value = if op == b'+' { value + rhs } else { value - rhs };
        }
        Ok(value)
    }

    fn product(&mut self) -> Result<f64, String> {
        let mut value = self.unary()?;
        while let Some(op @ (b'*' | b'/')) = self.peek() {
            self.at += 1;
            let rhs = self.unary()?;
            if op == b'/' {
                if rhs.abs() < 1e-300 {
                    return Err(String::from("it divides by zero"));
                }
                value /= rhs;
            } else {
                value *= rhs;
            }
        }
        Ok(value)
    }

    fn unary(&mut self) -> Result<f64, String> {
        match self.peek() {
            Some(b'-') => {
                self.at += 1;
                Ok(-self.unary()?)
            }
            Some(b'+') => {
                self.at += 1;
                self.unary()
            }
            _ => self.atom(),
        }
    }

    fn atom(&mut self) -> Result<f64, String> {
        match self.peek() {
            Some(b'(') => {
                self.depth += 1;
                if self.depth > MAX_DEPTH {
                    return Err(String::from("it is nested too deeply"));
                }
                self.at += 1;
                let value = self.sum()?;
                if self.peek() != Some(b')') {
                    return Err(String::from("a bracket is not closed"));
                }
                self.at += 1;
                self.depth -= 1;
                Ok(value)
            }
            Some(c) if c.is_ascii_digit() || c == b'.' => {
                let start = self.at;
                while self
                    .text
                    .get(self.at)
                    .is_some_and(|c| c.is_ascii_digit() || *c == b'.')
                {
                    self.at += 1;
                }
                // An exponent such as 1e-3.
                if self
                    .text
                    .get(self.at)
                    .is_some_and(|c| *c == b'e' || *c == b'E')
                {
                    let save = self.at;
                    self.at += 1;
                    if self
                        .text
                        .get(self.at)
                        .is_some_and(|c| *c == b'+' || *c == b'-')
                    {
                        self.at += 1;
                    }
                    if self.text.get(self.at).is_some_and(u8::is_ascii_digit) {
                        while self.text.get(self.at).is_some_and(u8::is_ascii_digit) {
                            self.at += 1;
                        }
                    } else {
                        self.at = save;
                    }
                }
                let word = std::str::from_utf8(self.text.get(start..self.at).unwrap_or_default())
                    .unwrap_or_default();
                word.parse::<f64>()
                    .map_err(|_| format!("{word:?} is not a number"))
            }
            Some(c) if c.is_ascii_alphabetic() => {
                let start = self.at;
                while self
                    .text
                    .get(self.at)
                    .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
                {
                    self.at += 1;
                }
                let name = std::str::from_utf8(self.text.get(start..self.at).unwrap_or_default())
                    .unwrap_or_default();
                self.parameters
                    .get(name)
                    .map(|p| p.value)
                    .ok_or_else(|| format!("there is no parameter named {name}"))
            }
            Some(c) => Err(format!("{:?} is not expected there", char::from(c))),
            None => Err(String::from("it ends too early")),
        }
    }
}

/// Evaluates an expression against the document's parameters.
///
/// # Errors
///
/// A readable reason: an unknown parameter, a syntax mistake, division by
/// zero, or a result that is not a finite number.
pub fn evaluate(text: &str, parameters: &BTreeMap<String, Parameter>) -> Result<f64, String> {
    if text.len() > MAX_LENGTH {
        return Err(String::from("it is too long"));
    }
    let mut parser = Parser {
        text: text.as_bytes(),
        at: 0,
        depth: 0,
        parameters,
    };
    let value = parser.sum()?;
    if let Some(c) = parser.peek() {
        return Err(format!("{:?} is not expected there", char::from(c)));
    }
    if value.is_finite() {
        Ok(value)
    } else {
        Err(String::from("it is not a finite number"))
    }
}

/// The parameter names an expression uses, if it parses.
#[must_use]
pub fn names(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while let Some(&c) = bytes.get(i) {
        if c.is_ascii_alphabetic()
            && (i == 0
                || !bytes
                    .get(i - 1)
                    .is_some_and(|p| p.is_ascii_digit() || *p == b'.'))
        {
            let start = i;
            while bytes
                .get(i)
                .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
            {
                i += 1;
            }
            if let Some(word) = text.get(start..i) {
                out.push(word.to_owned());
            }
        } else {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::float_cmp)]
    use super::*;
    use crate::Unit;

    fn params() -> BTreeMap<String, Parameter> {
        BTreeMap::from([
            (
                "thickness".to_owned(),
                Parameter::new(20.0, Unit::Millimetre),
            ),
            ("width".to_owned(), Parameter::new(96.0, Unit::Millimetre)),
        ])
    }

    #[test]
    fn arithmetic_and_parameters() {
        let p = params();
        for (text, value) in [
            ("12", 12.0),
            ("thickness", 20.0),
            ("width / 2", 48.0),
            ("2 * (thickness + 1)", 42.0),
            ("-thickness + 25", 5.0),
            ("1e1 + .5", 10.5),
            ("width-thickness*2", 56.0),
        ] {
            assert!(
                (evaluate(text, &p).unwrap() - value).abs() < 1e-12,
                "{text}"
            );
        }
        assert_eq!(
            names("2 * (thickness + width) + 1e3"),
            ["thickness", "width"]
        );
    }

    #[test]
    fn mistakes_explain_themselves() {
        let p = params();
        for (text, expected) in [
            ("depth", "no parameter named depth"),
            ("(1 + 2", "not closed"),
            ("1 +", "ends too early"),
            ("2 $ 3", "not expected"),
            ("1 / 0", "divides by zero"),
            ("", "ends too early"),
        ] {
            let error = evaluate(text, &p).unwrap_err();
            assert!(error.contains(expected), "{text}: {error}");
        }
        assert!(
            evaluate(&"(".repeat(60), &p)
                .unwrap_err()
                .contains("nested")
        );
    }
}
