use clap::{Parser, Subcommand};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const DEFAULT_PLUGIN: &str = "zellij-tools";
const HEARTBEAT_INTERVAL: Duration = Duration::from_millis(100);

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
    /// Subscribe to the event stream from the zellij-tools plugin
    Subscribe,
}

fn subscribe(plugin: &str) -> std::io::Result<()> {
    let pipe_name = format!("zellij-tools-events-{}", uuid::Uuid::new_v4());

    let subscribe_msg = "zellij-tools::subscribe".to_string();

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

    // Read and forward stdout
    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        if !running.load(Ordering::SeqCst) {
            break;
        }
        match line {
            Ok(l) => {
                if !l.is_empty() {
                    println!("{}", l);
                }
            }
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
        Commands::Subscribe => {
            if let Err(e) = subscribe(&plugin) {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
    }
}
