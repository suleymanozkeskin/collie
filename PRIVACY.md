# Collie Plugin Privacy

Last updated: March 27, 2026

## Summary

The Collie Codex plugin is a local workflow plugin.
It does not run a separate hosted service operated by the plugin author.

## What the plugin does

The plugin provides a Codex skill that can:

- run local shell commands such as `cargo install collie-search --locked`
- run the local `collie` CLI in repositories you ask Codex to work with
- read and summarize local repository files through Codex

## Data handling

The plugin itself does not collect, sell, or store a separate database of user content.

When you use the plugin:

- Codex may read files in your local workspace, subject to your Codex permissions and approvals
- `cargo install` may contact the Rust package ecosystem to download the published `collie-search` crate
- normal GitHub or documentation links may be opened if you or Codex choose to browse them

## Third-party services

Depending on how you use the plugin, data may be handled by:

- OpenAI Codex
- crates.io
- static documentation hosted on GitHub or GitHub Pages

Their handling of data is governed by their own policies.

## No background collection

The plugin does not include its own telemetry pipeline, analytics collector, or remote API operated by the plugin author.

## Contact

Repository: https://github.com/suleymanozkeskin/collie
