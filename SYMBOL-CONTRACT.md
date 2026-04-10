# Symbol Contract

This document defines the normative semantics for every field in a Collie
`Symbol`. All adapters must conform. See `CORRECTION-PLAN.MD` for rationale.

## Fields

### `kind`

The symbol category. Allowed values:

`Function` `Method` `Class` `Struct` `Enum` `Interface` `Trait`
`Variable` `Field` `Property` `Constant` `Module` `TypeAlias` `Import`

Rules:
- `Function` — callable not attached to a type/class receiver.
- `Method` — callable semantically attached to a type or class.
  Namespace-scoped free functions remain `Function`.
- `kind:fn` expands to `Function | Method`.

### `name`

Local symbol name as written in source. No qualification, no type/parameter
text.

### `container_name`

Immediate owning semantic container, if any. Absent when there is no
meaningful semantic owner.

### `qualified_name`

Current behavior: the immediate semantic container joined with the local name
using `::`. Example: `Server::start`, `net::dial`.

Full ancestry beyond one container level is planned follow-up work and is not
yet guaranteed by the current implementation.

### `signature`

Compact declaration header. **No implementation body.**

Examples:
- Go: `func (s *Server) Start(ctx context.Context) error`
- Rust: `pub fn start(&self) -> Result<()>`
- Python: `def start(self, request) -> Response:`
- TypeScript: `function login(req: Request): Response`

Rules:
- No function or method body.
- No class or struct member list.
- For multi-line declarations, header portion only.
- May be absent when a compact declaration is not meaningful.

### `visibility`

Source-level access modifier. Allowed values: `pub`, `private`, `protected`,
`internal`.

Rules:
- Use only when the language has a real accessibility concept.
- Do not equate module export with `pub` (e.g. TypeScript `export`).
- Leave empty rather than encode wrong semantics.

### `line_start`, `line_end`, `byte_start`, `byte_end`

Source span for the full declaration node (may cover more than `signature`).

### `doc`

Extracted documentation. Future work.

## Supported Languages

go, rust, python, typescript, java, c, cpp, csharp, ruby, zig

Only these languages have active adapters. Unsupported `lang:` values are
rejected explicitly rather than silently degrading into a name search.

## Schema Version

`SYMBOL_SCHEMA_VERSION` in `src/storage/generation.rs` must be bumped whenever
symbol extraction semantics change. Existing indexes with a stale or missing
version trigger a full rebuild.
