use agent_video_workbench::{Result, store::Store};
use agentcut_core::OperationBatch;
use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "avw",
    version,
    about = "Persistent agent-operated video workbench"
)]
struct Cli {
    #[command(subcommand)]
    command: Action,
}

#[derive(Subcommand)]
enum Action {
    Create {
        project: PathBuf,
        #[arg(long)]
        name: String,
    },
    Status {
        project: PathBuf,
    },
    History {
        project: PathBuf,
    },
    Apply {
        project: PathBuf,
        #[arg(long)]
        request: PathBuf,
        #[arg(long)]
        dry_run: bool,
    },
    Restore {
        project: PathBuf,
        #[arg(long)]
        revision: u64,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        key: String,
    },
    Diff {
        project: PathBuf,
        #[arg(long)]
        from: u64,
        #[arg(long)]
        to: u64,
    },
}

fn execute(cli: Cli) -> Result<Value> {
    match cli.command {
        Action::Create { project, name } => Ok(serde_json::to_value(
            Store::create(&project, &name)?.project()?,
        )?),
        Action::Status { project } => Ok(serde_json::to_value(Store::open(&project)?.project()?)?),
        Action::History { project } => Ok(json!(Store::open(&project)?.history()?)),
        Action::Apply {
            project,
            request,
            dry_run,
        } => {
            let batch: OperationBatch = serde_json::from_slice(&std::fs::read(request)?)?;
            Ok(serde_json::to_value(
                Store::open(&project)?.apply(&batch, dry_run)?,
            )?)
        }
        Action::Restore {
            project,
            revision,
            expected_revision,
            key,
        } => Ok(serde_json::to_value(Store::open(&project)?.restore(
            revision,
            expected_revision,
            &key,
        )?)?),
        Action::Diff { project, from, to } => {
            let store = Store::open(&project)?;
            Ok(json!({"from":store.revision(from)?,"to":store.revision(to)?}))
        }
    }
}

fn main() {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) if e.use_stderr() => {
            println!(
                "{}",
                json!({"ok":false,"apiVersion":"1","error":{"code":"E_ARGUMENT","message":e.to_string()}})
            );
            std::process::exit(2);
        }
        Err(e) => {
            print!("{e}");
            return;
        }
    };
    match execute(cli) {
        Ok(result) => println!("{}", json!({"ok":true,"apiVersion":"1","result":result})),
        Err(e) => {
            println!(
                "{}",
                json!({"ok":false,"apiVersion":"1","error":{"code":e.code(),"message":e.to_string()}})
            );
            std::process::exit(1);
        }
    }
}
