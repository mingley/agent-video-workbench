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
    #[arg(long, default_value = "ffmpeg", global = true)]
    ffmpeg: PathBuf,
    #[arg(long, default_value = "ffprobe", global = true)]
    ffprobe: PathBuf,
    #[command(subcommand)]
    command: Action,
}

#[derive(Subcommand)]
enum Action {
    Doctor,
    Worker {
        project: PathBuf,
        #[arg(long, default_value_t = 0)]
        idle_seconds: u64,
    },
    RenderStart {
        project: PathBuf,
        #[arg(long, default_value = "seq_main")]
        sequence: String,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        key: String,
        #[arg(long)]
        no_launch: bool,
    },
    JobStatus {
        project: PathBuf,
        id: String,
    },
    JobCancel {
        project: PathBuf,
        id: String,
    },
    JobRetry {
        project: PathBuf,
        id: String,
    },
    Jobs {
        project: PathBuf,
    },
    Backup {
        project: PathBuf,
        destination: PathBuf,
    },
    Protect {
        project: PathBuf,
        #[arg(long)]
        request: PathBuf,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        key: String,
    },
    Capabilities,
    Describe {
        capability: String,
    },
    Import {
        project: PathBuf,
        source: PathBuf,
        #[arg(long)]
        id: String,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        key: String,
    },
    Render {
        project: PathBuf,
        #[arg(long, default_value = "seq_main")]
        sequence: String,
    },
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
    let backend = agent_video_workbench::media::backend(cli.ffmpeg, cli.ffprobe);
    match cli.command {
        Action::Worker {
            project,
            idle_seconds,
        } => {
            let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let signal = stop.clone();
            ctrlc::set_handler(move || signal.store(true, std::sync::atomic::Ordering::Relaxed))
                .map_err(|e| agent_video_workbench::Error::Invalid(e.to_string()))?;
            agent_video_workbench::jobs::worker(&project, stop, idle_seconds)
        }
        Action::RenderStart {
            project,
            sequence,
            expected_revision,
            key,
            no_launch,
        } => {
            let job = Store::open(&project)?.enqueue(
                &key,
                &agent_video_workbench::jobs::RenderInput {
                    sequence,
                    expected_revision,
                    ffmpeg: backend.ffmpeg_path().into(),
                    ffprobe: backend.ffprobe_path().into(),
                },
            )?;
            if !no_launch && job.state == "queued" {
                agent_video_workbench::jobs::launch(&project, &std::env::current_exe()?)?;
            }
            Ok(serde_json::to_value(job)?)
        }
        Action::JobStatus { project, id } => {
            Ok(serde_json::to_value(Store::open(&project)?.job(&id)?)?)
        }
        Action::JobCancel { project, id } => Ok(serde_json::to_value(
            Store::open(&project)?.cancel_job(&id)?,
        )?),
        Action::JobRetry { project, id } => Ok(serde_json::to_value(
            Store::open(&project)?.retry_job(&id)?,
        )?),
        Action::Jobs { project } => Ok(json!(Store::open(&project)?.jobs()?)),
        Action::Backup {
            project,
            destination,
        } => agent_video_workbench::media::backup(&project, &destination),
        Action::Protect {
            project,
            request,
            expected_revision,
            key,
        } => {
            let range = agent_video_workbench::json::read(&request)?;
            Ok(serde_json::to_value(Store::open(&project)?.protect(
                range,
                expected_revision,
                &key,
            )?)?)
        }
        Action::Capabilities => Ok(
            json!({"apiVersion":"1","operations":agentcut_core::capabilities::registry(),"limitations":["SDR only","synchronous render; crashed running jobs require manual reconciliation","no ASR or hosted-bot validation"]}),
        ),
        Action::Describe { capability } => Ok(agentcut_core::capabilities::describe(&capability)?),
        Action::Doctor => Ok(serde_json::to_value(agentcut_render::doctor(&backend))?),
        Action::Import {
            project,
            source,
            id,
            expected_revision,
            key,
        } => Ok(serde_json::to_value(agent_video_workbench::media::import(
            &project,
            &source,
            &id,
            expected_revision,
            &key,
            &backend,
        )?)?),
        Action::Render { project, sequence } => {
            agent_video_workbench::media::render(&project, &sequence, &backend)
        }
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
            let batch: OperationBatch = agent_video_workbench::json::read(&request)?;
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
