# Collie Codex Plugin

The repository includes a Codex plugin bundle at `plugins/collie`.

This plugin gives Codex users a reusable Collie skill that:

- installs or upgrades the latest published `collie-search` crate
- teaches the same search workflow exposed by `collie skill`
- works across repositories once installed into a personal marketplace

Presentation: https://suleymanozkeskin.github.io/collie/

## Current distribution model

Today, Codex plugins are installed from a marketplace file.
Self-serve publishing to the official public Codex Plugin Directory is not available yet, so the practical distribution path is to install this plugin from GitHub into a personal Codex marketplace.

The plugin metadata already points at the public presentation plus repo-hosted privacy and terms documents. The remaining missing polish item is branding assets such as an icon, logo, and optional screenshots.

## Install for personal Codex use

1. Clone the repository:

```sh
git clone https://github.com/suleymanozkeskin/collie.git
cd collie
```

2. Install the plugin into your personal Codex marketplace:

```sh
python3 scripts/install_personal_codex_plugin.py
```

This copies the plugin to `~/plugins/collie` and creates or updates `~/.agents/plugins/marketplace.json` with a `collie` marketplace entry.

3. Restart Codex.

4. Open the plugin UI in Codex and install or enable `Collie`.

## Install for this repository only

If you are already working inside this repository, Codex can load the repo-local marketplace at `.agents/plugins/marketplace.json`.

Restart Codex after updating plugin files so the local install cache refreshes.

## Updating the plugin skill

The plugin skill is generated from the canonical repo skill:

- source: `.agents/skills/SKILL.md`
- generated plugin copy: `plugins/collie/skills/use-collie/SKILL.md`

Regenerate manually with:

```sh
python3 scripts/sync_plugin_skill.py
```

The shared pre-commit hook also keeps the generated plugin skill in sync.
