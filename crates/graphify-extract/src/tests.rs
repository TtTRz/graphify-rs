use super::*;
use graphify_core::model::{GraphEdge, GraphNode};

#[test]
fn dispatch_table_covers_all_languages() {
    let map = dispatch_map();
    assert_eq!(map.get(".py"), Some(&"python"));
    assert_eq!(map.get(".rs"), Some(&"rust"));
    assert_eq!(map.get(".go"), Some(&"go"));
    assert_eq!(map.get(".tsx"), Some(&"typescript"));
    assert_eq!(map.get(".jl"), Some(&"julia"));
    assert_eq!(map.get(".mm"), Some(&"objc"));
}

fn make_test_node(id: &str, label: &str, source_file: &str, node_type: NodeType) -> GraphNode {
    GraphNode {
        id: id.to_string(),
        label: label.to_string(),
        source_file: source_file.to_string(),
        source_location: None,
        node_type,
        community: None,
        extra: Default::default(),
    }
}

fn make_test_edge(source: &str, target: &str, relation: &str, source_file: &str) -> GraphEdge {
    GraphEdge {
        source: source.to_string(),
        target: target.to_string(),
        relation: relation.to_string(),
        confidence: Confidence::Extracted,
        confidence_score: 1.0,
        source_file: source_file.to_string(),
        source_location: None,
        weight: 1.0,
        extra: Default::default(),
    }
}

#[test]
fn jsts_cross_file_creates_uses_edges() {
    // File: src/app.ts defines AppController, imports from "utils"
    // File: src/utils.ts defines parseDate, formatDate
    let mut result = ExtractionResult {
        nodes: vec![
            make_test_node("file_app", "app", "src/app.ts", NodeType::File),
            make_test_node("app_ctrl", "AppController", "src/app.ts", NodeType::Class),
            make_test_node(
                "import_utils",
                "utils/parseDate",
                "src/app.ts",
                NodeType::Module,
            ),
            make_test_node("file_utils", "utils", "src/utils.ts", NodeType::File),
            make_test_node(
                "parse_date",
                "parseDate",
                "src/utils.ts",
                NodeType::Function,
            ),
            make_test_node(
                "format_date",
                "formatDate",
                "src/utils.ts",
                NodeType::Function,
            ),
        ],
        edges: vec![
            make_test_edge("file_app", "app_ctrl", "defines", "src/app.ts"),
            make_test_edge("file_app", "import_utils", "imports", "src/app.ts"),
            make_test_edge("file_utils", "parse_date", "defines", "src/utils.ts"),
            make_test_edge("file_utils", "format_date", "defines", "src/utils.ts"),
        ],
        hyperedges: vec![],
    };

    resolve_cross_file_imports(&mut result);

    let uses_edges: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.relation == "uses")
        .collect();

    assert_eq!(
        uses_edges.len(),
        2,
        "expected 2 uses edges, got {}",
        uses_edges.len()
    );
    assert!(
        uses_edges
            .iter()
            .any(|e| e.source == "app_ctrl" && e.target == "parse_date")
    );
    assert!(
        uses_edges
            .iter()
            .any(|e| e.source == "app_ctrl" && e.target == "format_date")
    );

    for edge in &uses_edges {
        assert_eq!(edge.confidence, Confidence::Inferred);
        assert!((edge.weight - 0.8).abs() < f64::EPSILON);
        assert!((edge.confidence_score - 0.8).abs() < f64::EPSILON);
    }
}

#[test]
fn go_cross_file_creates_uses_edges() {
    // File: cmd/main.go defines Server, imports "myproject/pkg/utils"
    // File: pkg/utils/helpers.go defines ParseConfig, Validate
    let mut result = ExtractionResult {
        nodes: vec![
            make_test_node("file_main", "main", "cmd/main.go", NodeType::File),
            make_test_node("server", "Server", "cmd/main.go", NodeType::Struct),
            make_test_node(
                "import_utils",
                "myproject/pkg/utils",
                "cmd/main.go",
                NodeType::Package,
            ),
            make_test_node(
                "file_helpers",
                "helpers",
                "pkg/utils/helpers.go",
                NodeType::File,
            ),
            make_test_node(
                "parse_config",
                "ParseConfig",
                "pkg/utils/helpers.go",
                NodeType::Function,
            ),
            make_test_node(
                "validate",
                "Validate",
                "pkg/utils/helpers.go",
                NodeType::Function,
            ),
        ],
        edges: vec![
            make_test_edge("file_main", "server", "defines", "cmd/main.go"),
            make_test_edge("file_main", "import_utils", "imports", "cmd/main.go"),
            make_test_edge(
                "file_helpers",
                "parse_config",
                "defines",
                "pkg/utils/helpers.go",
            ),
            make_test_edge(
                "file_helpers",
                "validate",
                "defines",
                "pkg/utils/helpers.go",
            ),
        ],
        hyperedges: vec![],
    };

    resolve_cross_file_imports(&mut result);

    let uses_edges: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.relation == "uses")
        .collect();

    assert_eq!(
        uses_edges.len(),
        2,
        "expected 2 uses edges, got {}",
        uses_edges.len()
    );
    assert!(
        uses_edges
            .iter()
            .any(|e| e.source == "server" && e.target == "parse_config")
    );
    assert!(
        uses_edges
            .iter()
            .any(|e| e.source == "server" && e.target == "validate")
    );

    for edge in &uses_edges {
        assert_eq!(edge.confidence, Confidence::Inferred);
    }
}

#[test]
fn rust_cross_file_creates_uses_edges() {
    // File: src/main.rs defines App, imports "crate::model"
    // File: src/model.rs defines Config, Database
    let mut result = ExtractionResult {
        nodes: vec![
            make_test_node("file_main", "main", "src/main.rs", NodeType::File),
            make_test_node("app", "App", "src/main.rs", NodeType::Struct),
            make_test_node(
                "import_model",
                "crate::model",
                "src/main.rs",
                NodeType::Module,
            ),
            make_test_node("file_model", "model", "src/model.rs", NodeType::File),
            make_test_node("config", "Config", "src/model.rs", NodeType::Struct),
            make_test_node("database", "Database", "src/model.rs", NodeType::Struct),
        ],
        edges: vec![
            make_test_edge("file_main", "app", "defines", "src/main.rs"),
            make_test_edge("file_main", "import_model", "imports", "src/main.rs"),
            make_test_edge("file_model", "config", "defines", "src/model.rs"),
            make_test_edge("file_model", "database", "defines", "src/model.rs"),
        ],
        hyperedges: vec![],
    };

    resolve_cross_file_imports(&mut result);

    let uses_edges: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.relation == "uses")
        .collect();

    assert_eq!(
        uses_edges.len(),
        2,
        "expected 2 uses edges, got {}",
        uses_edges.len()
    );
    assert!(
        uses_edges
            .iter()
            .any(|e| e.source == "app" && e.target == "config")
    );
    assert!(
        uses_edges
            .iter()
            .any(|e| e.source == "app" && e.target == "database")
    );

    for edge in &uses_edges {
        assert_eq!(edge.confidence, Confidence::Inferred);
        assert!((edge.weight - 0.8).abs() < f64::EPSILON);
    }
}

#[test]
fn rust_cross_file_resolves_specific_type() {
    // `use crate::model::Config` should prefer Config over all entities in model
    let mut result = ExtractionResult {
        nodes: vec![
            make_test_node("file_main", "main", "src/main.rs", NodeType::File),
            make_test_node("app", "App", "src/main.rs", NodeType::Struct),
            make_test_node(
                "import_config",
                "crate::model::Config",
                "src/main.rs",
                NodeType::Module,
            ),
            make_test_node("file_model", "model", "src/model.rs", NodeType::File),
            make_test_node("config", "Config", "src/model.rs", NodeType::Struct),
            make_test_node("database", "Database", "src/model.rs", NodeType::Struct),
        ],
        edges: vec![
            make_test_edge("file_main", "app", "defines", "src/main.rs"),
            make_test_edge("file_main", "import_config", "imports", "src/main.rs"),
            make_test_edge("file_model", "config", "defines", "src/model.rs"),
            make_test_edge("file_model", "database", "defines", "src/model.rs"),
        ],
        hyperedges: vec![],
    };

    resolve_cross_file_imports(&mut result);

    let uses_edges: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.relation == "uses")
        .collect();

    assert_eq!(
        uses_edges.len(),
        1,
        "expected 1 uses edge, got {}",
        uses_edges.len()
    );
    assert_eq!(uses_edges[0].source, "app");
    assert_eq!(uses_edges[0].target, "config");
}

#[test]
fn cross_file_no_duplicate_edges() {
    let mut result = ExtractionResult {
        nodes: vec![
            make_test_node("file_app", "app", "src/app.ts", NodeType::File),
            make_test_node("ctrl", "Controller", "src/app.ts", NodeType::Class),
            make_test_node("import1", "utils/foo", "src/app.ts", NodeType::Module),
            make_test_node("import2", "utils/bar", "src/app.ts", NodeType::Module),
            make_test_node("file_utils", "utils", "src/utils.ts", NodeType::File),
            make_test_node("helper", "Helper", "src/utils.ts", NodeType::Class),
        ],
        edges: vec![
            make_test_edge("file_app", "ctrl", "defines", "src/app.ts"),
            make_test_edge("file_app", "import1", "imports", "src/app.ts"),
            make_test_edge("file_app", "import2", "imports", "src/app.ts"),
            make_test_edge("file_utils", "helper", "defines", "src/utils.ts"),
        ],
        hyperedges: vec![],
    };

    resolve_cross_file_imports(&mut result);

    let uses_edges: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.relation == "uses")
        .collect();

    assert_eq!(
        uses_edges.len(),
        1,
        "expected 1 uses edge (no dups), got {}",
        uses_edges.len()
    );
}

#[test]
fn cross_file_unresolved_import_creates_no_edges() {
    let mut result = ExtractionResult {
        nodes: vec![
            make_test_node("file_main", "main", "src/main.rs", NodeType::File),
            make_test_node("app", "App", "src/main.rs", NodeType::Struct),
            make_test_node(
                "import_serde",
                "serde::Deserialize",
                "src/main.rs",
                NodeType::Module,
            ),
        ],
        edges: vec![
            make_test_edge("file_main", "app", "defines", "src/main.rs"),
            make_test_edge("file_main", "import_serde", "imports", "src/main.rs"),
        ],
        hyperedges: vec![],
    };

    resolve_cross_file_imports(&mut result);

    let uses_edges: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.relation == "uses")
        .collect();

    assert!(
        uses_edges.is_empty(),
        "external imports should not create uses edges"
    );
}

#[test]
fn python_resolver_not_broken_by_cross_file() {
    let mut result = ExtractionResult {
        nodes: vec![
            make_test_node("file_a", "module_a", "src/a.py", NodeType::File),
            make_test_node("my_class", "MyClass", "src/a.py", NodeType::Class),
        ],
        edges: vec![make_test_edge("file_a", "MyClass", "imports", "src/a.py")],
        hyperedges: vec![],
    };

    resolve_python_imports(&mut result);

    assert_eq!(result.edges[0].target, "my_class");
}

#[test]
fn java_cross_file_creates_uses_edges() {
    let mut result = ExtractionResult {
        nodes: vec![
            make_test_node("file_app", "App", "src/App.java", NodeType::File),
            make_test_node("app_class", "App", "src/App.java", NodeType::Class),
            make_test_node(
                "import_util",
                "com.example.Util",
                "src/App.java",
                NodeType::Module,
            ),
            make_test_node("file_util", "Util", "src/Util.java", NodeType::File),
            make_test_node("util_class", "Util", "src/Util.java", NodeType::Class),
        ],
        edges: vec![
            make_test_edge("file_app", "app_class", "defines", "src/App.java"),
            make_test_edge("file_app", "import_util", "imports", "src/App.java"),
            make_test_edge("file_util", "util_class", "defines", "src/Util.java"),
        ],
        hyperedges: vec![],
    };

    resolve_cross_file_imports(&mut result);

    let uses_edges: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.relation == "uses")
        .collect();
    assert!(
        !uses_edges.is_empty(),
        "Java cross-file should create uses edges"
    );
    assert!(
        uses_edges
            .iter()
            .any(|e| e.source == "app_class" && e.target == "util_class")
    );
}

#[test]
fn c_include_creates_uses_edges() {
    let mut result = ExtractionResult {
        nodes: vec![
            make_test_node("file_main", "main", "src/main.c", NodeType::File),
            make_test_node("main_fn", "main", "src/main.c", NodeType::Function),
            make_test_node("import_utils", "utils.h", "src/main.c", NodeType::Module),
            make_test_node("file_utils", "utils", "src/utils.c", NodeType::File),
            make_test_node("helper_fn", "helper", "src/utils.c", NodeType::Function),
        ],
        edges: vec![
            make_test_edge("file_main", "main_fn", "defines", "src/main.c"),
            make_test_edge("file_main", "import_utils", "imports", "src/main.c"),
            make_test_edge("file_utils", "helper_fn", "defines", "src/utils.c"),
        ],
        hyperedges: vec![],
    };

    resolve_cross_file_imports(&mut result);

    let uses_edges: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.relation == "uses")
        .collect();
    assert!(!uses_edges.is_empty(), "C include should create uses edges");
    assert!(
        uses_edges
            .iter()
            .any(|e| e.source == "main_fn" && e.target == "helper_fn")
    );
}

#[test]
fn csharp_using_creates_uses_edges() {
    let mut result = ExtractionResult {
        nodes: vec![
            make_test_node("file_prog", "Program", "src/Program.cs", NodeType::File),
            make_test_node("prog_class", "Program", "src/Program.cs", NodeType::Class),
            make_test_node(
                "import_svc",
                "MyApp.Services.UserService",
                "src/Program.cs",
                NodeType::Module,
            ),
            make_test_node(
                "file_svc",
                "UserService",
                "src/UserService.cs",
                NodeType::File,
            ),
            make_test_node(
                "svc_class",
                "UserService",
                "src/UserService.cs",
                NodeType::Class,
            ),
        ],
        edges: vec![
            make_test_edge("file_prog", "prog_class", "defines", "src/Program.cs"),
            make_test_edge("file_prog", "import_svc", "imports", "src/Program.cs"),
            make_test_edge("file_svc", "svc_class", "defines", "src/UserService.cs"),
        ],
        hyperedges: vec![],
    };

    resolve_cross_file_imports(&mut result);

    let uses_edges: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.relation == "uses")
        .collect();
    assert!(!uses_edges.is_empty(), "C# using should create uses edges");
    assert!(
        uses_edges
            .iter()
            .any(|e| e.source == "prog_class" && e.target == "svc_class")
    );
}

#[test]
fn php_use_creates_uses_edges() {
    let mut result = ExtractionResult {
        nodes: vec![
            make_test_node(
                "file_ctrl",
                "Controller",
                "src/Controller.php",
                NodeType::File,
            ),
            make_test_node(
                "ctrl_class",
                "Controller",
                "src/Controller.php",
                NodeType::Class,
            ),
            make_test_node(
                "import_user",
                r"use App\Models\User",
                "src/Controller.php",
                NodeType::Module,
            ),
            make_test_node("file_user", "User", "src/User.php", NodeType::File),
            make_test_node("user_class", "User", "src/User.php", NodeType::Class),
        ],
        edges: vec![
            make_test_edge("file_ctrl", "ctrl_class", "defines", "src/Controller.php"),
            make_test_edge("file_ctrl", "import_user", "imports", "src/Controller.php"),
            make_test_edge("file_user", "user_class", "defines", "src/User.php"),
        ],
        hyperedges: vec![],
    };

    resolve_cross_file_imports(&mut result);

    let uses_edges: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.relation == "uses")
        .collect();
    assert!(!uses_edges.is_empty(), "PHP use should create uses edges");
    assert!(
        uses_edges
            .iter()
            .any(|e| e.source == "ctrl_class" && e.target == "user_class")
    );
}

#[test]
fn dart_import_creates_uses_edges() {
    let mut result = ExtractionResult {
        nodes: vec![
            make_test_node("file_main", "main", "lib/main.dart", NodeType::File),
            make_test_node("main_fn", "main", "lib/main.dart", NodeType::Function),
            make_test_node(
                "import_utils",
                "import 'package:myapp/utils.dart'",
                "lib/main.dart",
                NodeType::Module,
            ),
            make_test_node("file_utils", "utils", "lib/utils.dart", NodeType::File),
            make_test_node("helper_fn", "helper", "lib/utils.dart", NodeType::Function),
        ],
        edges: vec![
            make_test_edge("file_main", "main_fn", "defines", "lib/main.dart"),
            make_test_edge("file_main", "import_utils", "imports", "lib/main.dart"),
            make_test_edge("file_utils", "helper_fn", "defines", "lib/utils.dart"),
        ],
        hyperedges: vec![],
    };

    resolve_cross_file_imports(&mut result);

    let uses_edges: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.relation == "uses")
        .collect();
    assert!(
        !uses_edges.is_empty(),
        "Dart import should create uses edges"
    );
    assert!(
        uses_edges
            .iter()
            .any(|e| e.source == "main_fn" && e.target == "helper_fn")
    );
}

#[test]
fn kotlin_import_creates_uses_edges() {
    let mut result = ExtractionResult {
        nodes: vec![
            make_test_node("file_main", "Main", "src/Main.kt", NodeType::File),
            make_test_node("main_fn", "main", "src/Main.kt", NodeType::Function),
            make_test_node(
                "import_repo",
                "import com.example.UserRepo",
                "src/Main.kt",
                NodeType::Module,
            ),
            make_test_node("file_repo", "UserRepo", "src/UserRepo.kt", NodeType::File),
            make_test_node("repo_class", "UserRepo", "src/UserRepo.kt", NodeType::Class),
        ],
        edges: vec![
            make_test_edge("file_main", "main_fn", "defines", "src/Main.kt"),
            make_test_edge("file_main", "import_repo", "imports", "src/Main.kt"),
            make_test_edge("file_repo", "repo_class", "defines", "src/UserRepo.kt"),
        ],
        hyperedges: vec![],
    };

    resolve_cross_file_imports(&mut result);

    let uses_edges: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.relation == "uses")
        .collect();
    assert!(
        !uses_edges.is_empty(),
        "Kotlin import should create uses edges"
    );
    assert!(
        uses_edges
            .iter()
            .any(|e| e.source == "main_fn" && e.target == "repo_class")
    );
}

#[test]
fn python_star_import_expands_to_entities() {
    let mut result = ExtractionResult {
        nodes: vec![
            make_test_node("file_app", "app", "src/app.py", NodeType::File),
            make_test_node("app_fn", "run", "src/app.py", NodeType::Function),
            make_test_node("import_star", "utils.*", "src/app.py", NodeType::Module),
            make_test_node("file_utils", "utils", "src/utils.py", NodeType::File),
            make_test_node("helper1", "helper1", "src/utils.py", NodeType::Function),
            make_test_node("helper2", "helper2", "src/utils.py", NodeType::Function),
        ],
        edges: vec![
            make_test_edge("file_app", "app_fn", "defines", "src/app.py"),
            make_test_edge("file_app", "import_star", "imports", "src/app.py"),
            make_test_edge("file_utils", "helper1", "defines", "src/utils.py"),
            make_test_edge("file_utils", "helper2", "defines", "src/utils.py"),
        ],
        hyperedges: vec![],
    };

    resolve_python_imports(&mut result);

    let uses_edges: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.relation == "uses")
        .collect();
    assert_eq!(
        uses_edges.len(),
        2,
        "star import should expand to 2 uses edges, got {}",
        uses_edges.len()
    );
}

#[test]
fn scala_cross_file_creates_uses_edges() {
    let mut result = ExtractionResult {
        nodes: vec![
            make_test_node("file_main", "Main", "src/Main.scala", NodeType::File),
            make_test_node("main_fn", "main", "src/Main.scala", NodeType::Function),
            make_test_node(
                "import_calc",
                "import com.example.Calculator",
                "src/Main.scala",
                NodeType::Module,
            ),
            make_test_node(
                "file_calc",
                "Calculator",
                "src/Calculator.scala",
                NodeType::File,
            ),
            make_test_node(
                "calc_class",
                "Calculator",
                "src/Calculator.scala",
                NodeType::Class,
            ),
        ],
        edges: vec![
            make_test_edge("file_main", "main_fn", "defines", "src/Main.scala"),
            make_test_edge("file_main", "import_calc", "imports", "src/Main.scala"),
            make_test_edge("file_calc", "calc_class", "defines", "src/Calculator.scala"),
        ],
        hyperedges: vec![],
    };

    resolve_cross_file_imports(&mut result);

    let uses_edges: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.relation == "uses")
        .collect();
    assert!(
        !uses_edges.is_empty(),
        "Scala cross-file should create uses edges"
    );
    assert!(
        uses_edges
            .iter()
            .any(|e| e.source == "main_fn" && e.target == "calc_class")
    );
}

#[test]
fn swift_cross_file_creates_uses_edges() {
    let mut result = ExtractionResult {
        nodes: vec![
            make_test_node("file_app", "App", "src/App.swift", NodeType::File),
            make_test_node("app_fn", "run", "src/App.swift", NodeType::Function),
            make_test_node(
                "import_mgr",
                "import UserManager",
                "src/App.swift",
                NodeType::Module,
            ),
            make_test_node(
                "file_mgr",
                "UserManager",
                "src/UserManager.swift",
                NodeType::File,
            ),
            make_test_node(
                "mgr_class",
                "UserManager",
                "src/UserManager.swift",
                NodeType::Class,
            ),
        ],
        edges: vec![
            make_test_edge("file_app", "app_fn", "defines", "src/App.swift"),
            make_test_edge("file_app", "import_mgr", "imports", "src/App.swift"),
            make_test_edge("file_mgr", "mgr_class", "defines", "src/UserManager.swift"),
        ],
        hyperedges: vec![],
    };

    resolve_cross_file_imports(&mut result);

    let uses_edges: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.relation == "uses")
        .collect();
    assert!(
        !uses_edges.is_empty(),
        "Swift cross-file should create uses edges"
    );
    assert!(
        uses_edges
            .iter()
            .any(|e| e.source == "app_fn" && e.target == "mgr_class")
    );
}

#[test]
fn jsts_resolver_strips_alias() {
    let mut entities = HashMap::new();
    entities.insert(
        "utils".to_string(),
        vec![("parseDate".into(), "pd_id".into(), NodeType::Function)],
    );
    // "utils/parseDate as pd" should still resolve to utils entities
    let result = resolve_jsts_import("utils/parseDate as pd", &entities);
    assert!(!result.is_empty(), "aliased JS import should resolve");
}

#[test]
fn go_resolver_handles_blank_import() {
    let mut entities = HashMap::new();
    entities.insert(
        "driver".to_string(),
        vec![("Register".into(), "reg_id".into(), NodeType::Function)],
    );
    let empty = HashMap::new();
    // import _ "database/sql/driver"
    let result = resolve_go_import("_ database/sql/driver", &entities, &empty);
    assert!(!result.is_empty(), "Go blank import should resolve");
}

#[test]
fn go_resolver_handles_alias_import() {
    let mut entities = HashMap::new();
    entities.insert(
        "http".to_string(),
        vec![("Server".into(), "srv_id".into(), NodeType::Struct)],
    );
    let empty = HashMap::new();
    // import h "net/http"
    let result = resolve_go_import(r#"h "net/http""#, &entities, &empty);
    assert!(!result.is_empty(), "Go aliased import should resolve");
}

#[test]
fn rust_resolver_handles_glob() {
    let mut entities = HashMap::new();
    entities.insert(
        "model".to_string(),
        vec![
            ("Config".into(), "cfg_id".into(), NodeType::Struct),
            ("Database".into(), "db_id".into(), NodeType::Struct),
        ],
    );
    // use crate::model::*
    let result = resolve_rust_import("crate::model::*", &entities);
    assert_eq!(result.len(), 2, "glob import should return all entities");
}

#[test]
fn dot_resolver_handles_static_import() {
    let mut entities = HashMap::new();
    entities.insert(
        "Math".to_string(),
        vec![("sqrt".into(), "sqrt_id".into(), NodeType::Function)],
    );
    // import static java.lang.Math.sqrt
    let result = resolve_dot_import("static java.lang.Math.sqrt", &entities);
    assert!(
        !result.is_empty(),
        "Java static import should resolve: got empty"
    );
}

#[test]
fn dot_resolver_handles_csharp_alias() {
    let mut entities = HashMap::new();
    entities.insert(
        "MySqlClient".to_string(),
        vec![("Connection".into(), "conn_id".into(), NodeType::Class)],
    );
    // using MySql = MySql.Data.MySqlClient
    let result = resolve_dot_import("MySql = MySql.Data.MySqlClient", &entities);
    assert!(
        !result.is_empty(),
        "C# alias using should resolve: got empty"
    );
}

#[test]
fn dart_resolver_handles_relative_import() {
    let mut entities = HashMap::new();
    entities.insert(
        "user".to_string(),
        vec![("User".into(), "user_id".into(), NodeType::Class)],
    );
    let result = resolve_dart_import("import '../models/user.dart'", &entities);
    assert!(!result.is_empty(), "Dart relative import should resolve");
}

#[test]
fn dart_resolver_handles_deferred_import() {
    let mut entities = HashMap::new();
    entities.insert(
        "heavy".to_string(),
        vec![("compute".into(), "comp_id".into(), NodeType::Function)],
    );
    let result = resolve_dart_import(
        "import 'package:myapp/heavy.dart' deferred as heavy",
        &entities,
    );
    assert!(!result.is_empty(), "Dart deferred import should resolve");
}

#[test]
fn dart_resolver_handles_part_directive() {
    let mut entities = HashMap::new();
    entities.insert(
        "models".to_string(),
        vec![("Item".into(), "item_id".into(), NodeType::Class)],
    );
    let result = resolve_dart_import("part 'src/models.dart'", &entities);
    assert!(!result.is_empty(), "Dart part directive should resolve");
}

// --- resolve_cross_file_calls tests ---

#[test]
fn cross_file_calls_derived_from_uses_edges() {
    let mut result = ExtractionResult {
        nodes: vec![
            make_test_node("file_main", "main", "src/main.rs", NodeType::File),
            make_test_node("run", "run()", "src/main.rs", NodeType::Function),
            make_test_node("file_model", "model", "src/model.rs", NodeType::File),
            make_test_node("init", "init()", "src/model.rs", NodeType::Function),
        ],
        edges: vec![
            make_test_edge("file_main", "run", "defines", "src/main.rs"),
            make_test_edge("file_model", "init", "defines", "src/model.rs"),
            make_test_edge("run", "init", "uses", "src/main.rs"),
        ],
        hyperedges: vec![],
    };

    resolve_cross_file_calls(&mut result);

    let calls_edges: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.relation == "calls")
        .collect();

    assert_eq!(
        calls_edges.len(),
        1,
        "expected 1 calls edge, got {}",
        calls_edges.len()
    );
    assert_eq!(calls_edges[0].source, "run");
    assert_eq!(calls_edges[0].target, "init");
    assert_eq!(calls_edges[0].confidence, Confidence::Inferred);
    assert!((calls_edges[0].weight - 0.5).abs() < f64::EPSILON);
}

#[test]
fn cross_file_calls_skips_non_function_source() {
    let mut result = ExtractionResult {
        nodes: vec![
            make_test_node("file_main", "main", "src/main.rs", NodeType::File),
            make_test_node("app", "App", "src/main.rs", NodeType::Struct),
            make_test_node("file_model", "model", "src/model.rs", NodeType::File),
            make_test_node("config", "Config", "src/model.rs", NodeType::Struct),
        ],
        edges: vec![
            make_test_edge("file_main", "app", "defines", "src/main.rs"),
            make_test_edge("file_model", "config", "defines", "src/model.rs"),
            make_test_edge("app", "config", "uses", "src/main.rs"),
        ],
        hyperedges: vec![],
    };

    resolve_cross_file_calls(&mut result);

    let calls_edges: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.relation == "calls")
        .collect();

    assert!(
        calls_edges.is_empty(),
        "Struct→Struct uses should not produce calls edge"
    );
}

#[test]
fn cross_file_calls_no_duplicate_with_existing() {
    let mut result = ExtractionResult {
        nodes: vec![
            make_test_node("run", "run()", "src/main.rs", NodeType::Function),
            make_test_node("init", "init()", "src/model.rs", NodeType::Function),
        ],
        edges: vec![
            GraphEdge {
                source: "run".to_string(),
                target: "init".to_string(),
                relation: "calls".to_string(),
                confidence: Confidence::Extracted,
                confidence_score: 1.0,
                source_file: "src/main.rs".to_string(),
                source_location: None,
                weight: 1.0,
                extra: HashMap::new(),
            },
            make_test_edge("run", "init", "uses", "src/main.rs"),
        ],
        hyperedges: vec![],
    };

    resolve_cross_file_calls(&mut result);

    let calls_edges: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.relation == "calls")
        .collect();

    assert_eq!(
        calls_edges.len(),
        1,
        "should not duplicate existing calls edge"
    );
}

#[test]
fn cross_file_calls_no_edges_without_uses() {
    let mut result = ExtractionResult {
        nodes: vec![make_test_node(
            "run",
            "run()",
            "src/main.rs",
            NodeType::Function,
        )],
        edges: vec![make_test_edge("file_main", "run", "defines", "src/main.rs")],
        hyperedges: vec![],
    };

    resolve_cross_file_calls(&mut result);

    let calls_edges: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.relation == "calls")
        .collect();

    assert!(
        calls_edges.is_empty(),
        "should not create calls edges without uses edges"
    );
}
