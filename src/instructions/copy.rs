// (C) Copyright 2019-2020 Hewlett Packard Enterprise Development LP

use std::convert::TryFrom;
use std::collections::VecDeque;

use snafu::ensure;

use crate::dockerfile_parser::Instruction;
use crate::parser::{Pair, Rule};
use crate::{Span, parse_string};
use crate::SpannedString;
use crate::error::*;

/// A key/value pair passed to a `COPY` instruction as a flag.
///
/// Examples include: `COPY --from=foo /to /from`
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct CopyFlag {
  pub span: Span,
  pub name: SpannedString,
  pub value: SpannedString,
}

impl CopyFlag {
  fn from_record(record: Pair) -> Result<CopyFlag> {
    let span = Span::from_pair(&record);
    let mut name = None;
    let mut value = None;

    for field in record.into_inner() {
      match field.as_rule() {
        Rule::copy_flag_name => name = Some(parse_string(&field)?),
        Rule::copy_flag_value => value = Some(parse_string(&field)?),
        _ => return Err(unexpected_token(field))
      }
    }

    let name = name.ok_or_else(|| Error::GenericParseError {
      message: "copy flags require a key".into(),
    })?;

    let value = value.ok_or_else(|| Error::GenericParseError {
      message: "copy flags require a value".into()
    })?;

    Ok(CopyFlag {
      span, name, value
    })
  }
}

/// A source that is either a filename or the file contents (heredocs)
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum SourceType {
  FileName(SpannedString),
  FileContents(SpannedString),
}

/// A Dockerfile [`COPY` instruction][copy].
///
/// [copy]: https://docs.docker.com/engine/reference/builder/#copy
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct CopyInstruction {
  pub span: Span,
  pub flags: Vec<CopyFlag>,
  pub sources: Vec<SourceType>,
  pub destination: SpannedString
}

impl CopyInstruction {
  pub(crate) fn from_record(record: Pair) -> Result<CopyInstruction> {
    let span = Span::from_pair(&record);
    let mut flags = Vec::new();
    let mut destination = SpannedString { span: Span::new(0, 0), content: String::new() };

    let mut inner = record.into_inner();
    let field = inner.next().ok_or_else(|| Error::GenericParseError {
      message: "Copy instruction expected a field".into(),
    })?;
    
    match field.as_rule() {
      Rule::copy_standard => {
        let mut paths = Vec::new();
        for inner in field.into_inner() {
          match inner.as_rule() {
            Rule::copy_flag => flags.push(CopyFlag::from_record(inner)?),
            Rule::copy_pathspec => paths.push(parse_string(&inner)?),
            Rule::comment => continue,
            _ => return Err(unexpected_token(inner))
          }
        }
        ensure!(
          paths.len() >= 2,
          GenericParseError {
            message: "copy requires at least one source and a destination"
          }
        );
        destination = paths.pop().unwrap();
        Ok(CopyInstruction {
          span,
          flags,
          sources: paths.into_iter().map(SourceType::FileName).collect(),
          destination
        })
      },
      Rule::copy_heredoc => {
        let mut sources = Vec::new();
        let mut delimiters = VecDeque::new();
        let mut terminators = Vec::new();
        for inner in field.into_inner() {
          match inner.as_rule() {
            Rule::heredoc_delim => delimiters.push_back(parse_string(&inner)?),
            Rule::copy_flag => flags.push(CopyFlag::from_record(inner)?),
            Rule::copy_pathspec => destination = parse_string(&inner)?,
            Rule::heredoc_body => sources.push(parse_string(&inner)?),
            Rule::heredoc_terminator => {
              let terminator = parse_string(&inner)?;
              let expected_delimiter = delimiters.pop_front().ok_or_else(|| Error::GenericParseError {
                message: "Unexpected heredoc terminator without matching delimiter".into()
              })?;
              
              ensure!(
                (expected_delimiter.content.clone() + "\n") == terminator.content,
                GenericParseError {
                  message: "Invalid heredoc in copy instruction"
                }
              );
              terminators.push(terminator);
            },
            _ => return Err(unexpected_token(inner))
          }
        }
        ensure!(
          delimiters.is_empty(),
          GenericParseError {
            message: "Unmatched heredoc delimiters in copy instruction"
          }
        );
        ensure!(
          sources.len() >= 1,
          GenericParseError {
            message: "copy requires at least one source and a destination"
          }
        );
        Ok(CopyInstruction {
          span,
          flags,
          sources: sources.into_iter().map(SourceType::FileContents).collect(),
          destination
        })
      },
      _ => return Err(unexpected_token(field))
    }
  }
}

impl<'a> TryFrom<&'a Instruction> for &'a CopyInstruction {
  type Error = Error;

  fn try_from(instruction: &'a Instruction) -> std::result::Result<Self, Self::Error> {
    if let Instruction::Copy(c) = instruction {
      Ok(c)
    } else {
      Err(Error::ConversionError {
        from: format!("{:?}", instruction),
        to: "CopyInstruction".into()
      })
    }
  }
}

#[cfg(test)]
mod tests {
  use indoc::indoc;
  use pretty_assertions::assert_eq;

  use super::*;
  use crate::test_util::*;

  #[test]
  fn copy_basic() -> Result<()> {
    assert_eq!(
      parse_single("copy foo bar", Rule::copy)?,
      CopyInstruction {
        span: Span { start: 0, end: 12 },
        flags: vec![],
        sources: vec![SourceType::FileName(SpannedString {
          span: Span::new(5, 8),
          content: "foo".to_string()
        })],
        destination: SpannedString {
          span: Span::new(9, 12),
          content: "bar".to_string()
        },
      }.into()
    );

    Ok(())
  }

  #[test]
  fn copy_multiple_sources() -> Result<()> {
    assert_eq!(
      parse_single("copy foo bar baz qux", Rule::copy)?,
      CopyInstruction {
        span: Span { start: 0, end: 20 },
        flags: vec![],
        sources: vec![
          SourceType::FileName(SpannedString {
            span: Span::new(5, 8),
            content: "foo".to_string(),
          }),
          SourceType::FileName(SpannedString {
            span: Span::new(9, 12),
            content: "bar".to_string()
          }),
          SourceType::FileName(SpannedString {
            span: Span::new(13, 16),
            content: "baz".to_string()
          })
        ],
        destination: SpannedString {
          span: Span::new(17, 20),
          content: "qux".to_string()
        },
      }.into()
    );

    Ok(())
  }

  #[test]
  fn copy_multiline() -> Result<()> {
    // multiline is okay; whitespace on the next line is optional
    assert_eq!(
      parse_single("copy foo \\\nbar", Rule::copy)?,
      CopyInstruction {
        span: Span { start: 0, end: 14 },
        flags: vec![],
        sources: vec![SourceType::FileName(SpannedString {
          span: Span::new(5, 8),
          content: "foo".to_string(),
        })],
        destination: SpannedString {
          span: Span::new(11, 14),
          content: "bar".to_string(),
        },
      }.into()
    );

    // newlines must be escaped
    assert_eq!(
      parse_single("copy foo\nbar", Rule::copy).is_err(),
      true
    );

    Ok(())
  }

  #[test]
  fn copy_flags() -> Result<()> {
    assert_eq!(
      parse_single(
        "copy --from=alpine:3.10 /usr/lib/libssl.so.1.1 /tmp/",
        Rule::copy
      )?,
      CopyInstruction {
        span: Span { start: 0, end: 52 },
        flags: vec![
          CopyFlag {
            span: Span { start: 5, end: 23 },
            name: SpannedString {
              content: "from".into(),
              span: Span { start: 7, end: 11 },
            },
            value: SpannedString {
              content: "alpine:3.10".into(),
              span: Span { start: 12, end: 23 },
            }
          }
        ],
        sources: vec![SourceType::FileName(SpannedString {
          span: Span::new(24, 46),
          content: "/usr/lib/libssl.so.1.1".to_string(),
        })],
        destination: SpannedString {
          span: Span::new(47, 52),
          content: "/tmp/".into(),
        }
      }.into()
    );

    Ok(())
  }

  #[test]
  fn copy_comments() -> Result<()> {
    assert_eq!(
      parse_single(
        indoc!(r#"
          copy \
            --from=alpine:3.10 \

            # hello

            /usr/lib/libssl.so.1.1 \
            # world
            /tmp/
        "#),
        Rule::copy
      )?.into_copy().unwrap(),
      CopyInstruction {
        span: Span { start: 0, end: 86 },
        flags: vec![
          CopyFlag {
            span: Span { start: 9, end: 27 },
            name: SpannedString {
              span: Span { start: 11, end: 15 },
              content: "from".into(),
            },
            value: SpannedString {
              span: Span { start: 16, end: 27 },
              content: "alpine:3.10".into(),
            },
          }
        ],
        sources: vec![SourceType::FileName(SpannedString {
          span: Span::new(44, 66),
          content: "/usr/lib/libssl.so.1.1".to_string(),
        })],
        destination: SpannedString {
          span: Span::new(81, 86),
          content: "/tmp/".into(),
        },
      }.into()
    );

    Ok(())
  }

  #[test]
  fn copy_heredoc() -> Result<()> {
    assert_eq!(
      parse_single(
        indoc!(r#"
          COPY <<EOF /usr/share/nginx/html/index.html
          <!DOCTYPE html>
          <html>
          <head>
              <title>Welcome to nginx!</title>
          </head>
          <body>
              <h1>Welcome to nginx!</h1>
          </body>
          </html>
          EOF
        "#),
        Rule::copy
      )?.into_copy().unwrap(),
      CopyInstruction {
        span: Span { start: 0, end: 177 },
        flags: vec![],
        sources: vec![SourceType::FileContents(SpannedString {
          span: Span::new(44, 173),
          content: indoc!(r#"
          <!DOCTYPE html>
          <html>
          <head>
              <title>Welcome to nginx!</title>
          </head>
          <body>
              <h1>Welcome to nginx!</h1>
          </body>
          </html>
          "#).to_string(),
        })],
        destination: SpannedString {
          span: Span::new(11, 43),
          content: "/usr/share/nginx/html/index.html".to_string(),
        },
      }.into()
    );

    Ok(())
  }
  
  #[test]
  fn copy_heredoc_simple() -> Result<()> {
    assert_eq!(
      parse_single(
        indoc!(r#"
          COPY <<EOF /tmp/test.txt
          hello
          EOF
        "#),
        Rule::copy
      )?.into_copy().unwrap(),
      CopyInstruction {
        span: Span { start: 0, end: 35 },
        flags: vec![],
        sources: vec![SourceType::FileContents(SpannedString {
          span: Span::new(25, 31),
          content: "hello\n".to_string(),
        })],
        destination: SpannedString {
          span: Span::new(11, 24),
          content: "/tmp/test.txt".to_string(),
        },
      }.into()
    );

    Ok(())
  }

  #[test]
  fn copy_heredoc_incorrect() -> Result<()> {
    assert!(parse_single(
      indoc!(r#"
        COPY <<EOF /usr/share/nginx/html/index.html
        <!DOCTYPE html>
        <html>
        <head>
            <title>Welcome to nginx!</title>
        </head>
        <body>
            <h1>Welcome to nginx!</h1>
        </body>
        </html>
        WRONGTERMINATOR
      "#),
      Rule::copy
    ).is_err());

    Ok(())
  }

  #[test]
  fn copy_multi_heredoc() -> Result<()> {
    assert_eq!(
      parse_single(
        indoc!(r#"
          COPY <<EOF <<EOF2 /usr/share/nginx/html/index.html
          <!DOCTYPE html>
          <html>
          <head>
              <title>Welcome to nginx!</title>
          </head>
          <body>
              <h1>Welcome to nginx!</h1>
          </body>
          </html>
          EOF
          <!DOCTYPE html>
          <html>
          <head>
              <title>Welcome to nginx!</title>
          </head>
          <body>
              <h1>Welcome to nginx!</h1>
          </body>
          </html>
          EOF2
        "#),
        Rule::copy
      )?.into_copy().unwrap(),
      CopyInstruction {
        span: Span { start: 0, end: 318 },
        flags: vec![],
        sources: vec![SourceType::FileContents(SpannedString {
          span: Span::new(51, 180),
          content: indoc!(r#"
          <!DOCTYPE html>
          <html>
          <head>
              <title>Welcome to nginx!</title>
          </head>
          <body>
              <h1>Welcome to nginx!</h1>
          </body>
          </html>
          "#).to_string(),
        }), 
        SourceType::FileContents(SpannedString {
          span: Span::new(184, 313),
          content: indoc!(r#"
          <!DOCTYPE html>
          <html>
          <head>
              <title>Welcome to nginx!</title>
          </head>
          <body>
              <h1>Welcome to nginx!</h1>
          </body>
          </html>
          "#).to_string(),
        })],
        destination: SpannedString {
          span: Span::new(18, 50),
          content: "/usr/share/nginx/html/index.html".to_string(),
        },
      }.into()
    );

    Ok(())
  }
}
