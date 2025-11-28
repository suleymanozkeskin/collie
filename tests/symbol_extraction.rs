use collie_search::symbols::adapters::{
    GoAdapter, LanguageAdapter, PythonAdapter, RustAdapter, TypeScriptAdapter,
};
use collie_search::symbols::{Symbol, SymbolKind};
use std::path::Path;

fn names_of_kind(symbols: &[Symbol], kind: SymbolKind) -> Vec<String> {
    let mut names: Vec<String> = symbols
        .iter()
        .filter(|symbol| symbol.kind == kind)
        .map(|symbol| symbol.name.clone())
        .collect();
    names.sort();
    names
}

fn find_symbol<'a>(symbols: &'a [Symbol], kind: SymbolKind, name: &str) -> &'a Symbol {
    symbols
        .iter()
        .find(|symbol| symbol.kind == kind && symbol.name == name)
        .unwrap_or_else(|| panic!("missing symbol {kind:?} {name}"))
}

#[test]
fn go_extracts_functions() {
    let adapter = GoAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("pkg/api/handler.go"),
        "package api\n\nfunc handleRequest(w http.ResponseWriter, r *http.Request) {}\n",
    );

    let symbol = find_symbol(&symbols, SymbolKind::Function, "handleRequest");
    assert!(
        symbol
            .signature
            .as_deref()
            .unwrap_or("")
            .contains("handleRequest")
    );
}

#[test]
fn go_extracts_methods() {
    let adapter = GoAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("pkg/api/server.go"),
        "package api\n\ntype Server struct {}\nfunc (s *Server) Start() error { return nil }\n",
    );

    let symbol = find_symbol(&symbols, SymbolKind::Method, "Start");
    assert_eq!(symbol.container_name.as_deref(), Some("Server"));
}

#[test]
fn go_extracts_structs() {
    let adapter = GoAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("pkg/api/config.go"),
        "package api\n\ntype Config struct { Host string; Port int }\n",
    );

    find_symbol(&symbols, SymbolKind::Struct, "Config");
}

#[test]
fn go_extracts_interfaces() {
    let adapter = GoAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("pkg/api/handler.go"),
        "package api\n\ntype Handler interface { ServeHTTP(w ResponseWriter, r *Request) }\n",
    );

    find_symbol(&symbols, SymbolKind::Interface, "Handler");
}

#[test]
fn go_extracts_constants() {
    let adapter = GoAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("pkg/api/constants.go"),
        "package api\n\nconst MaxRetries = 3\n",
    );

    find_symbol(&symbols, SymbolKind::Constant, "MaxRetries");
}

#[test]
fn go_extracts_variables() {
    let adapter = GoAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("pkg/api/vars.go"),
        "package api\n\nvar defaultTimeout = time.Second * 30\n",
    );

    find_symbol(&symbols, SymbolKind::Variable, "defaultTimeout");
}

#[test]
fn go_extracts_type_aliases() {
    let adapter = GoAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("pkg/api/types.go"),
        "package api\n\ntype UserID string\n",
    );

    find_symbol(&symbols, SymbolKind::TypeAlias, "UserID");
}

#[test]
fn go_extracts_struct_fields() {
    let adapter = GoAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("pkg/api/config.go"),
        "package api\n\ntype Config struct { Host string; Port int }\n",
    );

    assert_eq!(
        names_of_kind(&symbols, SymbolKind::Field),
        vec!["Host", "Port"]
    );
}

#[test]
fn go_multi_symbol_file() {
    let adapter = GoAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("pkg/api/all.go"),
        r#"package api

type Config struct { Host string }
type Server struct { Port int }
type Handler interface { ServeHTTP() }

func boot() {}
func stop() {}
func health() {}
"#,
    );

    assert_eq!(
        names_of_kind(&symbols, SymbolKind::Function),
        vec!["boot", "health", "stop"]
    );
    assert_eq!(
        names_of_kind(&symbols, SymbolKind::Struct),
        vec!["Config", "Server"]
    );
    assert_eq!(
        names_of_kind(&symbols, SymbolKind::Interface),
        vec!["Handler"]
    );
}

#[test]
fn go_visibility_detection() {
    let adapter = GoAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("pkg/api/visibility.go"),
        "package api\n\nfunc ExportedFunc() {}\nfunc unexportedFunc() {}\n",
    );

    assert_eq!(
        find_symbol(&symbols, SymbolKind::Function, "ExportedFunc")
            .visibility
            .as_deref(),
        Some("pub")
    );
    assert_eq!(
        find_symbol(&symbols, SymbolKind::Function, "unexportedFunc")
            .visibility
            .as_deref(),
        Some("private")
    );
}

#[test]
fn rust_extracts_functions() {
    let adapter = RustAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("src/lib.rs"),
        "pub fn calculate_sum(a: i32, b: i32) -> i32 { a + b }\n",
    );

    let symbol = find_symbol(&symbols, SymbolKind::Function, "calculate_sum");
    assert_eq!(symbol.visibility.as_deref(), Some("pub"));
}

#[test]
fn rust_extracts_structs() {
    let adapter = RustAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("src/config.rs"),
        "pub struct Config { pub host: String, port: u16 }\n",
    );

    find_symbol(&symbols, SymbolKind::Struct, "Config");
    assert_eq!(
        names_of_kind(&symbols, SymbolKind::Field),
        vec!["host", "port"]
    );
}

#[test]
fn rust_extracts_enums() {
    let adapter = RustAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("src/color.rs"),
        "enum Color { Red, Green, Blue }\n",
    );

    find_symbol(&symbols, SymbolKind::Enum, "Color");
}

#[test]
fn rust_extracts_traits() {
    let adapter = RustAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("src/handler.rs"),
        "pub trait Handler { fn handle(&self); }\n",
    );

    find_symbol(&symbols, SymbolKind::Trait, "Handler");
}

#[test]
fn rust_extracts_impl_methods() {
    let adapter = RustAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("src/server.rs"),
        "struct Server;\nimpl Server { pub fn start(&self) -> Result<(), ()> { Ok(()) } }\n",
    );

    let symbol = find_symbol(&symbols, SymbolKind::Method, "start");
    assert_eq!(symbol.container_name.as_deref(), Some("Server"));
}

#[test]
fn rust_extracts_constants() {
    let adapter = RustAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("src/constants.rs"),
        "const MAX_SIZE: usize = 1024;\n",
    );

    find_symbol(&symbols, SymbolKind::Constant, "MAX_SIZE");
}

#[test]
fn rust_extracts_modules() {
    let adapter = RustAdapter;
    let symbols = adapter.extract_symbols(Path::new("src/lib.rs"), "pub mod handlers { }\n");

    find_symbol(&symbols, SymbolKind::Module, "handlers");
}

#[test]
fn rust_extracts_type_aliases() {
    let adapter = RustAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("src/types.rs"),
        "type Result<T> = std::result::Result<T, Error>;\n",
    );

    find_symbol(&symbols, SymbolKind::TypeAlias, "Result");
}

#[test]
fn python_extracts_functions() {
    let adapter = PythonAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("pkg/api/handler.py"),
        "def handle_request(request):\n    pass\n",
    );

    find_symbol(&symbols, SymbolKind::Function, "handle_request");
}

#[test]
fn python_extracts_classes() {
    let adapter = PythonAdapter;
    let symbols =
        adapter.extract_symbols(Path::new("pkg/api/server.py"), "class Server:\n    pass\n");

    find_symbol(&symbols, SymbolKind::Class, "Server");
}

#[test]
fn python_extracts_methods() {
    let adapter = PythonAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("pkg/api/server.py"),
        "class Server:\n    def start(self):\n        pass\n",
    );

    find_symbol(&symbols, SymbolKind::Class, "Server");
    let method = find_symbol(&symbols, SymbolKind::Method, "start");
    assert_eq!(method.container_name.as_deref(), Some("Server"));
}

#[test]
fn python_extracts_variables() {
    let adapter = PythonAdapter;
    let symbols = adapter.extract_symbols(Path::new("pkg/api/constants.py"), "MAX_RETRIES = 3\n");

    find_symbol(&symbols, SymbolKind::Variable, "MAX_RETRIES");
}

#[test]
fn python_extracts_imports() {
    let adapter = PythonAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("pkg/api/imports.py"),
        "from os.path import join\n",
    );

    find_symbol(&symbols, SymbolKind::Import, "join");
}

#[test]
fn python_extracts_decorators_preserved() {
    let adapter = PythonAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("pkg/api/server.py"),
        "@staticmethod\ndef create():\n    pass\n",
    );

    let functions = names_of_kind(&symbols, SymbolKind::Function);
    assert_eq!(functions, vec!["create"]);
}

#[test]
fn ts_extracts_functions() {
    let adapter = TypeScriptAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("src/api.ts"),
        "export function handleRequest(req: Request): Response { throw new Error(); }\n",
    );

    let symbol = find_symbol(&symbols, SymbolKind::Function, "handleRequest");
    assert_eq!(symbol.visibility.as_deref(), Some("pub"));
}

#[test]
fn ts_extracts_classes() {
    let adapter = TypeScriptAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("src/server.ts"),
        "class Server { constructor() {} }\n",
    );

    find_symbol(&symbols, SymbolKind::Class, "Server");
}

#[test]
fn ts_extracts_interfaces() {
    let adapter = TypeScriptAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("src/types.ts"),
        "interface Handler { handle(): void; }\n",
    );

    find_symbol(&symbols, SymbolKind::Interface, "Handler");
}

#[test]
fn ts_extracts_type_aliases() {
    let adapter = TypeScriptAdapter;
    let symbols = adapter.extract_symbols(Path::new("src/types.ts"), "type UserId = string;\n");

    find_symbol(&symbols, SymbolKind::TypeAlias, "UserId");
}

#[test]
fn ts_extracts_arrow_functions_as_variables() {
    let adapter = TypeScriptAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("src/handler.ts"),
        "const handler = (req: Request) => {};\n",
    );

    find_symbol(&symbols, SymbolKind::Variable, "handler");
}

#[test]
fn ts_extracts_enum() {
    let adapter = TypeScriptAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("src/color.ts"),
        "enum Color { Red, Green, Blue }\n",
    );

    find_symbol(&symbols, SymbolKind::Enum, "Color");
}
