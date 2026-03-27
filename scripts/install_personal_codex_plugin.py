#!/usr/bin/env python3
"""Install the Collie Codex plugin into the current user's personal marketplace."""

from __future__ import annotations

import json
import shutil
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parent.parent
SOURCE_PLUGIN = REPO_ROOT / "plugins" / "collie"
TARGET_PLUGIN = Path.home() / "plugins" / "collie"
MARKETPLACE_PATH = Path.home() / ".agents" / "plugins" / "marketplace.json"

PLUGIN_ENTRY = {
    "name": "collie",
    "source": {
        "source": "local",
        "path": "./plugins/collie",
    },
    "policy": {
        "installation": "AVAILABLE",
        "authentication": "ON_INSTALL",
    },
    "category": "Developer Tools",
}


def load_marketplace(path: Path) -> dict:
    if not path.exists():
        return {
            "name": "personal-marketplace",
            "interface": {"displayName": "Personal Plugins"},
            "plugins": [],
        }

    return json.loads(path.read_text(encoding="utf-8"))


def update_marketplace(data: dict) -> dict:
    data.setdefault("name", "personal-marketplace")
    interface = data.setdefault("interface", {})
    interface.setdefault("displayName", "Personal Plugins")
    plugins = data.setdefault("plugins", [])

    replaced = False
    for index, entry in enumerate(plugins):
        if entry.get("name") == PLUGIN_ENTRY["name"]:
            plugins[index] = PLUGIN_ENTRY
            replaced = True
            break

    if not replaced:
        plugins.append(PLUGIN_ENTRY)

    return data


def main() -> None:
    if not SOURCE_PLUGIN.exists():
        raise SystemExit(f"plugin source not found: {SOURCE_PLUGIN}")

    TARGET_PLUGIN.parent.mkdir(parents=True, exist_ok=True)
    shutil.copytree(SOURCE_PLUGIN, TARGET_PLUGIN, dirs_exist_ok=True)

    MARKETPLACE_PATH.parent.mkdir(parents=True, exist_ok=True)
    marketplace = update_marketplace(load_marketplace(MARKETPLACE_PATH))
    MARKETPLACE_PATH.write_text(json.dumps(marketplace, indent=2) + "\n", encoding="utf-8")

    print(f"Installed plugin files to {TARGET_PLUGIN}")
    print(f"Updated personal marketplace at {MARKETPLACE_PATH}")
    print("Restart Codex, then install or enable the Collie plugin from your personal marketplace.")


if __name__ == "__main__":
    main()
