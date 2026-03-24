use clap::{Parser, Subcommand};
use collie_search::cli::search::{ColorMode, OutputFormat};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "collie")]
#[command(version)]
#[command(
    about = "Index-backed code search. Faster than grep on large repos.",
    long_about = "\
Collie indexes your repository and keeps the index up to date via a \
background daemon. Searches run against the index, making them \
near-instant even on very large codebases.

QUICK START:
  collie watch .                    Start the daemon (indexes the repo)
  collie search handler             Find files containing \"handler\"
  collie search \"kind:fn handler\"   Find functions named \"handler\"
  collie search -e 'TODO|FIXME'     Regex search (index-accelerated)
  collie status .                   Check if the daemon is running
  collie stop .                     Stop the daemon

SEARCH MODES:
  Token search (default):  Matches indexed tokens. Use % for wildcards:
    collie search handler           Exact token match
    collie search 'handle%'         Prefix match
    collie search '%handler'        Suffix match
    collie search '%handle%'        Substring match
    collie search 'handle request'  Multi-term (AND)

  Symbol search:  Structured queries with kind/language/path filters:
    collie search 'kind:fn handler'
    collie search 'kind:struct Config'
    collie search 'kind:method qname:Server::run'
    collie search 'kind:fn lang:go path:pkg/ init'

  Regex search (-e):  Full regex with index acceleration:
    collie search -e 'func\\s+\\w+Handler'
    collie search -e 'TODO|FIXME|HACK'
    collie search -e 'impl.*for.*Error' -i

AI AGENT INTEGRATION:
  Use --format json for structured, parseable output:
    collie search handler --format json
    collie search -e 'TODO|FIXME' --format json
    collie search 'kind:fn handler' --format json
  Exit codes: 0 = results found, 1 = no results, 2 = error

PDF SUPPORT:
  Enable PDF text extraction in .collie/config.toml:
    [index]
    include_pdfs = true"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Search pattern (when no subcommand)
    #[arg(short = 's', long)]
    search: Option<String>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the indexing daemon for a repository
    ///
    /// Indexes all source files in the repository, then watches for changes
    /// and keeps the index up to date. The daemon runs in the background
    /// by default. Run this before searching.
    Watch {
        /// Repository path
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Run in foreground instead of daemonizing
        #[arg(long)]
        foreground: bool,

        /// Automatically restart the daemon if it crashes
        #[arg(long)]
        restart_on_crash: bool,
    },

    /// Search the index for matching files or symbols
    ///
    /// By default, searches for indexed tokens (identifiers, keywords).
    /// Use -e/--regex for full regex matching with index acceleration.
    /// Use kind:/lang:/path: prefixes for structured symbol search.
    Search {
        /// Search pattern: token, symbol query, or regex (with -e)
        ///
        /// Token patterns support % wildcards:
        ///   handler       exact match
        ///   handle%       prefix
        ///   %handler      suffix
        ///   %handle%      substring
        ///   handle request  multi-term AND
        ///
        /// Symbol queries use structured filters:
        ///   kind:fn handler
        ///   kind:struct lang:go Config
        ///   kind:method qname:Server::run
        ///   kind:fn path:src/api/ init
        #[arg(verbatim_doc_comment)]
        pattern: String,

        /// Max number of results to return
        #[arg(short = 'n', default_value = "20")]
        limit: Option<usize>,

        /// Context lines around each match (symmetric)
        #[arg(short = 'C', long)]
        context: Option<usize>,

        /// Lines of context after each match
        #[arg(short = 'A', long)]
        after_context: Option<usize>,

        /// Lines of context before each match
        #[arg(short = 'B', long)]
        before_context: Option<usize>,

        /// Suppress code snippets, show file paths only
        #[arg(long)]
        no_snippets: bool,

        /// Treat pattern as a regex (index-accelerated grep)
        ///
        /// Extracts literal fragments from the regex to narrow candidates
        /// via the index, then applies the full regex on matching files.
        #[arg(short = 'e', long = "regex")]
        is_regex: bool,

        /// Case-insensitive matching (applies to regex mode)
        #[arg(short = 'i', long)]
        ignore_case: bool,

        /// Allow . to match newlines in regex mode
        #[arg(short = 'U', long)]
        multiline: bool,

        /// Print only the paths of files with matches
        #[arg(short = 'l', long = "files-with-matches")]
        files_only: bool,

        /// Print only the count of matching files
        #[arg(short = 'c', long)]
        count: bool,

        /// Filter results by glob pattern
        ///
        /// Matches against the file path relative to the repo root.
        /// Examples: "*.go", "src/**/*.rs", "pkg/api/*"
        #[arg(short = 'g', long)]
        glob: Option<String>,

        /// When to use colored output
        #[arg(long, default_value = "auto")]
        color: ColorMode,

        /// Output format
        #[arg(long, default_value = "default")]
        format: OutputFormat,

        /// Repository path to search (default: current directory)
        ///
        /// Allows searching a repo without cd-ing into it first.
        #[arg(long, default_value = ".")]
        path: PathBuf,
    },

    /// Stop the background daemon
    Stop {
        /// Repository path
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Show daemon status and index statistics
    Status {
        /// Repository path
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Rebuild the index from scratch
    ///
    /// Stops the running daemon, deletes the existing index, and rebuilds
    /// from all source files. Use when the index seems corrupted or after
    /// major repository changes.
    Rebuild {
        /// Repository path (positional or --path)
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Repository path (alias for the positional argument)
        #[arg(long = "path", id = "path_flag")]
        path_flag: Option<PathBuf>,
    },

    /// Remove the index and all collie data for a repository
    ///
    /// Stops the daemon if running, then removes the .collie/ directory.
    /// The index will be rebuilt on the next `collie watch`.
    Clean {
        /// Repository path
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Manage configuration
    ///
    /// Configuration lives in .collie/config.toml. Use --init to create
    /// an example config file with all available options documented.
    Config {
        /// Repository path
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Create an example config file at .collie/config.toml
        #[arg(long)]
        init: bool,
    },

    /// Print the agent skill reference (search modes, JSON schema, workflows)
    Skill,

    #[command(name = "__daemon", hide = true)]
    InternalDaemon { path: PathBuf },
}

fn main() {
    match run() {
        Ok(code) => std::process::exit(code),
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(2);
        }
    }
}

/// Returns exit code: 0 = success/found, 1 = no results, 2 = error.
fn run() -> anyhow::Result<i32> {
    let cli = Cli::parse();

    match (cli.command, cli.search) {
        (
            Some(Commands::Watch {
                path,
                foreground,
                restart_on_crash,
            }),
            _,
        ) => {
            collie_search::cli::watch::run(path, foreground, restart_on_crash)?;
        }
        (
            Some(Commands::Search {
                pattern,
                limit,
                context,
                after_context,
                before_context,
                no_snippets,
                is_regex,
                ignore_case,
                multiline,
                files_only,
                count,
                glob,
                color,
                format,
                path,
            }),
            _,
        ) => {
            let found = collie_search::cli::search::run(collie_search::cli::search::SearchArgs {
                pattern,
                path: Some(path),
                limit,
                context,
                after_context,
                before_context,
                no_snippets,
                is_regex,
                ignore_case,
                multiline,
                files_only,
                count,
                glob,
                color,
                format,
            })?;
            return Ok(if found { 0 } else { 1 });
        }
        (None, Some(pattern)) => {
            let found = collie_search::cli::search::run(collie_search::cli::search::SearchArgs {
                pattern,
                ..Default::default()
            })?;
            return Ok(if found { 0 } else { 1 });
        }
        (Some(Commands::Stop { path }), _) => {
            collie_search::cli::stop::run(path)?;
        }
        (Some(Commands::Status { path, json }), _) => {
            collie_search::cli::status::run(path, json)?;
        }
        (Some(Commands::Rebuild { path, path_flag }), _) => {
            collie_search::cli::rebuild::run(path_flag.unwrap_or(path))?;
        }
        (Some(Commands::Clean { path }), _) => {
            collie_search::daemon::clean(path)?;
        }
        (Some(Commands::Config { path, init }), _) => {
            if init {
                let root = collie_search::daemon::resolve_worktree_root(path)?;
                let config_path = root.join(".collie").join("config.toml");
                if config_path.exists() {
                    println!("config already exists at {}", config_path.display());
                } else {
                    if let Some(parent) = config_path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::write(&config_path, collie_search::config::CONFIG_TEMPLATE)?;
                    println!("Created config at {}", config_path.display());
                }
            } else {
                println!("Use --init to create an example config file.");
            }
        }
        (Some(Commands::Skill), _) => {
            print!("{}", include_str!("../.agents/skills/SKILL.md"));
        }
        (Some(Commands::InternalDaemon { path }), _) => {
            collie_search::daemon::run_internal_daemon(path)?;
        }
        (None, None) => {
            Cli::parse_from(["collie", "--help"]);
        }
    }

    Ok(0)
}
