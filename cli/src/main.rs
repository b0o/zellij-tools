use clap::{Parser, Subcommand};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const PLUGIN_PATH: &str = "file:~/.config/zellij/plugins/zellij-tools.wasm";

#[derive(Parser)]
#[command(name = "zellij-tools")]
#[command(about = "CLI utilities for zellij-tools plugin")]
struct Cli {
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

fn subscribe_pane_focus(pane_filter: Option<u32>) -> std::io::Result<()> {
    let pipe_name = format!("zellij-tools-focus-{}", uuid::Uuid::new_v4());

    // Build subscribe message
    let subscribe_msg = match pane_filter {
        Some(pane_id) => format!("zellij-tools::subscribe-focus::{}", pane_id),
        None => "zellij-tools::subscribe-focus".to_string(),
    };

    // Spawn zellij pipe
    let mut child = Command::new("zellij")
        .args(["pipe", "--name", &pipe_name, "--plugin", PLUGIN_PATH])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;

    let mut stdin = child.stdin.take().expect("Failed to open stdin");
    let stdout = child.stdout.take().expect("Failed to open stdout");

    // Set up Ctrl+C handler
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    let pipe_name_clone = pipe_name.clone();

    ctrlc::set_handler(move || {
        eprintln!("\nUnsubscribing...");
        // Send unsubscribe (best effort, ignore errors)
        let _ = Command::new("zellij")
            .args([
                "pipe",
                "--plugin",
                PLUGIN_PATH,
                "--",
                &format!("zellij-tools::unsubscribe-focus::{}", pipe_name_clone),
            ])
            .status();
        r.store(false, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl-C handler");

    // Send subscribe message
    writeln!(stdin, "{}", subscribe_msg)?;
    stdin.flush()?;
    // Keep stdin open by not dropping it

    // Read and forward stdout
    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        if !running.load(Ordering::SeqCst) {
            break;
        }
        match line {
            Ok(l) => println!("{}", l),
            Err(e) => {
                eprintln!("Error reading: {}", e);
                break;
            }
        }
    }

    // Cleanup
    let _ = child.wait();
    Ok(())
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Subscribe { event } => match event {
            SubscribeEvent::PaneFocus { pane } => {
                if let Err(e) = subscribe_pane_focus(pane) {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
        },
    }
}
