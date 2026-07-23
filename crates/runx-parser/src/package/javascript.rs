use super::SkillPackageError;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Ident(String),
    String { value: String, escaped: bool },
    Punct(char),
}

pub(super) fn module_imports(path: &str, source: &str) -> Result<Vec<String>, SkillPackageError> {
    let tokens = tokenize(path, source)?;
    reject_effectful_module_tokens(path, &tokens)?;
    imports_from_tokens(path, &tokens)
}

/// Collect static module dependencies for a process-backed JavaScript tool.
/// Unlike deterministic modules, CLI tools may use process and Node APIs; the
/// package validator still needs the complete static import closure.
pub(super) fn process_module_imports(
    path: &str,
    source: &str,
) -> Result<Vec<String>, SkillPackageError> {
    let tokens = tokenize(path, source)?;
    let mut imports = imports_from_tokens(path, &tokens)?;
    collect_static_requires(path, &tokens, &mut imports)?;
    imports.sort();
    imports.dedup();
    Ok(imports)
}

fn collect_static_requires(
    path: &str,
    tokens: &[Token],
    imports: &mut Vec<String>,
) -> Result<(), SkillPackageError> {
    for (index, token) in tokens.iter().enumerate() {
        if !matches!(token, Token::Ident(identifier) if identifier == "require")
            || matches!(tokens.get(index.wrapping_sub(1)), Some(Token::Punct('.')))
            || !matches!(tokens.get(index + 1), Some(Token::Punct('(')))
        {
            continue;
        }
        match (tokens.get(index + 2), tokens.get(index + 3)) {
            (
                Some(Token::String {
                    value,
                    escaped: false,
                }),
                Some(Token::Punct(')')),
            ) => imports.push(value.clone()),
            (Some(Token::String { escaped: true, .. }), _) => {
                return Err(SkillPackageError::invalid(
                    path,
                    "CommonJS require specifiers must not use string escapes",
                ));
            }
            _ => {
                return Err(SkillPackageError::invalid(
                    path,
                    "process-backed JavaScript may use only static require(\"specifier\") dependencies",
                ));
            }
        }
    }
    Ok(())
}

fn imports_from_tokens(path: &str, tokens: &[Token]) -> Result<Vec<String>, SkillPackageError> {
    let mut imports = Vec::new();
    let mut index = 0usize;
    while index < tokens.len() {
        match tokens.get(index) {
            Some(Token::Ident(keyword)) if keyword == "import" => {
                if matches!(tokens.get(index.wrapping_sub(1)), Some(Token::Punct('.'))) {
                    index += 1;
                    continue;
                }
                index = parse_import(path, tokens, index, &mut imports)?;
            }
            Some(Token::Ident(keyword)) if keyword == "export" => {
                index = parse_export(path, tokens, index, &mut imports)?;
            }
            _ => index += 1,
        }
    }
    imports.sort();
    imports.dedup();
    Ok(imports)
}

fn reject_effectful_module_tokens(path: &str, tokens: &[Token]) -> Result<(), SkillPackageError> {
    for (index, token) in tokens.iter().enumerate() {
        let Token::Ident(identifier) = token else {
            continue;
        };
        let next = tokens.get(index + 1);
        if matches!(identifier.as_str(), "fetch" | "require")
            && matches!(next, Some(Token::Punct('(')))
        {
            return Err(effectful_module_error(path, identifier));
        }
        if matches!(identifier.as_str(), "RUNX_INPUTS_JSON" | "RUNX_INPUTS_PATH") {
            return Err(effectful_module_error(path, identifier));
        }
        if identifier == "process"
            && matches!(next, Some(Token::Punct('.')))
            && matches!(
                tokens.get(index + 2),
                Some(Token::Ident(field)) if matches!(field.as_str(), "env" | "stdout" | "stderr")
            )
        {
            return Err(effectful_module_error(path, "process runtime plumbing"));
        }
    }
    Ok(())
}

fn effectful_module_error(path: &str, boundary: &str) -> SkillPackageError {
    SkillPackageError::invalid(
        path,
        format!(
            "deterministic JavaScript modules cannot own {boundary}; compose a native tool or declare a cli-tool boundary"
        ),
    )
}

fn parse_import(
    path: &str,
    tokens: &[Token],
    index: usize,
    imports: &mut Vec<String>,
) -> Result<usize, SkillPackageError> {
    match tokens.get(index + 1) {
        Some(Token::Punct('(')) => Err(SkillPackageError::invalid(
            path,
            "dynamic import() is not available in deterministic JavaScript modules",
        )),
        Some(Token::String {
            value,
            escaped: false,
        }) => {
            imports.push(value.clone());
            Ok(index + 2)
        }
        Some(Token::String { escaped: true, .. }) => Err(SkillPackageError::invalid(
            path,
            "JavaScript module specifiers must not use string escapes",
        )),
        _ => parse_from_clause(path, tokens, index + 1, imports),
    }
}

fn parse_export(
    path: &str,
    tokens: &[Token],
    index: usize,
    imports: &mut Vec<String>,
) -> Result<usize, SkillPackageError> {
    match tokens.get(index + 1) {
        Some(Token::Punct('{' | '*')) => parse_from_clause(path, tokens, index + 1, imports),
        _ => Ok(index + 1),
    }
}

fn parse_from_clause(
    path: &str,
    tokens: &[Token],
    start: usize,
    imports: &mut Vec<String>,
) -> Result<usize, SkillPackageError> {
    let limit = start.saturating_add(64).min(tokens.len());
    let mut index = start;
    while index < limit {
        match tokens.get(index) {
            Some(Token::Punct(';')) => return Ok(index + 1),
            Some(Token::Ident(value)) if value == "from" => match tokens.get(index + 1) {
                Some(Token::String {
                    value,
                    escaped: false,
                }) => {
                    imports.push(value.clone());
                    return Ok(index + 2);
                }
                Some(Token::String { escaped: true, .. }) => {
                    return Err(SkillPackageError::invalid(
                        path,
                        "JavaScript module specifiers must not use string escapes",
                    ));
                }
                _ => {
                    return Err(SkillPackageError::invalid(
                        path,
                        "JavaScript import/export from must be followed by a plain string literal",
                    ));
                }
            },
            Some(Token::Ident(value)) if matches!(value.as_str(), "import" | "export") => {
                return Ok(index);
            }
            _ => index += 1,
        }
    }
    Ok(index.max(start + 1))
}

fn tokenize(path: &str, source: &str) -> Result<Vec<Token>, SkillPackageError> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'#' if index == 0 && bytes.get(1) == Some(&b'!') => {
                index = 2;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            byte if byte.is_ascii_whitespace() => index += 1,
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                index += 2;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                let start = index;
                index += 2;
                while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/')
                {
                    index += 1;
                }
                if index + 1 >= bytes.len() {
                    return Err(SkillPackageError::invalid(
                        path,
                        format!("unterminated JavaScript block comment at byte {start}"),
                    ));
                }
                index += 2;
            }
            b'/' if regex_can_start_after(tokens.last()) => {
                index = skip_regex(path, bytes, index)?;
            }
            quote @ (b'\'' | b'\"') => {
                let (token, next) = string_token(path, bytes, index, quote)?;
                tokens.push(token);
                index = next;
            }
            b'`' => index = skip_template(path, bytes, index)?,
            byte if is_ident_start(byte) => {
                let start = index;
                index += 1;
                while index < bytes.len() && is_ident_continue(bytes[index]) {
                    index += 1;
                }
                let value = std::str::from_utf8(&bytes[start..index]).map_err(|error| {
                    SkillPackageError::invalid(path, format!("invalid UTF-8 identifier: {error}"))
                })?;
                tokens.push(Token::Ident(value.to_owned()));
            }
            byte => {
                tokens.push(Token::Punct(char::from(byte)));
                index += 1;
            }
        }
    }
    Ok(tokens)
}

fn regex_can_start_after(previous: Option<&Token>) -> bool {
    match previous {
        None => true,
        Some(Token::Punct(character)) => REGEX_PREFIX_PUNCTUATION.contains(character),
        Some(Token::Ident(keyword)) => REGEX_PREFIX_KEYWORDS.contains(&keyword.as_str()),
        Some(Token::String { .. }) => false,
    }
}

const REGEX_PREFIX_PUNCTUATION: &[char] = &[
    '(', '[', '{', ',', ';', ':', '=', '!', '?', '&', '|', '+', '-', '*', '%', '^', '~', '<', '>',
];

const REGEX_PREFIX_KEYWORDS: &[&str] = &[
    "await",
    "case",
    "delete",
    "do",
    "else",
    "in",
    "instanceof",
    "new",
    "of",
    "return",
    "throw",
    "typeof",
    "void",
    "yield",
];

fn skip_regex(path: &str, bytes: &[u8], start: usize) -> Result<usize, SkillPackageError> {
    let mut index = start + 1;
    let mut in_character_class = false;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index = index.saturating_add(2),
            b'[' if !in_character_class => {
                in_character_class = true;
                index += 1;
            }
            b']' if in_character_class => {
                in_character_class = false;
                index += 1;
            }
            b'/' if !in_character_class => {
                index += 1;
                while index < bytes.len() && is_ident_continue(bytes[index]) {
                    index += 1;
                }
                return Ok(index);
            }
            b'\n' | b'\r' => {
                return Err(SkillPackageError::invalid(
                    path,
                    format!("unterminated JavaScript regular expression at byte {start}"),
                ));
            }
            _ => index += 1,
        }
    }
    Err(SkillPackageError::invalid(
        path,
        format!("unterminated JavaScript regular expression at byte {start}"),
    ))
}

fn string_token(
    path: &str,
    bytes: &[u8],
    start: usize,
    quote: u8,
) -> Result<(Token, usize), SkillPackageError> {
    let mut index = start + 1;
    let mut escaped = false;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => {
                escaped = true;
                index = index.saturating_add(2);
            }
            byte if byte == quote => {
                let value = std::str::from_utf8(&bytes[start + 1..index]).map_err(|error| {
                    SkillPackageError::invalid(path, format!("invalid UTF-8 string: {error}"))
                })?;
                return Ok((
                    Token::String {
                        value: value.to_owned(),
                        escaped,
                    },
                    index + 1,
                ));
            }
            b'\n' | b'\r' => {
                return Err(SkillPackageError::invalid(
                    path,
                    format!("unterminated JavaScript string at byte {start}"),
                ));
            }
            _ => index += 1,
        }
    }
    Err(SkillPackageError::invalid(
        path,
        format!("unterminated JavaScript string at byte {start}"),
    ))
}

fn skip_template(path: &str, bytes: &[u8], start: usize) -> Result<usize, SkillPackageError> {
    let mut index = start + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index = index.saturating_add(2),
            b'`' => return Ok(index + 1),
            _ => index += 1,
        }
    }
    Err(SkillPackageError::invalid(
        path,
        format!("unterminated JavaScript template literal at byte {start}"),
    ))
}

const fn is_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || matches!(byte, b'_' | b'$')
}

const fn is_ident_continue(byte: u8) -> bool {
    is_ident_start(byte) || byte.is_ascii_digit()
}

#[cfg(test)]
mod tests {
    use super::{module_imports, process_module_imports};

    #[test]
    fn finds_static_and_literal_dynamic_imports_without_reading_comments_or_strings() {
        let source = r#"
            // import "./ignored.mjs";
            const prose = "import './also-ignored.mjs'";
            import { one } from "./one.mjs";
            export { two } from './two.js';
        "#;
        assert_eq!(
            module_imports("domain/main.mjs", source),
            Ok(vec!["./one.mjs".to_owned(), "./two.js".to_owned(),])
        );
    }

    #[test]
    fn rejects_dynamic_import_even_with_a_literal_specifier() {
        let error = module_imports("domain/main.mjs", "import('./other.mjs')")
            .err()
            .map(|error| error.to_string());
        assert!(error.is_some_and(|message| message.contains("dynamic import()")));
    }

    #[test]
    fn process_import_scan_does_not_treat_exported_function_bodies_as_re_exports() {
        let source = r#"
            import fs from "node:fs";
            export function createStore(from) {
                return from ?? fs;
            }
        "#;

        assert_eq!(
            process_module_imports("tools/data/store/run.mjs", source),
            Ok(vec!["node:fs".to_owned()])
        );
    }

    #[test]
    fn process_import_scan_accepts_node_shebangs() {
        assert_eq!(
            process_module_imports(
                "tools/provider/action/run.mjs",
                "#!/usr/bin/env node\nimport fs from 'node:fs';\n",
            ),
            Ok(vec!["node:fs".to_owned()])
        );
    }

    #[test]
    fn process_import_scan_collects_static_commonjs_dependencies() {
        assert_eq!(
            process_module_imports(
                "tools/provider/action/run.cjs",
                "const helper = require('./helper.cjs');\n",
            ),
            Ok(vec!["./helper.cjs".to_owned()])
        );
    }

    #[test]
    fn process_import_scan_rejects_dynamic_commonjs_dependencies() {
        let error = process_module_imports(
            "tools/provider/action/run.cjs",
            "const helper = require(process.env.HELPER);\n",
        )
        .err()
        .map(|error| error.to_string());

        assert!(error.is_some_and(|message| message.contains("only static require")));
    }

    #[test]
    fn ignores_quotes_and_import_words_inside_regular_expressions() {
        let source = r#"
            const segment = value.split(/\s+/)[0]?.replace(/[^a-zA-Z'-]/g, "");
            const field = value.replace(/^['"]|['"]$/gu, "");
            const marker = /import\(['"]ignored['"]\)/u;
            import actual from "./actual.mjs";
        "#;

        assert_eq!(
            module_imports("domain/main.mjs", source),
            Ok(vec!["./actual.mjs".to_owned()])
        );
    }

    #[test]
    fn rejects_effect_and_process_plumbing_outside_literals() {
        for (source, boundary) in [
            ("export default () => fetch('/data');", "fetch"),
            ("export default () => require('node:fs');", "require"),
            (
                "export default () => process.env.TOKEN;",
                "process runtime plumbing",
            ),
            ("export default () => RUNX_INPUTS_JSON;", "RUNX_INPUTS_JSON"),
        ] {
            let error = module_imports("domain/main.mjs", source)
                .err()
                .map(|error| error.to_string());
            assert!(
                error
                    .as_deref()
                    .is_some_and(|message| message.contains(boundary)),
                "unexpected result: {error:?}"
            );
        }
        assert!(
            module_imports(
                "domain/main.mjs",
                "export default () => 'fetch() process.env RUNX_INPUTS_JSON';",
            )
            .is_ok(),
            "effect-like text inside a string is data, not executable plumbing"
        );
    }
}
