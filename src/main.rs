use sigmaos_arch_update::PacmanUpdateManager;
use std::io::{self, Write};

fn main() {
    if let Err(error) = run() {
        eprintln!("arch-update: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut apply = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--check" => {}
            "--apply" => apply = true,
            "--help" | "-h" => {
                println!("Usage: arch-update [--check | --apply]\n\n--check  List upgrades available from the local pacman sync databases (default).\n--apply  Ask for confirmation, then run pacman -Syu interactively.");
                return Ok(());
            }
            _ => return Err(format!("unknown argument: {arg}").into()),
        }
    }

    let manager = PacmanUpdateManager::new("pacman");
    let updates = manager.check_updates()?;
    if updates.is_empty() {
        println!("No upgrades are available in the local package databases.");
    } else {
        println!("{} upgrade(s) available:", updates.len());
        for update in &updates {
            println!(
                "{}  {} -> {}",
                update.name, update.current_version, update.new_version
            );
        }
    }

    if apply {
        print!("Run pacman -Syu now? Type 'yes' to continue: ");
        io::stdout().flush()?;
        let mut answer = String::new();
        io::stdin().read_line(&mut answer)?;
        if answer.trim() != "yes" {
            println!("Upgrade cancelled.");
            return Ok(());
        }
        manager.apply_upgrade()?;
    }
    Ok(())
}
