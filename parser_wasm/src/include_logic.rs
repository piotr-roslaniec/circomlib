use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use wasm_bindgen::__rt::std::collections::{HashMap, HashSet};
use wasm_bindgen::__rt::std::path::{Path, PathBuf};
use program_structure::ast::produce_report_with_message;
use program_structure::error_code::ReportCode;
use program_structure::error_definition::Report;
use path_clean::PathClean;
use rayon::prelude::*;

fn normalize_path(path: &str) -> String {
    let mut components = vec![""]; // Becomes "/" root
    for part in path.split('/') {
        match part {
            "." | "" => continue,
            ".." => {
                components.pop();
            }
            _ => components.push(part),
        }
    }
    components.join("/")
}

pub struct FileStack {
    current_location: PathBuf,
    black_paths: HashSet<PathBuf>,
    stack: Vec<PathBuf>,
}

impl FileStack {
    pub fn new(src: PathBuf) -> FileStack {
        let mut location = src.clone();
        location.pop();
        FileStack { current_location: location, black_paths: HashSet::new(), stack: vec![src] }
    }

    pub fn add_include(
        &mut self,
        name: &str,
        libraries: &[PathBuf],
        files_map: &HashMap<String, String>,
    ) -> Result<PathBuf, Report> {
        let mut search_paths = Vec::with_capacity(libraries.len() + 1);
        search_paths.push(self.current_location.clone());
        search_paths.extend_from_slice(libraries);
        log::info!("Libraries: {:?}", search_paths);

        if let Some(found_path) = search_paths.par_iter().find_map_any(|lib| {
            let path = lib.join(name).clean();
            log::info!("Checking path: {:?}", path);

            let path_str = match path.to_str() {
                Some(s) => s,
                None => return None,
            };
            log::info!("Canonical path: {:?}", path_str);

            if files_map.contains_key(path_str) {
                Some(path.clone())
            } else {
                None
            }
        }) {
            if !self.black_paths.contains(&found_path) {
                self.stack.push(found_path.clone());
            }
            return Ok(found_path);
        }

        log::info!("Include not found: {}", name);
        Err(produce_report_with_message(ReportCode::IncludeNotFound, name.to_string()))
    }

    pub fn take_next(&mut self) -> Option<PathBuf> {
        while let Some(file) = self.stack.pop() {
            if !self.black_paths.contains(&file) {
                self.current_location =
                    file.parent().unwrap_or_else(|| Path::new("")).to_path_buf();
                self.black_paths.insert(file.clone());
                return Some(file);
            }
        }
        None
    }
}

pub struct IncludesNode {
    pub path: PathBuf,
    pub custom_gates_pragma: bool,
}

#[derive(Default)]
pub struct IncludesGraph {
    nodes: Vec<IncludesNode>,
    adjacency: HashMap<PathBuf, Vec<usize>>,
    custom_gates_nodes: Vec<usize>,
}

impl IncludesGraph {
    pub fn new() -> IncludesGraph {
        IncludesGraph::default()
    }

    pub fn add_node(&mut self, path: PathBuf, custom_gates_pragma: bool, custom_gates_usage: bool) {
        self.nodes.push(IncludesNode { path: path.clone(), custom_gates_pragma });
        if custom_gates_usage {
            self.custom_gates_nodes.push(self.nodes.len() - 1);
        }
    }

    pub fn add_edge(&mut self, path: PathBuf) {
        log::info!("Adding edge: {:?}", path);
        let path = path.clean();
        let edges = self.adjacency.entry(path).or_default();
        edges.push(self.nodes.len() - 1);
    }

    pub fn get_problematic_paths(&self) -> Vec<Vec<PathBuf>> {
        self.custom_gates_nodes
            .par_iter()
            .flat_map(|&from| self.traverse(from, Vec::new(), HashSet::new()))
            .collect()
    }

    fn traverse(
        &self,
        from: usize,
        path: Vec<PathBuf>,
        traversed_edges: HashSet<(usize, usize)>,
    ) -> Vec<Vec<PathBuf>> {
        let mut problematic_paths = Vec::new();
        let node = &self.nodes[from];
        let from_path = &node.path;
        let using_pragma = node.custom_gates_pragma;

        let mut new_path = path.clone();
        new_path.push(from_path.clone());

        if !using_pragma {
            problematic_paths.push(new_path.clone());
        }

        if let Some(edges) = self.adjacency.get(from_path) {
            let results: Vec<Vec<PathBuf>> = edges
                .par_iter()
                .filter_map(|&to| {
                    let edge = (from, to);
                    if !traversed_edges.contains(&edge) {
                        let mut new_traversed_edges = traversed_edges.clone();
                        new_traversed_edges.insert(edge);
                        Some(self.traverse(to, new_path.clone(), new_traversed_edges))
                    } else {
                        None
                    }
                })
                .flatten()
                .collect();
            problematic_paths.extend(results);
        }

        problematic_paths
    }

    pub fn display_path(path: &[PathBuf]) -> String {
        path.iter()
            .map(|file| file.file_name().unwrap_or_else(|| file.as_os_str()).to_string_lossy())
            .collect::<Vec<_>>()
            .join(" -> ")
    }
}
