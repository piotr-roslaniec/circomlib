use alloc::{format, vec};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use wasm_bindgen::__rt::std;
use super::lang;
use program_structure::ast::{AST};
use program_structure::ast::produce_report;
use program_structure::error_code::ReportCode;
use program_structure::error_definition::{ReportCollection, Report};
use program_structure::file_definition::FileID;
use rayon::prelude::*;

pub fn preprocess(expr: &str, file_id: FileID) -> Result<String, ReportCollection> {
    let bytes = expr.as_bytes();
    let mut comment_ranges = Vec::new();
    let mut i = 0;
    let mut state = 0;
    let mut block_start = 0;

    while i < bytes.len() {
        match state {
            0 => {
                if bytes[i] == b'/' && i + 1 < bytes.len() {
                    if bytes[i + 1] == b'/' {
                        // Line comment
                        let start = i;
                        i += 2;
                        while i < bytes.len() && bytes[i] != b'\n' {
                            i += 1;
                        }
                        comment_ranges.push(start..i);
                    } else if bytes[i + 1] == b'*' {
                        // Block comment
                        block_start = i;
                        i += 2;
                        state = 2;
                    } else {
                        i += 1;
                    }
                } else {
                    i += 1;
                }
            }
            2 => {
                if bytes[i] == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                    i += 2;
                    comment_ranges.push(block_start..i);
                    state = 0;
                } else {
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }

    if state == 2 {
        return Err(vec![produce_report(
            ReportCode::UnclosedComment,
            block_start..block_start,
            file_id,
        )]);
    }

    let mut pp = bytes.to_vec();
    let comment_ranges = &comment_ranges; // Make borrow explicit for the closure
    pp.par_iter_mut().enumerate().for_each(|(idx, byte)| {
        if comment_ranges.iter().any(|range| range.contains(&idx)) {
            *byte = b' ';
        }
    });

    Ok(String::from_utf8(pp).unwrap())
}

pub fn parse_file(src: &str, file_id: FileID) -> Result<AST, ReportCollection> {
    use lalrpop_util::ParseError::*;

    let mut errors = Vec::new();
    let preprocess = preprocess(src, file_id)?;

    let ast = lang::ParseAstParser::new()
        .parse(file_id, &mut errors, &preprocess)
        // TODO: is this always fatal?
        .map_err(|parse_error| match parse_error {
            InvalidToken { location } => {
                produce_generic_report(format!("{:?}", parse_error), location..location, file_id)
            }
            UnrecognizedToken { ref token, .. } => {
                produce_generic_report(format!("{:?}", parse_error), token.0..token.2, file_id)
            }
            ExtraToken { ref token } => {
                produce_generic_report(format!("{:?}", parse_error), token.0..token.2, file_id)
            }
            _ => produce_generic_report(format!("{:?}", parse_error), 0..0, file_id),
        })
        .map_err(|e| vec![e])?;

    if !errors.is_empty() {
        return Err(errors.into_iter().collect());
    }

    Ok(ast)
}

fn produce_generic_report(format: String, token: std::ops::Range<usize>, file_id: usize) -> Report {
    let mut report = Report::error(format, ReportCode::IllegalExpression);
    report.add_primary(token, file_id, "here".to_string());
    report
}
