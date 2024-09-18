use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use wasm_bindgen::__rt::std::collections::{HashMap, HashSet};
use wasm_bindgen::__rt::std::path::PathBuf;
use program_structure::ast::produce_report_with_message;
use program_structure::error_code::ReportCode;
use program_structure::error_definition::Report;
use path_clean::PathClean;

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
        f_stack: &mut FileStack,
        name: String,
        libraries: &Vec<PathBuf>,
        files_map: &HashMap<String, String>,
    ) -> Result<String, Report> {
        let mut libraries2 = Vec::new();
        libraries2.push(f_stack.current_location.clone());
        libraries2.append(&mut libraries.clone());
        log::info!("Libraries: {:?}", libraries2);
        for lib in libraries2 {
            log::info!("lib={}", lib.display());
            log::info!("name={}", name);
            let mut path = PathBuf::new();
            path.push(lib);
            path.push(name.clone());
            log::info!("Canonicalization path: {:?}", path);
            path = path.clean();
            // let canonical_path = CanonicalPath::new(&path).map_err(|e| produce_report_with_message(ReportCode::FileOs, path.display().to_string()));
            // CanonicalPath::new(&path).unwrap();
            // match canonical_path {
            //     Err(_) => {
            //         log::error!("Error canonicalizing path: {:?}", path.display());
            //     }
            //     Ok(path) => {
            log::info!("Checking path: {:?}", path);
            let canonical_str = path
                .to_str()
                .ok_or(produce_report_with_message(ReportCode::FileOs, path.display().to_string()))?
                .to_string();
            log::info!("Canonical path: {:?}", canonical_str);
            let canonical_str = normalize_path(&canonical_str);
            log::info!("Normalized path: {:?}", canonical_str);
            let canonical_path_buf = PathBuf::from(canonical_str.clone());
            if files_map.contains_key(&canonical_str) {
                if !f_stack.black_paths.contains(&canonical_path_buf) {
                    f_stack.stack.push(canonical_path_buf.clone());
                }
                return Result::Ok(canonical_str);
            } else {
                log::info!("files_map keys: {:?}", files_map.keys());
            }
            // }
            // }
        }
        log::info!("Include not found: {}", name);
        Result::Err(produce_report_with_message(ReportCode::IncludeNotFound, name))
    }

    pub fn take_next(f_stack: &mut FileStack) -> Option<PathBuf> {
        loop {
            match f_stack.stack.pop() {
                None => {
                    break None;
                }
                Some(file) if !f_stack.black_paths.contains(&file) => {
                    f_stack.current_location = file.clone();
                    f_stack.current_location.pop();
                    f_stack.black_paths.insert(file.clone());
                    break Some(file);
                }
                _ => {}
            }
        }
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
        self.nodes.push(IncludesNode { path, custom_gates_pragma });
        if custom_gates_usage {
            self.custom_gates_nodes.push(self.nodes.len() - 1);
        }
    }

    pub fn add_edge(&mut self, old_path: String) -> Result<(), Report> {
        log::info!("Adding edge: {}", old_path);
        let path = normalize_path(&old_path);
        log::info!("Normalized path: {}", path);
        let path = PathBuf::from(&path).clean();
        let edges = self.adjacency.entry(path).or_default();
        edges.push(self.nodes.len() - 1);
        Ok(())
    }

    pub fn get_problematic_paths(&self) -> Vec<Vec<PathBuf>> {
        let mut problematic_paths = Vec::new();
        for from in &self.custom_gates_nodes {
            problematic_paths.append(&mut self.traverse(*from, Vec::new(), HashSet::new()));
        }
        problematic_paths
    }

    fn traverse(
        &self,
        from: usize,
        path: Vec<PathBuf>,
        traversed_edges: HashSet<(usize, usize)>,
    ) -> Vec<Vec<PathBuf>> {
        let mut problematic_paths = Vec::new();
        let (from_path, using_pragma) = {
            let node = &self.nodes[from];
            (&node.path, node.custom_gates_pragma)
        };
        let new_path = {
            let mut new_path = path.clone();
            new_path.push(from_path.clone());
            new_path
        };
        if !using_pragma {
            problematic_paths.push(new_path.clone());
        }
        if let Some(edges) = self.adjacency.get(from_path) {
            for to in edges {
                let edge = (from, *to);
                if !traversed_edges.contains(&edge) {
                    let new_traversed_edges = {
                        let mut new_traversed_edges = traversed_edges.clone();
                        new_traversed_edges.insert(edge);
                        new_traversed_edges
                    };
                    problematic_paths.append(&mut self.traverse(
                        *to,
                        new_path.clone(),
                        new_traversed_edges,
                    ));
                }
            }
        }
        problematic_paths
    }

    pub fn display_path(path: &Vec<PathBuf>) -> String {
        let mut res = String::new();
        let mut sep = "";
        for file in path.iter().map(|file| file.display().to_string()) {
            res.push_str(sep);
            let result_split = file.rsplit_once("/");
            if result_split.is_some() {
                res.push_str(result_split.unwrap().1);
            } else {
                res.push_str(&file);
            }
            sep = " -> ";
        }
        res
    }
}
