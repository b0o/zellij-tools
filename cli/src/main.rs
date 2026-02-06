use clap::{Parser, Subcommand};
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const DEFAULT_PLUGIN: &str = "zellij-tools";

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
    #[arg(long, global = true)]
    plugin: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Subscribe to events from the zellij-tools plugin
    Subscribe {
        #[command(subcommand)]
        event: SubscribeEvent,
    },
}

#[derive(Subcommand)]
enum SubscribeEvent {
    /// Subscribe to pane focus change events
    PaneFocus {
        /// Only receive events for this specific pane ID
        #[arg(long)]
        pane: Option<u32>,
    },
}

fn subscribe_pane_focus(pane_filter: Option<u32>, plugin: &str) -> std::io::Result<()> {
    let pipe_name = format!("zellij-tools-focus-{}", uuid::Uuid::new_v4());

    // Build subscribe message
    let subscribe_msg = match pane_filter {
        Some(pane_id) => format!("zellij-tools::subscribe-focus::{}", pane_id),
        None => "zellij-tools::subscribe-focus".to_string(),
    };

    // Spawn zellij pipe with subscribe message as positional payload.
    // The payload is sent immediately; stdin is kept open to hold the pipe alive.
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

    // Hold stdin open to keep the pipe alive (dropping it would close the pipe)
    let _stdin = child.stdin.take().expect("Failed to open stdin");
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
                &format!("zellij-tools::unsubscribe-focus::{}", pipe_name_clone),
            ])
            .status();
        r.store(false, Ordering::SeqCst);
        // Kill child process to unblock reader.lines()
        if let Ok(mut child) = child_clone.lock() {
            let _ = child.kill();
        }
    })
    .expect("Error setting Ctrl-C handler");

    // Read and forward stdout
    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        if !running.load(Ordering::SeqCst) {
            break;
        }
        match line {
            Ok(l) => println!("{}", l),
            Err(e) => {
                if running.load(Ordering::SeqCst) {
                    eprintln!("Error reading: {}", e);
                }
                break;
            }
        }
    }

    // Cleanup
    if let Ok(mut child) = child.lock() {
        let _ = child.wait();
    }
    Ok(())
}

fn main() {
    let cli = Cli::parse();

    let plugin = resolve_plugin(cli.plugin.as_deref());

    match cli.command {
        Commands::Subscribe { event } => match event {
            SubscribeEvent::PaneFocus { pane } => {
                if let Err(e) = subscribe_pane_focus(pane, &plugin) {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
        },
    }
}
