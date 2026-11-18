#![forbid(unsafe_code)]

use std::{
    env,
    path::PathBuf,
    process::{Command, ExitCode},
};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("provjera") => match run_checks() {
            Ok(()) => {
                println!("Sve provjere kvalitete za List2Cart su uspjesno prosle!");
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("Provjera nije prosla: {err}");
                ExitCode::FAILURE
            }
        },
        _ => {
            eprintln!("Uporaba: cargo xtask provjera");
            ExitCode::FAILURE
        }
    }
}

fn workspace_root() -> Result<PathBuf, String> {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(PathBuf::from)
        .ok_or_else(|| "Nije moguce pronaci korijen radnog prostora".to_owned())
}

fn run_step(root: &PathBuf, name: &str, args: &[&str]) -> Result<(), String> {
    println!("==> Pokrecem: {name} (cargo {})", args.join(" "));
    let status = Command::new("cargo")
        .current_dir(root)
        .args(args)
        .status()
        .map_err(|e| format!("Greska pri pokretanju `{name}`: {e}"))?;
    if !status.success() {
        return Err(format!("Korak `{name}` je pao s kodom {:?}", status.code()));
    }
    Ok(())
}

fn has_cargo_subcommand(subcmd: &str) -> bool {
    Command::new("cargo")
        .args([subcmd, "--version"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn run_checks() -> Result<(), String> {
    let root = workspace_root()?;
    run_step(&root, "rustfmt", &["fmt", "--all", "--", "--check"])?;
    run_step(
        &root,
        "clippy",
        &[
            "clippy",
            "-p",
            "list2cart-core",
            "-p",
            "list2cart-sync",
            "-p",
            "xtask",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    run_step(
        &root,
        "testovi",
        &[
            "test",
            "-p",
            "list2cart-core",
            "-p",
            "list2cart-sync",
            "-p",
            "xtask",
        ],
    )?;

    if has_cargo_subcommand("deny") {
        run_step(
            &root,
            "cargo-deny",
            &["deny", "--config", "quality/deny.toml", "check"],
        )?;
    } else {
        println!("(Napomena: cargo-deny nije instaliran lokalno, preskacem provjeru licenci)");
    }

    Ok(())
}
