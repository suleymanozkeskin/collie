#!/usr/bin/env python3
"""Generate the plugin skill from the canonical Collie skill reference."""

from __future__ import annotations

from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parent.parent
CANONICAL_SKILL = REPO_ROOT / ".agents" / "skills" / "SKILL.md"
PLUGIN_SKILL = REPO_ROOT / "plugins" / "collie" / "skills" / "use-collie" / "SKILL.md"

PLUGIN_FRONTMATTER = """---
name: use-collie
description: Install or upgrade the latest published Collie CLI, then use it for fast, index-backed code search.
---
"""

INSTALL_SECTION = """
# Use Collie

<!-- Generated from `/.agents/skills/SKILL.md` by `scripts/sync_plugin_skill.py`. -->

Use this skill when the user wants fast local code search, symbol lookup, regex search, or Collie setup in a repository.

## Install or upgrade

1. Check whether `cargo` is available.
2. Check whether `collie` is already installed with `collie --version`.
3. If `collie` is missing, or if you need to ensure the latest published version is installed, run:

```sh
cargo install collie-search --locked
```

This installs the `collie` command from the latest published `collie-search` crate.

4. Verify the installed version:

```sh
collie --version
```

If `cargo` is unavailable, tell the user Rust/Cargo must be installed before this plugin can install Collie.
"""


def strip_frontmatter(text: str) -> str:
    if not text.startswith("---\n"):
        return text.lstrip()

    closing = text.find("\n---\n", 4)
    if closing == -1:
        raise ValueError(f"frontmatter is not closed in {CANONICAL_SKILL}")

    return text[closing + len("\n---\n") :].lstrip()


def main() -> None:
    canonical = CANONICAL_SKILL.read_text(encoding="utf-8")
    body = strip_frontmatter(canonical)
    output = f"{PLUGIN_FRONTMATTER}\n{INSTALL_SECTION.strip()}\n\n{body}"
    PLUGIN_SKILL.write_text(output, encoding="utf-8")


if __name__ == "__main__":
    main()
