use agent_video_workbench::{
    Result,
    service::{Request, Service},
};
use clap::{Parser, Subcommand};
use serde::Serialize;
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
    #[arg(long, global = true)]
    workspace: Option<PathBuf>,
    #[arg(long, global=true, requires_all=["whisper_model","whisper_model_sha256"])]
    whisper: Option<PathBuf>,
    #[arg(long, global = true, requires = "whisper")]
    whisper_model: Option<PathBuf>,
    #[arg(long, global = true, requires = "whisper")]
    whisper_model_sha256: Option<String>,
    #[arg(long, global = true, default_value = "en")]
    whisper_language: String,
    #[arg(long, global = true)]
    download_host: Vec<String>,
    #[arg(long, global = true)]
    download_loopback_http: bool,
    #[arg(long, global = true)]
    analysis_provider: Option<PathBuf>,
    #[command(subcommand)]
    command: Action,
}

#[derive(Subcommand, Serialize)]
#[serde(
    tag = "command",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
enum Action {
    Doctor,
    Catalog {
        workspace: PathBuf,
        #[arg(long, default_value = "")]
        query: String,
        #[arg(long, default_value_t = 0)]
        offset: u32,
        #[arg(long, default_value_t = 50)]
        limit: u32,
    },
    Maintain {
        workspace: PathBuf,
        #[arg(long, default_value_t = 86400)]
        grace_seconds: u64,
        #[arg(long,default_value_t=true,action=clap::ArgAction::Set)]
        dry_run: bool,
    },
    VerifyProject {
        project: PathBuf,
    },
    Relink {
        project: PathBuf,
        source: PathBuf,
        #[arg(long)]
        sha256: String,
    },
    CacheGc {
        project: PathBuf,
        #[arg(long, default_value_t = 86400)]
        grace_seconds: u64,
        #[arg(long,default_value_t=true,action=clap::ArgAction::Set)]
        dry_run: bool,
    },
    BackupRestore {
        source: PathBuf,
        destination: PathBuf,
    },
    BatchStart {
        project: PathBuf,
        #[arg(long, value_delimiter = ',')]
        sequences: Vec<String>,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        key: String,
        #[arg(long)]
        no_launch: bool,
        #[arg(long, default_value_t = 0)]
        priority: i32,
    },
    BatchStatus {
        project: PathBuf,
        id: String,
    },
    Delivery {
        project: PathBuf,
        id: String,
        destination: PathBuf,
    },
    Studio {
        project: PathBuf,
        #[arg(long)]
        request: PathBuf,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        key: String,
        #[arg(long)]
        dry_run: bool,
    },
    StudioState {
        project: PathBuf,
        #[arg(long, default_value = "avw.")]
        prefix: String,
        #[arg(long, default_value_t = 0)]
        offset: u32,
        #[arg(long, default_value_t = 50)]
        limit: u32,
    },
    Reviews {
        project: PathBuf,
        #[arg(long, default_value = "seq_main")]
        sequence: String,
    },
    Mcp {
        #[arg(long)]
        root: PathBuf,
    },
    Request {
        file: PathBuf,
    },
    Schema,
    AgentGuide,
    Resume {
        project: PathBuf,
    },
    RequestOutcome {
        project: PathBuf,
        key: String,
    },
    Assemble {
        project: PathBuf,
        #[arg(long)]
        request: PathBuf,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        key: String,
        #[arg(long)]
        dry_run: bool,
    },
    EditPreflight {
        project: PathBuf,
        #[arg(long, default_value = "seq_main")]
        sequence: String,
    },
    PlanEdit {
        project: PathBuf,
        #[arg(long)]
        request: PathBuf,
        #[arg(long)]
        expected_revision: u64,
    },
    ApplyEditPlan {
        project: PathBuf,
        #[arg(long)]
        request: PathBuf,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        key: String,
        #[arg(long)]
        plan_sha256: String,
        #[arg(long)]
        dry_run: bool,
    },
    Compose {
        project: PathBuf,
        #[arg(long)]
        request: PathBuf,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        key: String,
        #[arg(long)]
        dry_run: bool,
    },
    TranscribeStart {
        project: PathBuf,
        #[arg(long)]
        asset_id: String,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        key: String,
        #[arg(long)]
        no_launch: bool,
    },
    TranscriptImport {
        project: PathBuf,
        #[arg(long)]
        request: PathBuf,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        key: String,
    },
    TranscriptSearch {
        project: PathBuf,
        query: String,
        #[arg(long)]
        asset_id: Option<String>,
        #[arg(long, default_value_t = 0)]
        offset: u32,
        #[arg(long, default_value_t = 50)]
        limit: u32,
    },
    Inspect {
        project: PathBuf,
        #[arg(long)]
        asset_id: String,
        #[arg(long, default_value = "metadata")]
        kind: String,
        #[arg(long, default_value_t = 0)]
        start_ms: i64,
        #[arg(long)]
        end_ms: Option<i64>,
    },
    Artifacts {
        project: PathBuf,
    },
    Artifact {
        project: PathBuf,
        id: String,
        #[arg(long)]
        sheet: bool,
        #[arg(long)]
        preview: bool,
    },
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
        #[arg(long, default_value_t = 0)]
        priority: i32,
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
        #[arg(long)]
        no_launch: bool,
    },
    Imports {
        project: PathBuf,
    },
    Jobs {
        project: PathBuf,
    },
    Backup {
        project: PathBuf,
        destination: PathBuf,
    },
    Unprotect {
        project: PathBuf,
        id: String,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        key: String,
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
    let asr = asr_config(&cli)?;
    let provider = provider_config(&cli)?;
    let service = Service {
        root: cli.workspace,
        backend: agent_video_workbench::media::backend(cli.ffmpeg.clone(), cli.ffprobe.clone()),
        executable: std::env::current_exe()?,
        asr,
        provider,
        downloads: agent_video_workbench::transfer::Policy {
            hosts: cli.download_host.clone(),
            allow_loopback_http: cli.download_loopback_http,
        },
    };
    let request = match cli.command {
        Action::Worker {
            project,
            idle_seconds,
        } => {
            let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let signal = stop.clone();
            ctrlc::set_handler(move || signal.store(true, std::sync::atomic::Ordering::Relaxed))
                .map_err(|e| agent_video_workbench::Error::Invalid(e.to_string()))?;
            return agent_video_workbench::jobs::worker(&project, stop, idle_seconds);
        }
        Action::Request { file } => {
            if file.as_os_str() == "-" {
                use std::io::Read;
                let mut bytes = Vec::new();
                std::io::stdin()
                    .take(8 * 1024 * 1024 + 1)
                    .read_to_end(&mut bytes)?;
                agent_video_workbench::json::parse(&bytes)?
            } else {
                agent_video_workbench::json::read(&file)?
            }
        }
        Action::Studio {
            project,
            request,
            expected_revision,
            key,
            dry_run,
        } => Request::Studio {
            project,
            edit: agent_video_workbench::json::read(&request)?,
            expected_revision,
            key,
            dry_run,
        },
        Action::Apply {
            project,
            request,
            dry_run,
        } => Request::Apply {
            project,
            batch: agent_video_workbench::json::read(&request)?,
            dry_run,
        },
        Action::Protect {
            project,
            request,
            expected_revision,
            key,
        } => Request::Protect {
            project,
            range: agent_video_workbench::json::read(&request)?,
            expected_revision,
            key,
        },
        Action::Assemble {
            project,
            request,
            expected_revision,
            key,
            dry_run,
        } => Request::Assemble {
            project,
            edit: agent_video_workbench::json::read(&request)?,
            expected_revision,
            key,
            dry_run,
        },
        Action::Compose {
            project,
            request,
            expected_revision,
            key,
            dry_run,
        } => Request::Compose {
            project,
            edit: agent_video_workbench::json::read(&request)?,
            expected_revision,
            key,
            dry_run,
        },
        Action::PlanEdit {
            project,
            request,
            expected_revision,
        } => Request::PlanEdit {
            project,
            plan: agent_video_workbench::json::read(&request)?,
            expected_revision,
        },
        Action::ApplyEditPlan {
            project,
            request,
            expected_revision,
            key,
            plan_sha256,
            dry_run,
        } => Request::ApplyEditPlan {
            project,
            plan: agent_video_workbench::json::read(&request)?,
            expected_revision,
            key,
            plan_sha256,
            dry_run,
        },
        Action::TranscriptImport {
            project,
            request,
            expected_revision,
            key,
        } => Request::TranscriptImport {
            project,
            transcript: agent_video_workbench::json::read(&request)?,
            expected_revision,
            key,
        },
        other => serde_json::from_value(serde_json::to_value(other)?)?,
    };
    service.execute(request)
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
    if let Action::Mcp { root } = &cli.command {
        let result = (|| {
            std::fs::create_dir_all(root)?;
            let asr = asr_config(&cli)?;
            let service = Service {
                root: Some(root.canonicalize()?),
                backend: agent_video_workbench::media::backend(
                    cli.ffmpeg.clone(),
                    cli.ffprobe.clone(),
                ),
                executable: std::env::current_exe()?,
                asr,
                provider: provider_config(&cli)?,
                downloads: agent_video_workbench::transfer::Policy {
                    hosts: cli.download_host.clone(),
                    allow_loopback_http: cli.download_loopback_http,
                },
            };
            agent_video_workbench::mcp::serve(
                &service,
                &mut std::io::stdin().lock(),
                &mut std::io::stdout().lock(),
            )
        })();
        if let Err(error) = result {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    let result = agent_video_workbench::service::envelope(execute(cli));
    println!("{result}");
    if result["ok"] == false {
        std::process::exit(1);
    }
}

fn asr_config(cli: &Cli) -> Result<Option<agent_video_workbench::asr::Config>> {
    let Some(program) = &cli.whisper else {
        return Ok(None);
    };
    let model = cli
        .whisper_model
        .as_ref()
        .ok_or_else(|| agent_video_workbench::Error::Invalid("ASR model path missing".into()))?;
    let hash = cli
        .whisper_model_sha256
        .clone()
        .ok_or_else(|| agent_video_workbench::Error::Invalid("ASR model hash missing".into()))?;
    Ok(Some(agent_video_workbench::asr::Config {
        program: program.canonicalize()?,
        program_sha256: agent_video_workbench::media::hash_file(program)?,
        model: model.canonicalize()?,
        model_sha256: hash,
        language: cli.whisper_language.clone(),
    }))
}

fn provider_config(cli: &Cli) -> Result<Option<agent_video_workbench::analysis::Provider>> {
    cli.analysis_provider
        .as_ref()
        .map(|p| {
            Ok(agent_video_workbench::analysis::Provider {
                program: p.canonicalize()?,
                sha256: agent_video_workbench::media::hash_file(p)?,
            })
        })
        .transpose()
}
