use crate::parser::{Cursor, Parse, Parser, Peek, Result};
use crate::token::Index;
use alloc::vec::Vec;

fn peek<K: Peek>(cursor: Cursor) -> Result<bool> {
    // This is a little fancy because when parsing something like:
    //
    //      (type (component (type $foo)))
    //
    // we need to disambiguate that from
    //
    //      (type (component (type $foo (func))))
    //
    // where the first is a type reference and the second is an inline
    // component type defining a type internally. The peek here not only
    // peeks for `K` but also for the index and possibly trailing
    // strings.

    // Peek for the given keyword type
    if !K::peek(cursor)? {
        return Ok(false);
    }

    // Move past the given keyword
    let cursor = match cursor.keyword()? {
        Some((_, c)) => c,
        _ => return Ok(false),
    };

    // Peek an id or integer index, followed by `)` or string to disambiguate
    let cursor = match cursor.id()? {
        Some((_, cursor)) => Some(cursor),
        None => cursor.integer()?.map(|p| p.1),
    };
    Ok(match cursor {
        Some(cursor) => cursor.rparen()?.is_some() || cursor.string()?.is_some(),
        None => false,
    })
}

/// Parses core item references.
#[derive(Clone, Debug)]
pub struct CoreItemRef<'a, K> {
    /// The item kind being parsed.
    pub kind: K,
    /// The item or instance reference.
    pub idx: Index<'a>,
    /// Export name to resolve the item from.
    pub export_name: Option<&'a str>,
}

impl<'a, K: Parse<'a>> Parse<'a> for CoreItemRef<'a, K> {
    fn parse(parser: Parser<'a>) -> Result<Self> {
        // This does not parse the surrounding `(` and `)` because
        // core prefix is context dependent and only the caller knows if it should be
        // present for core references; therefore, the caller parses the parens and any core prefix
        let kind = parser.parse::<K>()?;
        let idx = parser.parse()?;
        let export_name = parser.parse()?;
        Ok(Self {
            kind,
            idx,
            export_name,
        })
    }
}

impl<K: Peek> Peek for CoreItemRef<'_, K> {
    fn peek(cursor: Cursor<'_>) -> Result<bool> {
        peek::<K>(cursor)
    }

    fn display() -> &'static str {
        "a core item reference"
    }
}

/// Parses component item references.
#[derive(Clone, Debug)]
pub struct ItemRef<'a, K> {
    /// The item kind being parsed.
    pub kind: K,
    /// The item or instance reference.
    pub idx: Index<'a>,
    /// Export names to resolve the item from.
    pub export_names: Vec<&'a str>,
}

impl<'a, K: Parse<'a>> Parse<'a> for ItemRef<'a, K> {
    fn parse(parser: Parser<'a>) -> Result<Self> {
        let kind = parser.parse::<K>()?;
        let idx = parser.parse()?;
        let mut export_names = Vec::new();
        while !parser.is_empty() {
            export_names.push(parser.parse()?);
        }
        Ok(Self {
            kind,
            idx,
            export_names,
        })
    }
}

impl<K: Peek> Peek for ItemRef<'_, K> {
    fn peek(cursor: Cursor<'_>) -> Result<bool> {
        peek::<K>(cursor)
    }

    fn display() -> &'static str {
        "a component item reference"
    }
}

/// Convenience structure to parse `$f` or `(item $f)`.
#[derive(Clone, Debug)]
pub struct IndexOrRef<'a, K>(pub ItemRef<'a, K>);

impl<'a, K> Parse<'a> for IndexOrRef<'a, K>
where
    K: Parse<'a> + Default,
{
    fn parse(parser: Parser<'a>) -> Result<Self> {
        if parser.peek::<Index<'_>>()? {
            Ok(IndexOrRef(ItemRef {
                kind: K::default(),
                idx: parser.parse()?,
                export_names: Vec::new(),
            }))
        } else {
            Ok(IndexOrRef(parser.parens(|p| p.parse())?))
        }
    }
}

/// Convenience structure to parse `$f` or `(item $f)`.
#[derive(Clone, Debug)]
pub struct IndexOrCoreRef<'a, K>(pub CoreItemRef<'a, K>);

impl<'a, K> Parse<'a> for IndexOrCoreRef<'a, K>
where
    K: Parse<'a> + Default,
{
    fn parse(parser: Parser<'a>) -> Result<Self> {
        if parser.peek::<Index<'_>>()? {
            Ok(IndexOrCoreRef(CoreItemRef {
                kind: K::default(),
                idx: parser.parse()?,
                export_name: None,
            }))
        } else {
            Ok(IndexOrCoreRef(parser.parens(|p| p.parse())?))
        }
    }
}
