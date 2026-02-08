use clap::{Parser, Subcommand};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

const DEFAULT_PLUGIN: &str = "zellij-tools";
const HEARTBEAT_INTERVAL: Duration = Duration::from_millis(500);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

fn resolve_plugin(cli_override: Option<&str>) -> String {
    cli_override
        .map(String::from)
        .or_else(|| std::env::var("ZELLIJ_TOOLS_PLUGIN").ok())
        .unwrap_or_else(|| DEFAULT_PLUGIN.to_string())
}

#[derive(Parser)]
#[command(name = "zellij-tools")]
#[command(about = "CLI utilities for zellij-tools plugin")]
struct Cli {
    /// Plugin reference (name alias or file: path) [env: ZELLIJ_TOOLS_PLUGIN]
    #[arg(long = "zellij-plugin", global = true)]
    plugin: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Focus a pane or tab
    Focus {
        #[command(subcommand)]
        target: FocusTarget,
    },
    /// Subscribe to the event stream from the zellij-tools plugin
    Subscribe {
        /// Include full object details in each event
        #[arg(long)]
        full: bool,
    },
    /// Print the session tree (tabs, panes, stable IDs) as JSON
    Tree,
}

#[derive(Subcommand)]
enum FocusTarget {
    /// Focus a pane by ID
    Pane {
        pane_id: u32,
        /// Focus a plugin pane ID (mutually exclusive with --terminal)
        #[arg(short = 'p', long = "plugin", conflicts_with = "terminal_pane")]
        plugin_pane: bool,
        /// Focus a terminal pane ID (default)
        #[arg(short = 't', long = "terminal", conflicts_with = "plugin_pane")]
        terminal_pane: bool,
    },
    /// Focus a tab (position by default, or stable ID with --id)
    Tab {
        tab: u64,
        /// Interpret the value as stable tab ID (mutually exclusive with --position)
        #[arg(short = 'i', long, conflicts_with = "position")]
        id: bool,
        /// Interpret the value as tab position (default, 1-based)
        #[arg(short = 'p', long, conflicts_with = "id")]
        position: bool,
    },
}

fn send_pipe_message(plugin: &str, msg: &str) -> std::io::Result<()> {
    let status = Command::new("zellij")
        .args(["pipe", "--plugin", plugin, "--", msg])
        .status()?;

    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "zellij pipe exited with status {}",
            status
        )))
    }
}

fn focus(plugin: &str, target: FocusTarget) -> std::io::Result<()> {
    let msg = match target {
        FocusTarget::Pane {
            pane_id,
            plugin_pane,
            ..
        } => {
            let pane = if plugin_pane {
                format!("plugin_{}", pane_id)
            } else {
                format!("terminal_{}", pane_id)
            };
            format!("zellij-tools::focus-pane::{}", pane)
        }
        FocusTarget::Tab { tab, id, .. } => {
            if id {
                format!("zellij-tools::focus-tab::id::{}", tab)
            } else {
                format!("zellij-tools::focus-tab::position::{}", tab)
            }
        }
    };

    send_pipe_message(plugin, &msg)
}

fn subscribe(plugin: &str, full: bool) -> std::io::Result<()> {
    let pipe_name = format!("zellij-tools-events-{}", uuid::Uuid::new_v4());

    let subscribe_msg = if full {
        "zellij-tools::subscribe::full".to_string()
    } else {
        "zellij-tools::subscribe".to_string()
    };

    // Spawn zellij pipe with subscribe message as positional payload.
    let mut child = Command::new("zellij")
        .args([
            "pipe",
            "--name",
            &pipe_name,
            "--plugin",
            plugin,
            "--",
            &subscribe_msg,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;

    let stdin = child.stdin.take().expect("Failed to open stdin");
    let stdout = child.stdout.take().expect("Failed to open stdout");

    // Wrap child in Arc<Mutex> so Ctrl+C handler can kill it
    let child = Arc::new(Mutex::new(child));

    // Set up Ctrl+C handler
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    let pipe_name_clone = pipe_name.clone();
    let plugin_clone = plugin.to_string();
    let child_clone = Arc::clone(&child);

    ctrlc::set_handler(move || {
        eprintln!("\nUnsubscribing...");
        // Send unsubscribe (best effort, ignore errors)
        let _ = Command::new("zellij")
            .args([
                "pipe",
                "--plugin",
                &plugin_clone,
                "--",
                &format!("zellij-tools::unsubscribe::{}", pipe_name_clone),
            ])
            .status();
        r.store(false, Ordering::SeqCst);
        // Kill child process to unblock reader.lines()
        if let Ok(mut child) = child_clone.lock() {
            let _ = child.kill();
        }
    })
    .expect("Error setting Ctrl-C handler");

    // Spawn heartbeat thread: sends empty lines to stdin to trigger pipe()
    // calls on the plugin, which flushes buffered cli_pipe_output data.
    // Without this, events emitted from update() would never reach the CLI.
    let heartbeat_running = running.clone();
    let stdin = Arc::new(Mutex::new(stdin));
    let heartbeat_stdin = Arc::clone(&stdin);
    std::thread::spawn(move || {
        while heartbeat_running.load(Ordering::SeqCst) {
            std::thread::sleep(HEARTBEAT_INTERVAL);
            if let Ok(mut stdin) = heartbeat_stdin.lock() {
                if stdin.write_all(b"\n").is_err() || stdin.flush().is_err() {
                    break;
                }
            }
        }
    });

    // Spawn reader thread that sends lines over a channel
    let (tx, rx) = mpsc::channel();
    let reader_running = running.clone();
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            if !reader_running.load(Ordering::SeqCst) {
                break;
            }
            match line {
                Ok(l) if !l.is_empty() => {
                    if tx.send(l).is_err() {
                        break;
                    }
                }
                Ok(_) => continue,
                Err(_) => break,
            }
        }
    });

    // Phase 1: Wait for ACK with timeout
    match rx.recv_timeout(CONNECT_TIMEOUT) {
        Ok(line) => {
            if line.contains(r#""Ack""#) {
                // ACK received, connected successfully
            } else {
                // First line isn't an ACK — treat it as a normal event
                // (backwards compat with older plugin versions)
                println!("{}", line);
            }
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            eprintln!(
                "error: timed out waiting for plugin to respond ({}s)",
                CONNECT_TIMEOUT.as_secs()
            );
            eprintln!("hint: is the zellij-tools plugin loaded?");
            if let Ok(mut child) = child.lock() {
                let _ = child.kill();
                let _ = child.wait();
            }
            std::process::exit(1);
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            eprintln!("error: plugin connection closed before responding");
            if let Ok(mut child) = child.lock() {
                let _ = child.wait();
            }
            std::process::exit(1);
        }
    }

    // Phase 2: Normal event loop (no timeout)
    while running.load(Ordering::SeqCst) {
        match rx.recv() {
            Ok(line) => {
                if line.contains(r#""Ack""#) {
                    continue; // Skip any duplicate ACKs
                }
                println!("{}", line);
            }
            Err(_) => break,
        }
    }

    // Cleanup
    if let Ok(mut child) = child.lock() {
        let _ = child.wait();
    }
    Ok(())
}

/// Send a one-shot request to the plugin and read a single JSON response.
fn tree(plugin: &str) -> std::io::Result<()> {
    let pipe_name = format!("zellij-tools-tree-{}", uuid::Uuid::new_v4());
    let msg = "zellij-tools::tree".to_string();

    let mut child = Command::new("zellij")
        .args(["pipe", "--name", &pipe_name, "--plugin", plugin, "--", &msg])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;

    // The tree response is emitted from pipe(), so it arrives immediately.
    // Send a single heartbeat to flush buffered output, then close stdin.
    if let Some(mut stdin) = child.stdin.take() {
        // Send one heartbeat then close stdin so the pipe closes after response.
        let _ = stdin.write_all(b"\n");
        let _ = stdin.flush();
        drop(stdin);

        if let Some(stdout) = child.stdout.take() {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                match line {
                    Ok(l) if !l.is_empty() => {
                        println!("{}", l);
                        // Got our response, we're done
                        break;
                    }
                    Ok(_) => continue,
                    Err(e) => {
                        eprintln!("Error reading: {}", e);
                        break;
                    }
                }
            }
        }
    }

    let _ = child.kill();
    let _ = child.wait();
    Ok(())
}

fn main() {
    let cli = Cli::parse();

    let plugin = resolve_plugin(cli.plugin.as_deref());

    let result = match cli.command {
        Commands::Focus { target } => focus(&plugin, target),
        Commands::Subscribe { full } => subscribe(&plugin, full),
        Commands::Tree => tree(&plugin),
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_focus_pane_command() {
        let cli = Cli::try_parse_from(["zellij-tools", "focus", "pane", "2"]).unwrap();

        match cli.command {
            Commands::Focus {
                target:
                    FocusTarget::Pane {
                        pane_id,
                        plugin_pane,
                        terminal_pane,
                    },
            } => {
                assert_eq!(pane_id, 2);
                assert!(!plugin_pane);
                assert!(!terminal_pane);
            }
            _ => panic!("expected focus pane command"),
        }
    }

    #[test]
    fn parses_focus_pane_plugin_flag() {
        let cli = Cli::try_parse_from(["zellij-tools", "focus", "pane", "2", "--plugin"]).unwrap();

        match cli.command {
            Commands::Focus {
                target: FocusTarget::Pane { plugin_pane, .. },
            } => assert!(plugin_pane),
            _ => panic!("expected focus pane command"),
        }
    }

    #[test]
    fn parses_focus_tab_command() {
        let cli = Cli::try_parse_from(["zellij-tools", "focus", "tab", "3"]).unwrap();

        match cli.command {
            Commands::Focus {
                target: FocusTarget::Tab { tab, id, position },
            } => {
                assert_eq!(tab, 3);
                assert!(!id);
                assert!(!position);
            }
            _ => panic!("expected focus tab command"),
        }
    }

    #[test]
    fn parses_focus_tab_id_flag() {
        let cli = Cli::try_parse_from(["zellij-tools", "focus", "tab", "42", "--id"]).unwrap();

        match cli.command {
            Commands::Focus {
                target: FocusTarget::Tab { tab, id, .. },
            } => {
                assert_eq!(tab, 42);
                assert!(id);
            }
            _ => panic!("expected focus tab command"),
        }
    }

    #[test]
    fn rejects_mutually_exclusive_focus_flags() {
        let pane = Cli::try_parse_from([
            "zellij-tools",
            "focus",
            "pane",
            "2",
            "--plugin",
            "--terminal",
        ]);
        assert!(pane.is_err());

        let tab = Cli::try_parse_from(["zellij-tools", "focus", "tab", "2", "--id", "--position"]);
        assert!(tab.is_err());
    }
}
