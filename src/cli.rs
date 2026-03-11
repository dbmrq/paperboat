//! Command-line interface handling.
//!
//! This module contains CLI argument parsing, goal resolution, and help text.
//! It provides the main entry point utilities for the paperboat application.

use crate::backend::BackendConfig;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

/// Parsed command-line arguments.
#[allow(clippy::struct_excessive_bools)]
pub struct CliArgs {
    /// Show help and exit
    pub help_mode: bool,
    /// Disable TUI mode and run headlessly
    pub headless_mode: bool,
    /// Run in MCP server mode
    pub mcp_server_mode: bool,
    /// Validate config files only (don't run the app)
    pub validate_config_mode: bool,
    /// Enable JSON formatted logs (for machine parsing)
    pub json_logs: bool,
    /// Enable metrics collection (can also be set via `PAPERBOAT_METRICS` env var)
    pub metrics_enabled: bool,
    /// Backend configuration (backend and optional transport).
    ///
    /// Parsed from `--backend` flag in format "backend:transport".
    /// Examples: "cursor", "cursor:cli", "cursor:acp", "auggie:acp"
    pub backend_config: Option<BackendConfig>,
    /// The goal/task to accomplish
    pub goal: Option<String>,
}

/// Parse command-line arguments and extract flags and the goal.
///
/// # Goal Resolution
///
/// The goal argument is resolved in this priority order:
/// 1. If argument is a file path that exists → read file contents as goal
/// 2. Otherwise → use argument as direct prompt string
/// 3. No argument → `None` (handled later with interactive prompt)
///
/// # Backend Selection
///
/// The `--backend` flag supports an optional transport suffix:
/// - `--backend cursor` - Cursor with default transport (CLI)
/// - `--backend cursor:cli` - Cursor with CLI transport (explicit)
/// - `--backend cursor:acp` - Cursor with ACP transport
/// - `--backend auggie` - Auggie with default transport (ACP)
/// - `--backend auggie:acp` - Auggie with ACP transport (explicit)
pub fn parse_args(args: &[String]) -> CliArgs {
    // --help or -h shows help and exits
    let help_mode = args.contains(&"--help".to_string()) || args.contains(&"-h".to_string());
    // --headless disables TUI mode (TUI is enabled by default)
    let headless_mode = args.contains(&"--headless".to_string());
    let mcp_server_mode = args.get(1).is_some_and(|a| a == "--mcp-server");
    let validate_config_mode = args.contains(&"--validate-config".to_string());
    // --json-logs enables JSON formatted logs (can also be set via PAPERBOAT_JSON_LOGS env var)
    let json_logs = args.contains(&"--json-logs".to_string())
        || std::env::var("PAPERBOAT_JSON_LOGS").is_ok_and(|v| v == "1" || v == "true");
    // --metrics enables metrics collection (can also be set via PAPERBOAT_METRICS env var)
    let metrics_enabled = args.contains(&"--metrics".to_string())
        || std::env::var("PAPERBOAT_METRICS").is_ok_and(|v| v == "1" || v == "true");

    // --backend <name[:transport]> selects the backend and optional transport
    // Examples: cursor, cursor:cli, cursor:acp, auggie, auggie:acp
    let backend_config = args
        .iter()
        .position(|a| a == "--backend")
        .and_then(|i| args.get(i + 1))
        .map(|backend_str| {
            match BackendConfig::parse(backend_str) {
                Ok(config) => config,
                Err(e) => {
                    // Print error and exit immediately for invalid backend config
                    eprintln!(
                        "❌ Invalid --backend value '{backend_str}': {e}\n\n\
                        Valid backends:\n  \
                          auggie        Augment's Auggie CLI (default transport: acp)\n  \
                          cursor        Cursor's agent CLI (default transport: cli)\n\n\
                        Transport options (optional suffix):\n  \
                          cursor:cli    CLI transport (better MCP support, default for Cursor)\n  \
                          cursor:acp    ACP transport (JSON-RPC protocol)\n  \
                          auggie:acp    ACP transport (only option for Auggie)\n\n\
                        Examples:\n  \
                          --backend cursor\n  \
                          --backend cursor:cli\n  \
                          --backend cursor:acp\n  \
                          --backend auggie"
                    );
                    std::process::exit(1);
                }
            }
        });

    // Goal is the first non-flag argument after the program name
    // Skip arguments that are values for flags (like --backend value)
    let flags_with_values = ["--backend", "--socket"];
    let goal_arg = args
        .iter()
        .enumerate()
        .skip(1)
        .filter(|(i, arg)| {
            // Skip flag arguments
            if arg.starts_with("--") || arg.starts_with('-') {
                return false;
            }
            // Skip values of flags that take arguments
            if *i > 0 {
                if let Some(prev) = args.get(i - 1) {
                    if flags_with_values.contains(&prev.as_str()) {
                        return false;
                    }
                }
            }
            true
        })
        .map(|(_, arg)| arg.clone())
        .next();

    // Resolve goal: check if argument is a file path or direct prompt
    let goal = goal_arg.and_then(|arg| resolve_goal_argument(&arg));

    CliArgs {
        help_mode,
        headless_mode,
        mcp_server_mode,
        validate_config_mode,
        json_logs,
        metrics_enabled,
        backend_config,
        goal,
    }
}

/// Resolves a goal argument, checking if it's a file path or a direct prompt.
///
/// Returns:
/// - `Some(content)` if argument is a file that exists (reads file content)
/// - `Some(arg)` if argument is not a file (uses as direct prompt)
/// - Exits with error if file exists but cannot be read
pub fn resolve_goal_argument(arg: &str) -> Option<String> {
    let path = PathBuf::from(arg);

    // Check if the argument looks like a file path and exists
    if path.exists() {
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                let trimmed = content.trim();
                if trimmed.is_empty() {
                    eprintln!("❌ Goal file is empty: {}", path.display());
                    std::process::exit(1);
                }
                eprintln!("📄 Reading goal from file: {}", path.display());
                Some(trimmed.to_string())
            }
            Err(e) => {
                eprintln!("❌ Failed to read goal file '{}': {}", path.display(), e);
                std::process::exit(1);
            }
        }
    } else {
        // Not a file, use as direct prompt
        let trimmed = arg.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }
}

/// Prompts the user for a goal interactively via stdin.
///
/// This function handles:
/// - Checking if stdin is a terminal (returns None if not)
/// - Timeout handling (60 seconds)
/// - Ctrl+C/Ctrl+D handling (exits gracefully)
/// - Empty input handling
///
/// Returns `Some(goal)` if user enters valid input, `None` if stdin is not a terminal.
pub fn prompt_goal_interactively() -> Option<String> {
    // Don't prompt if stdin is not a terminal
    if !std::io::IsTerminal::is_terminal(&io::stdin()) {
        return None;
    }

    println!("\n🎯 No goal provided. Please enter your task:");
    println!("(Enter your task/goal. Press Enter twice or Ctrl+D when done)\n");
    print!("> ");
    io::stdout().flush().ok()?;

    // Use a channel with timeout for reading input
    let (tx, rx) = mpsc::channel();
    let stdin_thread = std::thread::spawn(move || {
        let stdin = io::stdin();
        let mut lines = Vec::new();

        // Read multiple lines until empty line or EOF
        loop {
            let mut line = String::new();
            let read_result = stdin.lock().read_line(&mut line);
            match read_result {
                Ok(0) => {
                    // EOF (Ctrl+D on Unix)
                    break;
                }
                Ok(_) => {
                    // Check for empty line (just Enter pressed) to end input
                    if line.trim().is_empty() && !lines.is_empty() {
                        break;
                    }
                    lines.push(line);
                }
                Err(_) => {
                    // Read error (possibly Ctrl+C)
                    let _ = tx.send(None);
                    return;
                }
            }
        }

        let combined = lines.concat();
        let trimmed = combined.trim();
        if trimmed.is_empty() {
            let _ = tx.send(None);
        } else {
            let _ = tx.send(Some(trimmed.to_string()));
        }
    });

    // Wait for input with a 60-second timeout
    let result = match rx.recv_timeout(Duration::from_secs(60)) {
        Ok(Some(goal)) => {
            println!("\n✓ Goal received\n");
            Some(goal)
        }
        Ok(None) => {
            // Empty input or error
            println!();
            eprintln!("\n❌ No goal provided. Exiting.");
            std::process::exit(1);
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            println!();
            eprintln!("\n⏱ Input timeout. Exiting.");
            std::process::exit(1);
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            // Thread panicked or disconnected
            println!();
            eprintln!("\n❌ Input error. Exiting.");
            std::process::exit(1);
        }
    };

    // Wait for the stdin thread to finish
    let _ = stdin_thread.join();

    result
}

/// Print help message.
pub fn print_help() {
    println!(
        r#"paperboat - AI-powered task automation

USAGE:
    paperboat [OPTIONS] [GOAL]

ARGS:
    <GOAL>    Task description as a string, or path to a file containing the goal
              If omitted, paperboat will prompt interactively for input

EXAMPLES:
    paperboat "Fix all TODO comments in src/"
    paperboat plan.txt
    paperboat

OPTIONS:
    -h, --help              Show this help message and exit
    --backend <NAME>        Select AI backend (auggie, cursor, cursor:cli, cursor:acp)
    --headless              Disable TUI mode and run in console mode
    --json-logs             Enable JSON formatted logs
    --metrics               Enable Prometheus metrics collection
    --validate-config       Validate configuration files and exit

ENVIRONMENT:
    PAPERBOAT_BACKEND       Set default backend (same format as --backend)
    PAPERBOAT_LOG_DIR       Set custom log directory (default: .paperboat/logs)
    PAPERBOAT_JSON_LOGS     Enable JSON logs (set to "1" or "true")
    PAPERBOAT_METRICS       Enable metrics (set to "1" or "true")
    PAPERBOAT_METRICS_PORT  Set Prometheus metrics port (default: 9090)

CONFIG:
    Configuration files are loaded from:
      - ~/.paperboat/         User-level configuration
      - .paperboat/           Project-level configuration (overrides user-level)

For more information, see https://github.com/your-repo/paperboat
"#
    );
}

/// Print usage error message when no goal is provided in non-interactive mode.
pub fn print_no_goal_error() {
    eprintln!(
        "❌ No goal provided.\n\n\
        Usage:\n  \
          paperboat \"Your task description\"     # Direct prompt\n  \
          paperboat path/to/goal.txt            # Read goal from file\n  \
          paperboat                             # Interactive mode (terminal required)\n\n\
        Options:\n  \
          --backend <name>    Select AI backend (auggie, cursor)\n  \
          --headless          Disable TUI mode\n  \
          --help              Show help"
    );
}
