//! Cross-language conformance tests for the symbol extraction contract.
//!
//! These tests assert the **target** contract defined in SYMBOL-CONTRACT.md.
//! They verify:
//! - signature is declaration-header-only (no implementation bodies)
//! - Method means type/class-attached callable, not namespace-contained
//! - TypeScript class methods are extracted
//! - TypeScript export does not set visibility = pub
//! - normalize_language rejects languages without active adapters
//! - TypeScript type aliases with object-literal braces are not truncated

use collie_search::symbols::adapters::{
    CAdapter, CSharpAdapter, CppAdapter, GoAdapter, JavaAdapter, LanguageAdapter, PythonAdapter,
    RubyAdapter, RustAdapter, TypeScriptAdapter, ZigAdapter,
};
use collie_search::symbols::query::normalize_language;
use collie_search::symbols::{Symbol, SymbolKind};
use std::path::Path;

fn find_symbol<'a>(symbols: &'a [Symbol], kind: SymbolKind, name: &str) -> &'a Symbol {
    symbols
        .iter()
        .find(|s| s.kind == kind && s.name == name)
        .unwrap_or_else(|| {
            let available: Vec<_> = symbols
                .iter()
                .map(|s| format!("{:?} {:?}", s.kind, s.name))
                .collect();
            panic!("missing {kind:?} {name:?}, found: {available:?}")
        })
}

// ---------------------------------------------------------------------------
// 1a. Signature must not contain body
// ---------------------------------------------------------------------------

#[test]
fn go_function_signature_excludes_body() {
    let adapter = GoAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("handler.go"),
        "package main\n\nfunc handle(r *Request) error {\n\treturn nil\n}\n",
    );
    let sym = find_symbol(&symbols, SymbolKind::Function, "handle");
    let sig = sym.signature.as_deref().unwrap();
    assert!(
        !sig.contains("return nil"),
        "signature must not contain body: {sig}"
    );
}

#[test]
fn go_method_signature_excludes_body() {
    let adapter = GoAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("server.go"),
        "package main\n\ntype Server struct{}\n\nfunc (s *Server) Start() error {\n\treturn nil\n}\n",
    );
    let sym = find_symbol(&symbols, SymbolKind::Method, "Start");
    let sig = sym.signature.as_deref().unwrap();
    assert!(
        !sig.contains("return nil"),
        "signature must not contain body: {sig}"
    );
}

#[test]
fn go_struct_signature_excludes_fields() {
    let adapter = GoAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("config.go"),
        "package main\n\ntype Config struct {\n\tHost string\n\tPort int\n}\n",
    );
    let sym = find_symbol(&symbols, SymbolKind::Struct, "Config");
    let sig = sym.signature.as_deref().unwrap();
    assert!(
        !sig.contains("Host string"),
        "signature must not contain struct fields: {sig}"
    );
}

#[test]
fn rust_function_signature_excludes_body() {
    let adapter = RustAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("lib.rs"),
        "pub fn calculate(a: i32) -> i32 {\n    a + 1\n}\n",
    );
    let sym = find_symbol(&symbols, SymbolKind::Function, "calculate");
    let sig = sym.signature.as_deref().unwrap();
    assert!(
        !sig.contains("a + 1"),
        "signature must not contain body: {sig}"
    );
}

#[test]
fn rust_method_signature_excludes_body() {
    let adapter = RustAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("server.rs"),
        "struct S;\nimpl S {\n    pub fn run(&self) -> bool {\n        true\n    }\n}\n",
    );
    let sym = find_symbol(&symbols, SymbolKind::Method, "run");
    let sig = sym.signature.as_deref().unwrap();
    assert!(
        !sig.contains("true"),
        "signature must not contain body: {sig}"
    );
}

#[test]
fn rust_struct_signature_excludes_fields() {
    let adapter = RustAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("config.rs"),
        "pub struct Config {\n    pub host: String,\n    port: u16,\n}\n",
    );
    let sym = find_symbol(&symbols, SymbolKind::Struct, "Config");
    let sig = sym.signature.as_deref().unwrap();
    assert!(
        !sig.contains("host: String"),
        "signature must not contain struct fields: {sig}"
    );
}

#[test]
fn python_function_signature_excludes_body() {
    let adapter = PythonAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("handler.py"),
        "def handle(request):\n    return response\n",
    );
    let sym = find_symbol(&symbols, SymbolKind::Function, "handle");
    let sig = sym.signature.as_deref().unwrap();
    assert!(
        !sig.contains("return response"),
        "signature must not contain body: {sig}"
    );
}

#[test]
fn python_method_signature_excludes_body() {
    let adapter = PythonAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("server.py"),
        "class S:\n    def run(self):\n        return True\n",
    );
    let sym = find_symbol(&symbols, SymbolKind::Method, "run");
    let sig = sym.signature.as_deref().unwrap();
    assert!(
        !sig.contains("return True"),
        "signature must not contain body: {sig}"
    );
}

#[test]
fn python_class_signature_excludes_body() {
    let adapter = PythonAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("server.py"),
        "class Server:\n    x = 1\n",
    );
    let sym = find_symbol(&symbols, SymbolKind::Class, "Server");
    let sig = sym.signature.as_deref().unwrap();
    assert!(
        !sig.contains("x = 1"),
        "signature must not contain class body: {sig}"
    );
}

#[test]
fn ts_function_signature_excludes_body() {
    let adapter = TypeScriptAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("handler.ts"),
        "function handle(req: Request): Response {\n  return res;\n}\n",
    );
    let sym = find_symbol(&symbols, SymbolKind::Function, "handle");
    let sig = sym.signature.as_deref().unwrap();
    assert!(
        !sig.contains("return res"),
        "signature must not contain body: {sig}"
    );
}

#[test]
fn ts_class_signature_excludes_body() {
    let adapter = TypeScriptAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("server.ts"),
        "class Server {\n  port: number;\n}\n",
    );
    let sym = find_symbol(&symbols, SymbolKind::Class, "Server");
    let sig = sym.signature.as_deref().unwrap();
    assert!(
        !sig.contains("port: number"),
        "signature must not contain class body: {sig}"
    );
}

#[test]
fn java_method_signature_excludes_body() {
    let adapter = JavaAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("C.java"),
        "class C {\n  public void run() {\n    System.out.println(\"x\");\n  }\n}\n",
    );
    let sym = find_symbol(&symbols, SymbolKind::Method, "run");
    let sig = sym.signature.as_deref().unwrap();
    assert!(
        !sig.contains("System.out"),
        "signature must not contain body: {sig}"
    );
}

#[test]
fn java_class_signature_excludes_body() {
    let adapter = JavaAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("Server.java"),
        "public class Server {\n  int port;\n}\n",
    );
    let sym = find_symbol(&symbols, SymbolKind::Class, "Server");
    let sig = sym.signature.as_deref().unwrap();
    assert!(
        !sig.contains("int port"),
        "signature must not contain class body: {sig}"
    );
}

#[test]
fn cpp_function_signature_excludes_body() {
    let adapter = CppAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("handler.cpp"),
        "void handle(int fd) {\n  close(fd);\n}\n",
    );
    let sym = find_symbol(&symbols, SymbolKind::Function, "handle");
    let sig = sym.signature.as_deref().unwrap();
    assert!(
        !sig.contains("close(fd)"),
        "signature must not contain body: {sig}"
    );
}

#[test]
fn cpp_class_signature_excludes_body() {
    let adapter = CppAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("server.cpp"),
        "class Server {\n  int port;\n};\n",
    );
    let sym = find_symbol(&symbols, SymbolKind::Class, "Server");
    let sig = sym.signature.as_deref().unwrap();
    assert!(
        !sig.contains("int port"),
        "signature must not contain class body: {sig}"
    );
}

#[test]
fn c_function_signature_excludes_body() {
    let adapter = CAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("handler.c"),
        "void handle(int fd) {\n  close(fd);\n}\n",
    );
    let sym = find_symbol(&symbols, SymbolKind::Function, "handle");
    let sig = sym.signature.as_deref().unwrap();
    assert!(
        !sig.contains("close(fd)"),
        "signature must not contain body: {sig}"
    );
}

#[test]
fn csharp_method_signature_excludes_body() {
    let adapter = CSharpAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("C.cs"),
        "class C {\n  void Run() {\n    Console.Write(\"x\");\n  }\n}\n",
    );
    let sym = find_symbol(&symbols, SymbolKind::Method, "Run");
    let sig = sym.signature.as_deref().unwrap();
    assert!(
        !sig.contains("Console.Write"),
        "signature must not contain body: {sig}"
    );
}

#[test]
fn ruby_method_signature_excludes_body() {
    let adapter = RubyAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("c.rb"),
        "class C\n  def run\n    puts 'x'\n  end\nend\n",
    );
    let sym = find_symbol(&symbols, SymbolKind::Method, "run");
    let sig = sym.signature.as_deref().unwrap();
    assert!(
        !sig.contains("puts"),
        "signature must not contain body: {sig}"
    );
}

#[test]
fn zig_function_signature_excludes_body() {
    let adapter = ZigAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("init.zig"),
        "pub fn init() !void {\n    return error.Fail;\n}\n",
    );
    let sym = find_symbol(&symbols, SymbolKind::Function, "init");
    let sig = sym.signature.as_deref().unwrap();
    assert!(
        !sig.contains("return error"),
        "signature must not contain body: {sig}"
    );
}

// ---------------------------------------------------------------------------
// 1b. TypeScript method extraction
// ---------------------------------------------------------------------------

#[test]
fn ts_extracts_class_methods() {
    let adapter = TypeScriptAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("server.ts"),
        "class Server {\n  start(port: number): void {}\n  private stop(): void {}\n}\n",
    );
    let method = find_symbol(&symbols, SymbolKind::Method, "start");
    assert_eq!(method.container_name.as_deref(), Some("Server"));
    find_symbol(&symbols, SymbolKind::Method, "stop");
}

// ---------------------------------------------------------------------------
// 1c. C++ namespace function is Function, not Method
// ---------------------------------------------------------------------------

#[test]
fn cpp_namespace_function_is_function_not_method() {
    let adapter = CppAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("net.cpp"),
        "namespace net {\n  void dial(const char* addr) {\n    connect();\n  }\n}\n",
    );
    let sym = find_symbol(&symbols, SymbolKind::Function, "dial");
    assert_eq!(sym.kind, SymbolKind::Function);
    assert_eq!(sym.container_name.as_deref(), Some("net"));
}

#[test]
fn cpp_class_method_is_method() {
    let adapter = CppAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("server.cpp"),
        "class Server {\n  void start() {}\n};\n",
    );
    let sym = find_symbol(&symbols, SymbolKind::Method, "start");
    assert_eq!(sym.container_name.as_deref(), Some("Server"));
}

// ---------------------------------------------------------------------------
// 1d. TypeScript type alias with object literal is not truncated
// ---------------------------------------------------------------------------

#[test]
fn ts_type_alias_with_object_literal_preserves_braces() {
    let adapter = TypeScriptAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("types.ts"),
        "type Handler = { run(): void }\n",
    );
    let sym = find_symbol(&symbols, SymbolKind::TypeAlias, "Handler");
    let sig = sym.signature.as_deref().unwrap();
    assert!(
        sig.contains("run(): void"),
        "object literal type must be preserved in signature: {sig}"
    );
}

// ---------------------------------------------------------------------------
// 1e. TypeScript export does not set visibility = pub
// ---------------------------------------------------------------------------

#[test]
fn ts_export_does_not_set_pub_visibility() {
    let adapter = TypeScriptAdapter;
    let symbols = adapter.extract_symbols(
        Path::new("api.ts"),
        "export function handle(): void {}\n",
    );
    let sym = find_symbol(&symbols, SymbolKind::Function, "handle");
    assert_eq!(sym.visibility, None);
}

// ---------------------------------------------------------------------------
// 1f. normalize_language rejects phantom languages
// ---------------------------------------------------------------------------

#[test]
fn normalize_language_rejects_unsupported() {
    assert_eq!(normalize_language("javascript"), None);
    assert_eq!(normalize_language("js"), None);
    assert_eq!(normalize_language("php"), None);
    assert_eq!(normalize_language("swift"), None);
    assert_eq!(normalize_language("kotlin"), None);
    assert_eq!(normalize_language("kt"), None);
}
