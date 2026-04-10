//! Integration test: verify Rust extraction produces compatible output
//! with the Python version on a sample Python file.

#[test]
fn extract_sample_python_file() {
    let source = br#"
import os
from pathlib import Path

class FileManager:
    def __init__(self, root):
        self.root = root

    def list_files(self):
        return os.listdir(self.root)

def process(path):
    fm = FileManager(path)
    return fm.list_files()
"#;

    let dir = tempfile::tempdir().unwrap();
    let py_file = dir.path().join("sample.py");
    std::fs::write(&py_file, source).unwrap();

    let result = graphify_extract::extract(&[py_file]);

    assert!(
        result.nodes.len() >= 4,
        "should have file + class + 2 functions, got {} nodes: {:?}",
        result.nodes.len(),
        result.nodes.iter().map(|n| &n.label).collect::<Vec<_>>()
    );

    assert!(
        result.nodes.iter().any(|n| n.label == "FileManager"),
        "should extract FileManager class, got: {:?}",
        result.nodes.iter().map(|n| &n.label).collect::<Vec<_>>()
    );

    assert!(
        result.nodes.iter().any(|n| n.label.contains("list_files")),
        "should extract list_files method, got: {:?}",
        result.nodes.iter().map(|n| &n.label).collect::<Vec<_>>()
    );

    assert!(
        result
            .edges
            .iter()
            .any(|e| e.relation == "imports" || e.relation == "imports_from"),
        "should extract imports, got relations: {:?}",
        result.edges.iter().map(|e| &e.relation).collect::<Vec<_>>()
    );

    assert!(
        result
            .edges
            .iter()
            .any(|e| e.relation == "contains" || e.relation == "defines"),
        "should have structural edges (contains/defines), got relations: {:?}",
        result.edges.iter().map(|e| &e.relation).collect::<Vec<_>>()
    );
}
