#![no_std]

extern crate num_bigint_dig as num_bigint;
extern crate num_traits;
extern crate serde;
extern crate serde_derive;
#[macro_use]
extern crate lalrpop_util;
extern crate alloc;

lalrpop_mod!(pub lang);

mod include_logic;
mod parser_logic;
mod syntax_sugar_remover;

use alloc::{format, vec};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::str::FromStr;
use wasm_bindgen::__rt::std::collections::HashMap;
use wasm_bindgen::__rt::std::path::{Path, PathBuf};
use program_structure::ast::{
    produce_compiler_version_report, produce_report, produce_report_with_message,
    produce_version_warning_report, Expression, Version,
};
use program_structure::error_code::ReportCode;
use program_structure::error_definition::ReportCollection;
use program_structure::error_definition::Report;
use program_structure::file_definition::FileLibrary;
use program_structure::program_archive::ProgramArchive;
use crate::include_logic::{FileStack, IncludesGraph};
use crate::syntax_sugar_remover::apply_syntactic_sugar;

pub fn find_file(
    crr_file: &Path,
    ext_link_libraries: &[PathBuf],
    files_map: &HashMap<String, String>,
) -> (bool, String, String, PathBuf, Vec<Report>) {
    let mut path = String::new();
    let mut src = String::new();
    let mut crr_str_file = crr_file.to_path_buf();
    let mut reports = Vec::new();

    for aux in ext_link_libraries {
        let mut p = aux.clone();
        p.push(crr_file);
        crr_str_file = p.clone();
        log::info!("Trying to open file {}", crr_str_file.display());
        match open_file(&crr_str_file, files_map) {
            Ok((new_path, new_src)) => {
                path = new_path;
                src = new_src;
                return (true, path, src, crr_str_file, reports);
            }
            Err(e) => {
                reports.push(e);
            }
        }
    }
    (false, path, src, crr_str_file, reports)
}

pub fn run_parser(
    file: String,
    version: &str,
    link_libraries: Vec<PathBuf>,
    files_map: &HashMap<String, String>,
) -> Result<(ProgramArchive, ReportCollection), (FileLibrary, ReportCollection)> {
    let mut file_library = FileLibrary::new();
    let mut definitions = Vec::new();
    let mut main_components = Vec::new();
    let mut file_stack = FileStack::new(PathBuf::from(file));
    let mut includes_graph = IncludesGraph::new();
    let mut warnings = Vec::new();

    let mut ext_link_libraries = Vec::with_capacity(link_libraries.len() + 1);
    ext_link_libraries.push(PathBuf::new());
    ext_link_libraries.extend(link_libraries.iter().cloned());

    while let Some(crr_file) = FileStack::take_next(&mut file_stack) {
        log::info!("Parsing file {}", crr_file.display());
        let (found, path, src, crr_str_file, reports) =
            find_file(&crr_file, &ext_link_libraries, files_map);
        if !found {
            log::error!("File {} not found", crr_str_file.display());
            return Err((file_library, reports));
        }
        let file_id = file_library.add_file(path.clone(), src.clone());
        let program =
            parser_logic::parse_file(&src, file_id).map_err(|e| (file_library.clone(), e))?;
        if let Some(main) = program.main_component {
            main_components.push((file_id, main, program.custom_gates));
        }
        includes_graph.add_node(crr_str_file, program.custom_gates, program.custom_gates_declared);
        let includes = program.includes;
        definitions.push((file_id, program.definitions));
        for include in includes {
            log::info!("Including file {}", include);
            let path_include = FileStack::add_include(
                &mut file_stack,
                include.clone(),
                &link_libraries,
                files_map,
            )
                .map_err(|e| (file_library.clone(), vec![e]))?;
            includes_graph.add_edge(path_include).map_err(|e| (file_library.clone(), vec![e]))?;
        }
        warnings.append(
            &mut check_number_version(
                path.clone(),
                program.compiler_version,
                parse_number_version(version),
            )
                .map_err(|e| (file_library.clone(), vec![e]))?,
        );
        if program.custom_gates {
            check_custom_gates_version(
                path,
                program.compiler_version,
                parse_number_version(version),
            )
                .map_err(|e| (file_library.clone(), vec![e]))?
        }
    }

    log::info!("Checking main components");
    if main_components.is_empty() {
        let report = produce_report(ReportCode::NoMainFoundInProject, 0..0, 0);
        warnings.push(report);
        Err((file_library, warnings))
    } else if main_components.len() > 1 {
        let report = produce_report_with_main_components(main_components);
        warnings.push(report);
        Err((file_library, warnings))
    } else {
        let mut errors: ReportCollection = includes_graph.get_problematic_paths().iter().map(|path|
            Report::error(
                format!(
                    "Missing custom templates pragma in file {} because of the following chain of includes {}",
                    path.last().unwrap().display(),
                    IncludesGraph::display_path(path)
                ),
                ReportCode::CustomGatesPragmaError,
            )
        ).collect();
        if !errors.is_empty() {
            warnings.append(&mut errors);
            Err((file_library, warnings))
        } else {
            let (main_id, main_component, custom_gates) = main_components.pop().unwrap();
            let result_program_archive = ProgramArchive::new(
                file_library,
                main_id,
                main_component,
                definitions,
                custom_gates,
            );
            match result_program_archive {
                Err((lib, mut rep)) => {
                    warnings.append(&mut rep);
                    Err((lib, warnings))
                }
                Ok(mut program_archive) => {
                    let lib = program_archive.get_file_library().clone();
                    let program_archive_result = apply_syntactic_sugar(&mut program_archive);
                    match program_archive_result {
                        Err(v) => {
                            warnings.push(v);
                            Err((lib, warnings))
                        }
                        Ok(_) => Ok((program_archive, warnings)),
                    }
                }
            }
        }
    }
}

fn produce_report_with_main_components(
    main_components: Vec<(usize, (Vec<String>, Expression), bool)>,
) -> Report {
    let mut r = produce_report(ReportCode::MultipleMain, 0..0, 0);
    for (idx, (i, exp, _)) in main_components.into_iter().enumerate() {
        if idx > 0 {
            r.add_secondary(
                exp.1.get_meta().location.clone(),
                i,
                Some("Here is another main component".to_string()),
            );
        } else {
            r.add_primary(
                exp.1.get_meta().location.clone(),
                i,
                "This is a main component".to_string(),
            );
        }
    }
    r
}

fn open_file(
    path: &Path,
    files_map: &HashMap<String, String>,
) -> Result<(String, String), Report> {
    let path_str = path.to_str().unwrap();
    if let Some(src) = files_map.get(path_str) {
        Ok((path_str.to_string(), src.clone()))
    } else {
        Err(produce_report_with_message(ReportCode::FileOs, path_str.to_string()))
    }
}

fn parse_number_version(version: &str) -> Version {
    let mut parts = version.splitn(3, '.');
    let major = parts.next().and_then(|s| usize::from_str(s).ok()).unwrap_or(0);
    let minor = parts.next().and_then(|s| usize::from_str(s).ok()).unwrap_or(0);
    let patch = parts.next().and_then(|s| usize::from_str(s).ok()).unwrap_or(0);
    (major, minor, patch)
}

fn check_number_version(
    file_path: String,
    version_file: Option<Version>,
    version_compiler: Version,
) -> Result<ReportCollection, Report> {
    if let Some(required_version) = version_file {
        if required_version.0 == version_compiler.0
            && (required_version.1 < version_compiler.1
            || (required_version.1 == version_compiler.1
            && required_version.2 <= version_compiler.2))
        {
            Ok(vec![])
        } else {
            Err(produce_compiler_version_report(file_path, required_version, version_compiler))
        }
    } else {
        let report = produce_version_warning_report(file_path, version_compiler);
        Ok(vec![report])
    }
}

fn check_custom_gates_version(
    file_path: String,
    version_file: Option<Version>,
    version_compiler: Version,
) -> Result<(), Report> {
    let custom_gates_version: Version = (2, 0, 6);
    let version_to_check = version_file.unwrap_or(version_compiler);
    if version_to_check.0 < custom_gates_version.0
        || (version_to_check.0 == custom_gates_version.0
        && version_to_check.1 < custom_gates_version.1)
        || (version_to_check.0 == custom_gates_version.0
        && version_to_check.1 == custom_gates_version.1
        && version_to_check.2 < custom_gates_version.2)
    {
        let report = Report::error(
            format!(
                "File {} requires at least version {:?} to use custom templates (currently {:?})",
                file_path,
                custom_gates_version,
                version_to_check
            ),
            ReportCode::CustomGatesVersionError,
        );
        Err(report)
    } else {
        Ok(())
    }
}
