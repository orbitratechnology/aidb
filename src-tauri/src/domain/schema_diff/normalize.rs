//! Text forms for comparing and writing schema objects: identifier quoting, PostgreSQL's
//! canonical type names, and expressions reduced to tokens so that two spellings of the same
//! default or check compare equal.

use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::tokenizer::{Token, Tokenizer, Word};

/// Keywords PostgreSQL does not accept as plain names in every position (reserved, and those
/// that cannot be function or type names). Names matching one are quoted.
#[rustfmt::skip]
const KEYWORDS: &[&str] = &[
    "all", "analyse", "analyze", "and", "any", "array", "as", "asc", "asymmetric",
    "authorization", "between", "bigint", "binary", "bit", "boolean", "both", "case", "cast",
    "char", "character", "check", "coalesce", "collate", "collation", "column", "concurrently",
    "constraint", "create", "cross", "current_catalog", "current_date", "current_role",
    "current_schema", "current_time", "current_timestamp", "current_user", "dec", "decimal",
    "default", "deferrable", "desc", "distinct", "do", "else", "end", "except", "exists",
    "extract", "false", "fetch", "float", "for", "foreign", "freeze", "from", "full", "grant",
    "greatest", "group", "grouping", "having", "ilike", "in", "initially", "inner", "inout",
    "int", "integer", "intersect", "interval", "into", "is", "isnull", "join", "json",
    "lateral", "leading", "least", "left", "like", "limit", "localtime", "localtimestamp",
    "national", "natural", "nchar", "none", "normalize", "not", "notnull", "null", "nullif",
    "numeric", "offset", "on", "only", "or", "order", "out", "outer", "overlaps", "overlay",
    "placing", "position", "precision", "primary", "real", "references", "returning", "right",
    "row", "select", "session_user", "setof", "similar", "smallint", "some", "substring",
    "symmetric", "system_user", "table", "tablesample", "then", "time", "timestamp", "to",
    "trailing", "treat", "trim", "true", "union", "unique", "user", "using", "values",
    "varchar", "variadic", "verbose", "when", "where", "window", "with", "xmlattributes",
    "xmlconcat", "xmlelement", "xmlexists", "xmlforest", "xmlnamespaces", "xmlparse", "xmlpi",
    "xmlroot", "xmlserialize", "xmltable",
];

/// A name as SQL: bare when PostgreSQL would read it back unchanged, quoted otherwise.
#[must_use]
pub fn quote_ident(name: &str) -> String {
    let plain = name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !KEYWORDS.contains(&name);
    if plain {
        name.to_owned()
    } else {
        format!("\"{}\"", name.replace('"', "\"\""))
    }
}

/// `schema.name`, each part quoted as needed.
#[must_use]
pub fn qualified(schema: &str, name: &str) -> String {
    format!("{}.{}", quote_ident(schema), quote_ident(name))
}

/// A list of names, quoted and joined by commas.
#[must_use]
pub fn name_list(names: &[String]) -> String {
    names
        .iter()
        .map(|name| quote_ident(name))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The type as `format_type` prints it, from any of its spellings (`int4`, `varchar(40)`,
/// `timestamptz`, `decimal(10,2)`, `TEXT[]`, `timestamp(3) with time zone`). Names it does not
/// know are kept, lowercased unless quoted.
#[must_use]
pub fn canonical_type(text: &str) -> String {
    let mut text = text.trim().to_owned();
    let mut arrays = 0;
    loop {
        if let Some(rest) = text.strip_suffix("[]") {
            text = rest.trim_end().to_owned();
        } else if let Some(rest) = strip_suffix_ignore_case(&text, " array") {
            // `integer ARRAY` is the SQL standard spelling of `integer[]`.
            text = rest.trim_end().to_owned();
        } else {
            break;
        }
        arrays += 1;
    }
    // The modifier, wherever it sits: `varchar(40)`, `timestamp(3) with time zone`.
    let (base, modifier) = match (text.find('('), text.find(')')) {
        (Some(open), Some(close)) if open < close => (
            format!("{} {}", &text[..open], &text[close + 1..]),
            Some(
                text[open + 1..close]
                    .split(',')
                    .map(str::trim)
                    .collect::<Vec<_>>()
                    .join(","),
            ),
        ),
        _ => (text.clone(), None),
    };
    let base = if base.trim_start().starts_with('"') {
        base.trim().to_owned()
    } else {
        collapse_spaces(&base.to_lowercase())
    };
    let sized = |name: &str| match &modifier {
        Some(size) => format!("{name}({size})"),
        None => name.to_owned(),
    };
    let zoned = |name: &str, zone: &str| match &modifier {
        Some(precision) => format!("{name}({precision}) {zone}"),
        None => format!("{name} {zone}"),
    };
    let canonical = match base.as_str() {
        "int" | "int4" | "integer" | "serial" | "serial4" => "integer".to_owned(),
        "int2" | "smallint" | "smallserial" | "serial2" => "smallint".to_owned(),
        "int8" | "bigint" | "bigserial" | "serial8" => "bigint".to_owned(),
        "float4" | "real" => "real".to_owned(),
        "float8" | "double precision" | "double" => "double precision".to_owned(),
        "float" => match modifier.as_deref().and_then(|p| p.parse::<u32>().ok()) {
            Some(precision) if precision <= 24 => "real".to_owned(),
            _ => "double precision".to_owned(),
        },
        "bool" | "boolean" => "boolean".to_owned(),
        "varchar" | "character varying" | "char varying" => sized("character varying"),
        "char" | "character" | "bpchar" => {
            format!("character({})", modifier.as_deref().unwrap_or("1"))
        }
        "decimal" | "numeric" => match modifier.as_deref() {
            Some(size) if !size.contains(',') => format!("numeric({size},0)"),
            _ => sized("numeric"),
        },
        "timestamp" | "timestamp without time zone" => zoned("timestamp", "without time zone"),
        "timestamptz" | "timestamp with time zone" => zoned("timestamp", "with time zone"),
        "time" | "time without time zone" => zoned("time", "without time zone"),
        "timetz" | "time with time zone" => zoned("time", "with time zone"),
        "varbit" | "bit varying" => sized("bit varying"),
        "bit" => format!("bit({})", modifier.as_deref().unwrap_or("1")),
        other => sized(other),
    };
    format!("{canonical}{}", "[]".repeat(arrays))
}

/// Whether `text` names a serial type, which stands for an integer column filled from its own
/// sequence.
#[must_use]
pub fn serial_type(text: &str) -> bool {
    matches!(
        text.trim().to_lowercase().as_str(),
        "serial" | "serial4" | "bigserial" | "serial8" | "smallserial" | "serial2"
    )
}

/// The serial spelling of an integer type: `serial`, `bigserial` or `smallserial`.
#[must_use]
pub fn serial_name(data_type: &str) -> Option<&'static str> {
    match data_type {
        "integer" => Some("serial"),
        "bigint" => Some("bigserial"),
        "smallint" => Some("smallserial"),
        _ => None,
    }
}

fn strip_suffix_ignore_case<'a>(text: &'a str, suffix: &str) -> Option<&'a str> {
    let split = text.len().checked_sub(suffix.len())?;
    (text.is_char_boundary(split) && text[split..].eq_ignore_ascii_case(suffix))
        .then(|| &text[..split])
}

fn collapse_spaces(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Words that continue a multi-word type name after `::`.
const TYPE_WORDS: &[&str] = &["varying", "precision", "with", "without", "time", "zone"];

fn tokens(text: &str) -> Vec<Token> {
    Tokenizer::new(&PostgreSqlDialect {}, text)
        .tokenize()
        .unwrap_or_else(|_| vec![Token::make_word(text, None)])
}

fn is_word(token: &Token, value: &str) -> bool {
    matches!(token, Token::Word(word) if word.quote_style.is_none() && word.value.eq_ignore_ascii_case(value))
}

/// Whether `word` names `schema`, as PostgreSQL resolves names (bare names fold to lowercase).
fn names_schema(word: &Word, schema: &str) -> bool {
    match word.quote_style {
        None => word.value.to_lowercase() == schema,
        Some(_) => word.value == schema,
    }
}

/// An expression reduced for comparison: lowercase outside quotes, no casts, no parentheses
/// around single values or the whole, `schema.` dropped for the expression's own schema (also
/// inside `'schema.name'::regclass`), and single spaces between tokens. Two spellings of one
/// expression reduce to the same text; different expressions almost always differ.
#[must_use]
pub fn normalize_expression(text: &str, own_schema: &str) -> String {
    let source: Vec<Token> = tokens(text)
        .into_iter()
        .filter(|token| !matches!(token, Token::Whitespace(_) | Token::EOF))
        .collect();
    let mut out: Vec<Token> = Vec::with_capacity(source.len());
    let mut index = 0;
    while index < source.len() {
        let token = &source[index];
        match token {
            Token::DoubleColon => {
                // `'name'::regclass`: a relation named in a string.
                if source
                    .get(index + 1)
                    .is_some_and(|t| is_word(t, "regclass"))
                    && let Some(Token::SingleQuotedString(name)) = out.last_mut()
                {
                    let prefix = format!("{own_schema}.");
                    if let Some(rest) = name.strip_prefix(&prefix) {
                        *name = rest.to_owned();
                    }
                }
                index = skip_type(&source, index + 1);
                continue;
            }
            Token::Word(word)
                if names_schema(word, own_schema)
                    && source.get(index + 1) == Some(&Token::Period) =>
            {
                index += 2;
                continue;
            }
            Token::Word(word) if word.quote_style.is_none() => {
                out.push(Token::make_word(&word.value.to_lowercase(), None));
            }
            other => out.push(other.clone()),
        }
        index += 1;
    }
    // Parentheses around a single token, then around the whole, until none are left.
    loop {
        let before = out.len();
        let mut reduced = Vec::with_capacity(out.len());
        let mut i = 0;
        while i < out.len() {
            if out[i] == Token::LParen
                && out.get(i + 2) == Some(&Token::RParen)
                && out.get(i + 1).is_some_and(|t| !matches!(t, Token::LParen | Token::RParen))
                // `now()` keeps its parentheses: they belong to the call.
                && !(i > 0 && matches!(out[i - 1], Token::Word(_)))
            {
                reduced.push(out[i + 1].clone());
                i += 3;
                continue;
            }
            reduced.push(out[i].clone());
            i += 1;
        }
        out = remove_double_parentheses(reduced);
        if out.first() == Some(&Token::LParen) && closes_at_end(&out) {
            out = out[1..out.len() - 1].to_vec();
        }
        if out.len() == before {
            break;
        }
    }
    out.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" ")
}

/// `((x))` as `(x)`: a pair of parentheses directly around another pair adds nothing.
fn remove_double_parentheses(tokens: Vec<Token>) -> Vec<Token> {
    let closing = |start: usize| -> Option<usize> {
        let mut depth = 0_i32;
        for (index, token) in tokens.iter().enumerate().skip(start) {
            match token {
                Token::LParen => depth += 1,
                Token::RParen => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(index);
                    }
                }
                _ => {}
            }
        }
        None
    };
    let mut drop = vec![false; tokens.len()];
    for index in 0..tokens.len().saturating_sub(1) {
        // The inner pair goes, so `f((a))` keeps the call's own parentheses.
        if tokens[index] == Token::LParen
            && tokens[index + 1] == Token::LParen
            && let (Some(outer), Some(inner)) = (closing(index), closing(index + 1))
            && outer == inner + 1
        {
            drop[index + 1] = true;
            drop[inner] = true;
        }
    }
    tokens
        .into_iter()
        .zip(drop)
        .filter_map(|(token, dropped)| (!dropped).then_some(token))
        .collect()
}

/// Whether the parenthesis opening `tokens` closes at its last token.
fn closes_at_end(tokens: &[Token]) -> bool {
    let mut depth = 0_i32;
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::LParen => depth += 1,
            Token::RParen => {
                depth -= 1;
                if depth == 0 {
                    return index == tokens.len() - 1;
                }
            }
            _ => {}
        }
    }
    false
}

/// The index after a type name that starts at `start`: words (with `schema.` and multi-word
/// continuations), an optional `(...)` and any `[]`.
fn skip_type(tokens: &[Token], start: usize) -> usize {
    let mut index = start;
    if matches!(tokens.get(index), Some(Token::Word(_))) {
        index += 1;
    } else {
        return index;
    }
    if tokens.get(index) == Some(&Token::Period)
        && matches!(tokens.get(index + 1), Some(Token::Word(_)))
    {
        index += 2;
    }
    while tokens
        .get(index)
        .is_some_and(|t| TYPE_WORDS.iter().any(|word| is_word(t, word)))
    {
        index += 1;
    }
    if tokens.get(index) == Some(&Token::LParen) {
        let mut depth = 0;
        while let Some(token) = tokens.get(index) {
            index += 1;
            match token {
                Token::LParen => depth += 1,
                Token::RParen => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    while tokens.get(index) == Some(&Token::LBracket)
        && tokens.get(index + 1) == Some(&Token::RBracket)
    {
        index += 2;
    }
    index
}

/// `text` with names qualified by schema `from` moved to `to` (also inside
/// `'from.name'::regclass`), written back as SQL. Everything else is kept as written.
#[must_use]
pub fn retarget(text: &str, from: &str, to: &str) -> String {
    if from == to {
        return text.to_owned();
    }
    let source = tokens(text);
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < source.len() {
        match &source[index] {
            Token::Word(word)
                if names_schema(word, from) && source.get(index + 1) == Some(&Token::Period) =>
            {
                out.push_str(&quote_ident(to));
            }
            Token::SingleQuotedString(name)
                if source.get(index + 1) == Some(&Token::DoubleColon)
                    && source
                        .get(index + 2)
                        .is_some_and(|t| is_word(t, "regclass")) =>
            {
                let renamed = name.strip_prefix(&format!("{from}.")).map_or_else(
                    || name.clone(),
                    |rest| format!("{}.{rest}", quote_ident(to)),
                );
                out.push_str(&Token::SingleQuotedString(renamed).to_string());
            }
            Token::EOF => {}
            other => out.push_str(&other.to_string()),
        }
        index += 1;
    }
    out
}

/// `text` without the qualifier of schema `own` on names: how the object reads from inside its
/// schema.
#[must_use]
pub fn unqualify(text: &str, own: &str) -> String {
    let source = tokens(text);
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < source.len() {
        match &source[index] {
            Token::Word(word)
                if names_schema(word, own) && source.get(index + 1) == Some(&Token::Period) =>
            {
                index += 2;
                continue;
            }
            Token::SingleQuotedString(name)
                if source.get(index + 1) == Some(&Token::DoubleColon)
                    && source
                        .get(index + 2)
                        .is_some_and(|t| is_word(t, "regclass")) =>
            {
                let name = name.strip_prefix(&format!("{own}.")).unwrap_or(name);
                out.push_str(&Token::SingleQuotedString(name.to_owned()).to_string());
            }
            Token::EOF => {}
            other => out.push_str(&other.to_string()),
        }
        index += 1;
    }
    out
}

/// The sequence a `nextval('name'::regclass)` default draws from, as written.
#[must_use]
pub fn nextval_sequence(default: &str) -> Option<String> {
    let tokens: Vec<Token> = tokens(default)
        .into_iter()
        .filter(|token| !matches!(token, Token::Whitespace(_) | Token::EOF))
        .collect();
    match tokens.as_slice() {
        [
            Token::Word(call),
            Token::LParen,
            Token::SingleQuotedString(name),
            rest @ ..,
        ] if call.value.eq_ignore_ascii_case("nextval") && rest.last() == Some(&Token::RParen) => {
            Some(name.clone())
        }
        _ => None,
    }
}

/// Whether a default expression gives each row its own value (so adding a column with it
/// rewrites the table): it calls a function PostgreSQL treats as volatile.
#[must_use]
pub fn volatile_default(expression: &str) -> bool {
    const VOLATILE: &[&str] = &[
        "nextval",
        "random",
        "clock_timestamp",
        "timeofday",
        "gen_random_uuid",
        "uuid_generate_v1",
        "uuid_generate_v4",
        "uuidv4",
        "uuidv7",
        "txid_current",
        "statement_timestamp",
        "setseed",
    ];
    tokens(expression).windows(2).any(|pair| {
        matches!(&pair[0], Token::Word(word) if VOLATILE.contains(&word.value.to_lowercase().as_str()))
            && pair[1] == Token::LParen
    })
}

#[cfg(test)]
mod tests;
