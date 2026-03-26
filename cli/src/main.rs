use clap::{Parser, Subcommand};
use serde::Serialize;
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

    /// Zellij session to target [env: ZELLIJ_SESSION_NAME]
    #[arg(short = 's', long = "session", global = true)]
    session: Option<String>,

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
    /// Control scratchpad panes
    Scratchpad {
        #[command(subcommand)]
        action: ScratchpadAction,
    },
    /// Subscribe to the event stream from the zellij-tools plugin
    Subscribe {
        /// Include full object details in each event
        #[arg(long)]
        full: bool,
        /// Filter by canonical event names (eg PaneFocused,TabMoved)
        #[arg(long = "event", value_delimiter = ',', num_args = 1..)]
        events: Vec<SubscribeEventKind>,
        /// Filter by terminal pane IDs
        #[arg(long = "pane-id", value_delimiter = ',', num_args = 1..)]
        pane_ids: Vec<u32>,
        /// Filter by plugin pane IDs
        #[arg(long = "plugin-pane-id", value_delimiter = ',', num_args = 1..)]
        plugin_pane_ids: Vec<u32>,
        /// Filter by tab IDs
        #[arg(long = "tab-id", value_delimiter = ',', num_args = 1..)]
        tab_ids: Vec<usize>,
    },
    /// Print the session tree (tabs, panes, tab IDs) as JSON
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
    /// Focus a tab (position by default, or tab ID with --id)
    Tab {
        tab: usize,
        /// Interpret the value as tab ID (mutually exclusive with --position)
        #[arg(short = 'i', long, conflicts_with = "position")]
        id: bool,
        /// Interpret the value as tab position (default, 1-based)
        #[arg(short = 'p', long, conflicts_with = "id")]
        position: bool,
    },
}

#[derive(Subcommand)]
enum ScratchpadAction {
    /// Toggle a scratchpad (show if hidden, hide if focused)
    Toggle {
        /// Scratchpad name (if omitted, toggles the last-focused scratchpad)
        name: Option<String>,
    },
    /// Show a scratchpad
    Show {
        /// Scratchpad name
        name: String,
    },
    /// Hide a scratchpad
    Hide {
        /// Scratchpad name
        name: String,
    },
    /// Close a scratchpad (terminates the pane)
    Close {
        /// Scratchpad name
        name: String,
    },
    /// List scratchpads as JSON
    List {
        /// Only list specific scratchpads by name
        names: Vec<String>,
        /// Filter to a specific tab by tab ID
        #[arg(long = "tab")]
        tab_id: Option<usize>,
        /// Include full pane info for each instance
        #[arg(long)]
        full: bool,
    },
}

#[derive(clap::ValueEnum, Clone, Debug, PartialEq, Eq)]
enum SubscribeEventKind {
    #[value(name = "PaneFocused")]
    PaneFocused,
    #[value(name = "PaneUnfocused")]
    PaneUnfocused,
    #[value(name = "PaneOpened")]
    PaneOpened,
    #[value(name = "PaneClosed")]
    PaneClosed,
    #[value(name = "TabFocused")]
    TabFocused,
    #[value(name = "TabUnfocused")]
    TabUnfocused,
    #[value(name = "TabCreated")]
    TabCreated,
    #[value(name = "TabClosed")]
    TabClosed,
    #[value(name = "TabMoved")]
    TabMoved,
}

impl SubscribeEventKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::PaneFocused => "PaneFocused",
            Self::PaneUnfocused => "PaneUnfocused",
            Self::PaneOpened => "PaneOpened",
            Self::PaneClosed => "PaneClosed",
            Self::TabFocused => "TabFocused",
            Self::TabUnfocused => "TabUnfocused",
            Self::TabCreated => "TabCreated",
            Self::TabClosed => "TabClosed",
            Self::TabMoved => "TabMoved",
        }
    }
}

#[derive(Debug, Serialize)]
struct SubscribeInitSpec {
    full: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    events: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pane_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tab_ids: Option<Vec<usize>>,
}

fn build_subscribe_init_json(
    full: bool,
    events: &[SubscribeEventKind],
    pane_ids: &[u32],
    plugin_pane_ids: &[u32],
    tab_ids: &[usize],
) -> String {
    let typed_pane_ids: Vec<String> = pane_ids
        .iter()
        .map(|id| format!("terminal_{id}"))
        .chain(plugin_pane_ids.iter().map(|id| format!("plugin_{id}")))
        .collect();

    let spec = SubscribeInitSpec {
        full,
        events: (!events.is_empty())
            .then(|| events.iter().map(|e| e.as_str().to_string()).collect()),
        pane_ids: (!typed_pane_ids.is_empty()).then_some(typed_pane_ids),
        tab_ids: (!tab_ids.is_empty()).then(|| tab_ids.to_vec()),
    };
    serde_json::to_string(&spec).expect("subscribe init json should serialize")
}

/// Build a `Command` for `zellij`, optionally targeting a specific session.
fn zellij_cmd(session: Option<&str>) -> Command {
    let mut cmd = Command::new("zellij");
    if let Some(s) = session {
        cmd.env("ZELLIJ_SESSION_NAME", s);
    }
    cmd
}

fn send_pipe_message(plugin: &str, msg: &str, session: Option<&str>) -> std::io::Result<()> {
    let status = zellij_cmd(session)
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

fn scratchpad(
    plugin: &str,
    action: ScratchpadAction,
    session: Option<&str>,
) -> std::io::Result<()> {
    let msg = match action {
        ScratchpadAction::Toggle { name: Some(name) } => {
            format!("zellij-tools::scratchpad::toggle::{}", name)
        }
        ScratchpadAction::Toggle { name: None } => "zellij-tools::scratchpad::toggle".to_string(),
        ScratchpadAction::Show { name } => {
            format!("zellij-tools::scratchpad::show::{}", name)
        }
        ScratchpadAction::Hide { name } => {
            format!("zellij-tools::scratchpad::hide::{}", name)
        }
        ScratchpadAction::Close { name } => {
            format!("zellij-tools::scratchpad::close::{}", name)
        }
        ScratchpadAction::List { .. } => unreachable!("list is handled separately"),
    };

    send_pipe_message(plugin, &msg, session)
}

fn focus(plugin: &str, target: FocusTarget, session: Option<&str>) -> std::io::Result<()> {
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

    send_pipe_message(plugin, &msg, session)
}

fn subscribe(
    plugin: &str,
    full: bool,
    events: Vec<SubscribeEventKind>,
    pane_ids: Vec<u32>,
    plugin_pane_ids: Vec<u32>,
    tab_ids: Vec<usize>,
    session: Option<&str>,
) -> std::io::Result<()> {
    let pipe_name = format!("zellij-tools-events-{}", uuid::Uuid::new_v4());

    let subscribe_msg = if full {
        "zellij-tools::subscribe::full".to_string()
    } else {
        "zellij-tools::subscribe".to_string()
    };

    // Spawn zellij pipe with subscribe message as positional payload.
    let mut child = zellij_cmd(session)
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
    let session_owned = session.map(String::from);

    ctrlc::set_handler(move || {
        eprintln!("\nUnsubscribing...");
        // Send unsubscribe (best effort, ignore errors)
        let mut cmd = Command::new("zellij");
        if let Some(ref s) = session_owned {
            cmd.env("ZELLIJ_SESSION_NAME", s);
        }
        let _ = cmd
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

    let stdin = Arc::new(Mutex::new(stdin));

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
                let init_json =
                    build_subscribe_init_json(full, &events, &pane_ids, &plugin_pane_ids, &tab_ids);
                if let Ok(mut stdin) = stdin.lock() {
                    stdin.write_all(init_json.as_bytes())?;
                    stdin.write_all(b"\n")?;
                    stdin.flush()?;
                }

                match rx.recv_timeout(CONNECT_TIMEOUT) {
                    Ok(line) if line.contains(r#""InitAck""#) => {}
                    Ok(line) if line.contains(r#""InitError""#) => {
                        eprintln!("error: subscribe init rejected: {}", line);
                        if let Ok(mut child) = child.lock() {
                            let _ = child.kill();
                            let _ = child.wait();
                        }
                        std::process::exit(1);
                    }
                    Ok(line) => {
                        eprintln!("error: unexpected init response from plugin: {}", line);
                        if let Ok(mut child) = child.lock() {
                            let _ = child.kill();
                            let _ = child.wait();
                        }
                        std::process::exit(1);
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        eprintln!(
                            "error: timed out waiting for subscribe init ack ({}s)",
                            CONNECT_TIMEOUT.as_secs()
                        );
                        if let Ok(mut child) = child.lock() {
                            let _ = child.kill();
                            let _ = child.wait();
                        }
                        std::process::exit(1);
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        eprintln!("error: plugin connection closed during subscribe init");
                        if let Ok(mut child) = child.lock() {
                            let _ = child.wait();
                        }
                        std::process::exit(1);
                    }
                }
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

    // Start heartbeat only after init handshake succeeds.
    let heartbeat_running = running.clone();
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

/// Send a one-shot scratchpad list request and read a single JSON response.
fn scratchpad_list(
    plugin: &str,
    names: Vec<String>,
    tab_id: Option<usize>,
    full: bool,
    session: Option<&str>,
) -> std::io::Result<()> {
    let pipe_name = format!("zellij-tools-scratchpad-list-{}", uuid::Uuid::new_v4());

    // Build payload: zellij-tools::scratchpad::list[::full][::tab=<id>][::name1::name2::...]
    let mut parts = vec!["zellij-tools", "scratchpad", "list"];
    let tab_str;
    if full {
        parts.push("full");
    }
    if let Some(id) = tab_id {
        tab_str = format!("tab={}", id);
        parts.push(&tab_str);
    }
    let name_strs: Vec<String> = names;
    for name in &name_strs {
        parts.push(name);
    }
    let msg = parts.join("::");

    let mut child = zellij_cmd(session)
        .args(["pipe", "--name", &pipe_name, "--plugin", plugin, "--", &msg])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(b"\n");
        let _ = stdin.flush();
        drop(stdin);

        if let Some(stdout) = child.stdout.take() {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                match line {
                    Ok(l) if !l.is_empty() => {
                        println!("{}", l);
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

/// Send a one-shot request to the plugin and read a single JSON response.
fn tree(plugin: &str, session: Option<&str>) -> std::io::Result<()> {
    let pipe_name = format!("zellij-tools-tree-{}", uuid::Uuid::new_v4());
    let msg = "zellij-tools::tree".to_string();

    let mut child = zellij_cmd(session)
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
    let session = cli.session.as_deref();

    let result = match cli.command {
        Commands::Focus { target } => focus(&plugin, target, session),
        Commands::Scratchpad {
            action:
                ScratchpadAction::List {
                    names,
                    tab_id,
                    full,
                },
        } => scratchpad_list(&plugin, names, tab_id, full, session),
        Commands::Scratchpad { action } => scratchpad(&plugin, action, session),
        Commands::Subscribe {
            full,
            events,
            pane_ids,
            plugin_pane_ids,
            tab_ids,
        } => subscribe(
            &plugin,
            full,
            events,
            pane_ids,
            plugin_pane_ids,
            tab_ids,
            session,
        ),
        Commands::Tree => tree(&plugin, session),
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

    #[test]
    fn parses_subscribe_filter_flags() {
        let cli = Cli::try_parse_from([
            "zellij-tools",
            "subscribe",
            "--full",
            "--event",
            "PaneFocused,TabMoved",
            "--pane-id",
            "2,3",
            "--plugin-pane-id",
            "7,8",
            "--tab-id",
            "101,202",
        ])
        .unwrap();

        match cli.command {
            Commands::Subscribe {
                full,
                events,
                pane_ids,
                plugin_pane_ids,
                tab_ids,
            } => {
                assert!(full);
                assert_eq!(
                    events,
                    vec![
                        SubscribeEventKind::PaneFocused,
                        SubscribeEventKind::TabMoved
                    ]
                );
                assert_eq!(pane_ids, vec![2, 3]);
                assert_eq!(plugin_pane_ids, vec![7, 8]);
                assert_eq!(tab_ids, vec![101, 202]);
            }
            _ => panic!("expected subscribe command"),
        }
    }

    #[test]
    fn subscribe_init_json_contains_requested_filters() {
        let json = build_subscribe_init_json(
            true,
            &[
                SubscribeEventKind::PaneFocused,
                SubscribeEventKind::TabMoved,
            ],
            &[2],
            &[2],
            &[7, 9],
        );

        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["full"], true);
        assert_eq!(
            value["events"],
            serde_json::json!(["PaneFocused", "TabMoved"])
        );
        assert_eq!(
            value["pane_ids"],
            serde_json::json!(["terminal_2", "plugin_2"])
        );
        assert_eq!(value["tab_ids"], serde_json::json!([7, 9]));
    }

    #[test]
    fn subscribe_init_json_omits_empty_filter_fields() {
        let json = build_subscribe_init_json(false, &[], &[], &[], &[]);
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(value["full"], false);
        assert!(value.get("events").is_none());
        assert!(value.get("pane_ids").is_none());
        assert!(value.get("tab_ids").is_none());
    }

    #[test]
    fn parses_scratchpad_toggle_with_name() {
        let cli = Cli::try_parse_from(["zellij-tools", "scratchpad", "toggle", "term"]).unwrap();

        match cli.command {
            Commands::Scratchpad {
                action: ScratchpadAction::Toggle { name },
            } => assert_eq!(name.as_deref(), Some("term")),
            _ => panic!("expected scratchpad toggle command"),
        }
    }

    #[test]
    fn parses_scratchpad_toggle_without_name() {
        let cli = Cli::try_parse_from(["zellij-tools", "scratchpad", "toggle"]).unwrap();

        match cli.command {
            Commands::Scratchpad {
                action: ScratchpadAction::Toggle { name },
            } => assert_eq!(name, None),
            _ => panic!("expected scratchpad toggle command"),
        }
    }

    #[test]
    fn parses_scratchpad_show() {
        let cli = Cli::try_parse_from(["zellij-tools", "scratchpad", "show", "htop"]).unwrap();

        match cli.command {
            Commands::Scratchpad {
                action: ScratchpadAction::Show { name },
            } => assert_eq!(name, "htop"),
            _ => panic!("expected scratchpad show command"),
        }
    }

    #[test]
    fn parses_scratchpad_hide() {
        let cli = Cli::try_parse_from(["zellij-tools", "scratchpad", "hide", "htop"]).unwrap();

        match cli.command {
            Commands::Scratchpad {
                action: ScratchpadAction::Hide { name },
            } => assert_eq!(name, "htop"),
            _ => panic!("expected scratchpad hide command"),
        }
    }

    #[test]
    fn parses_scratchpad_close() {
        let cli = Cli::try_parse_from(["zellij-tools", "scratchpad", "close", "term"]).unwrap();

        match cli.command {
            Commands::Scratchpad {
                action: ScratchpadAction::Close { name },
            } => assert_eq!(name, "term"),
            _ => panic!("expected scratchpad close command"),
        }
    }

    #[test]
    fn parses_scratchpad_list_no_args() {
        let cli = Cli::try_parse_from(["zellij-tools", "scratchpad", "list"]).unwrap();

        match cli.command {
            Commands::Scratchpad {
                action:
                    ScratchpadAction::List {
                        names,
                        tab_id,
                        full,
                    },
            } => {
                assert!(names.is_empty());
                assert_eq!(tab_id, None);
                assert!(!full);
            }
            _ => panic!("expected scratchpad list command"),
        }
    }

    #[test]
    fn parses_scratchpad_list_with_names() {
        let cli =
            Cli::try_parse_from(["zellij-tools", "scratchpad", "list", "term", "htop"]).unwrap();

        match cli.command {
            Commands::Scratchpad {
                action: ScratchpadAction::List { names, .. },
            } => {
                assert_eq!(names, vec!["term", "htop"]);
            }
            _ => panic!("expected scratchpad list command"),
        }
    }

    #[test]
    fn parses_scratchpad_list_with_tab_filter() {
        let cli =
            Cli::try_parse_from(["zellij-tools", "scratchpad", "list", "--tab", "42"]).unwrap();

        match cli.command {
            Commands::Scratchpad {
                action: ScratchpadAction::List { tab_id, .. },
            } => {
                assert_eq!(tab_id, Some(42));
            }
            _ => panic!("expected scratchpad list command"),
        }
    }

    #[test]
    fn parses_scratchpad_list_with_full_flag() {
        let cli = Cli::try_parse_from(["zellij-tools", "scratchpad", "list", "--full"]).unwrap();

        match cli.command {
            Commands::Scratchpad {
                action: ScratchpadAction::List { full, .. },
            } => {
                assert!(full);
            }
            _ => panic!("expected scratchpad list command"),
        }
    }

    #[test]
    fn parses_scratchpad_list_all_options() {
        let cli = Cli::try_parse_from([
            "zellij-tools",
            "scratchpad",
            "list",
            "--full",
            "--tab",
            "7",
            "term",
            "htop",
        ])
        .unwrap();

        match cli.command {
            Commands::Scratchpad {
                action:
                    ScratchpadAction::List {
                        names,
                        tab_id,
                        full,
                    },
            } => {
                assert!(full);
                assert_eq!(tab_id, Some(7));
                assert_eq!(names, vec!["term", "htop"]);
            }
            _ => panic!("expected scratchpad list command"),
        }
    }

    #[test]
    fn parses_session_flag_before_subcommand() {
        let cli = Cli::try_parse_from(["zellij-tools", "-s", "my-session", "tree"]).unwrap();
        assert_eq!(cli.session.as_deref(), Some("my-session"));
    }

    #[test]
    fn parses_session_long_flag() {
        let cli = Cli::try_parse_from(["zellij-tools", "--session", "other", "focus", "pane", "1"])
            .unwrap();
        assert_eq!(cli.session.as_deref(), Some("other"));
    }

    #[test]
    fn session_flag_is_optional() {
        let cli = Cli::try_parse_from(["zellij-tools", "tree"]).unwrap();
        assert!(cli.session.is_none());
    }
}
