#![deny(unsafe_code)]
#![warn(missing_docs)]
#![deny(clippy::all)]
#![warn(unreachable_pub)]
//! Astrid CLI Mockup - UI/UX prototype for the interactive experience.
//!
//! This is a self-contained mockup for designing and testing the CLI experience.
//! No actual LLM calls or tool execution - everything is simulated.
//!
//! Run with: `cargo run -p astrid-cli-mockup`
//!
//! Or run a specific demo scenario:
//! `cargo run -p astrid-cli-mockup -- --demo simple-qa`
//!
//! Or capture snapshots without interactive mode:
//! `cargo run -p astrid-cli-mockup -- --snapshot showcase --steps 5`

use std::io;

mod demo;
mod mock;
mod ui;

use ui::App;

fn main() -> io::Result<()> {
    // Parse args for demo mode
    let args: Vec<String> = std::env::args().collect();
    let demo_scenario = args
        .iter()
        .position(|a| a == "--demo")
        .and_then(|i| args.get(i.saturating_add(1)))
        .map(String::as_str);

    // Check for snapshot mode
    let snapshot_scenario = args
        .iter()
        .position(|a| a == "--snapshot")
        .and_then(|i| args.get(i.saturating_add(1)))
        .map(String::as_str);

    let snapshot_count: usize = args
        .iter()
        .position(|a| a == "--steps")
        .and_then(|i| args.get(i.saturating_add(1)))
        .and_then(|s| s.parse().ok())
        .unwrap_or(5);

    // Snapshot mode - non-interactive, outputs frames to stdout
    if let Some(scenario) = snapshot_scenario {
        run_snapshot_mode(scenario, snapshot_count);
        return Ok(());
    }

    // Initialize terminal
    let mut terminal = ui::init_terminal()?;

    // Create app
    let mut app = App::new();

    // If demo mode, load the scenario
    if let Some(scenario) = demo_scenario {
        app.load_demo(scenario);
    }

    // Run the main loop
    let result = app.run(&mut terminal);

    // Restore terminal
    ui::restore_terminal(&mut terminal)?;

    result
}

/// Run in snapshot mode - advances demo and prints frames
fn run_snapshot_mode(scenario: &str, snapshots: usize) {
    use demo::DemoPlayer;
    use demo::DemoScenario;

    let mut app = App::new();

    // Load scenario in fast-forward mode
    if let Some(scenario_data) = DemoScenario::load(scenario) {
        app.demo_player = Some(DemoPlayer::new_fast_forward(scenario_data));
    } else {
        println!("Unknown scenario: {scenario}");
        return;
    }

    let width = 100;
    let height = 30;

    println!("=== Snapshot Mode: {scenario} ({snapshots} snapshots) ===\n");

    let mut snapshot_count: usize = 0;
    let mut total_advances: usize = 0;
    let max_advances: usize = 5000; // Safety limit

    // Take snapshots at meaningful intervals
    while snapshot_count < snapshots && total_advances < max_advances {
        // Advance the demo player multiple times to get meaningful state changes
        let mut advanced_this_round: usize = 0;
        let advances_per_snapshot: usize = 20; // Advance 20 times between snapshots

        while advanced_this_round < advances_per_snapshot && total_advances < max_advances {
            if app.demo_player.is_some() {
                let mut player = app.demo_player.take().unwrap();
                let complete = player.advance(&mut app);
                if complete {
                    println!("\n=== Demo Complete (total advances: {total_advances}) ===\n");
                    let snapshot = ui::render_snapshot(&app, width, height);
                    println!("{snapshot}");
                    return;
                }
                app.demo_player = Some(player);
            }
            advanced_this_round = advanced_this_round.saturating_add(1);
            total_advances = total_advances.saturating_add(1);
        }

        // Print snapshot
        snapshot_count = snapshot_count.saturating_add(1);
        println!(
            "\n┌─── Snapshot {snapshot_count} (after {total_advances} advances) ──────────────────────────────┐"
        );
        let snapshot = ui::render_snapshot(&app, width, height);
        println!("{snapshot}");
        println!("└───────────────────────────────────────────────────────────────────┘\n");
    }
}
