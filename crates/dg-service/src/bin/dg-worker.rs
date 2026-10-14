//! Local operator interface until the authenticated HTTP adapter is implemented.
use dg_service::{
    intake::{receive, Limits},
    model::PreparedUpload,
    runner::{now, Worker, WorkerConfig},
    store::Store,
    Result, ServiceError,
};
use std::{
    fs::{self, File},
    os::unix::fs::PermissionsExt,
    path::Path,
    sync::Arc,
};

async fn run() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let bad = || {
        ServiceError::new("INVALID_ARGUMENTS", "Usage: dg-worker CONFIG check|once|run|submit OWNER METADATA ZIP|status OWNER JOB|cancel OWNER JOB")
    };
    if args.len() < 2 {
        return Err(bad());
    }
    let cfg = WorkerConfig::load(Path::new(&args[0]))?;
    match (args[1].as_str(), args.len()) {
        ("check" | "once" | "run", 2) => {
            let worker = Arc::new(Worker::open(cfg).await?);
            let shutdown_worker = Arc::downgrade(&worker);
            let mut terminate =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
            tokio::spawn(async move {
                tokio::select! {_=tokio::signal::ctrl_c()=>{},_=terminate.recv()=>{}}
                if let Some(worker) = shutdown_worker.upgrade() {
                    worker.request_shutdown();
                }
            });
            worker.check().await?;
            if args[1] == "check" {
                println!("{{\"schema_version\":1,\"ready\":true}}");
                return Ok(());
            }
            worker.recover().await?;
            if args[1] == "once" {
                println!("{{\"processed\":{}}}", worker.run_once().await?);
                return Ok(());
            }
            loop {
                if worker.shutting_down() {
                    return Ok(());
                }
                worker.recover().await?;
                if !worker.run_once().await? {
                    tokio::select! {
                        _ = tokio::signal::ctrl_c() => return Ok(()),
                        _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {}
                    }
                }
            }
        }
        ("submit", 5) | ("status" | "cancel", 4) => {
            // These local commands are available to the trusted storage account only.
            let info = fs::symlink_metadata(&cfg.storage_root)?;
            if !info.is_dir() || info.permissions().mode() & 0o077 != 0 {
                return Err(ServiceError::new(
                    "INVALID_STORAGE",
                    "Initialize a private worker root first",
                ));
            }
            let store = Store::open(&cfg.storage_root.join("jobs.sqlite")).await?;
            let owner = &args[2];
            if owner.is_empty() || owner.len() > 128 || owner.chars().any(char::is_control) {
                return Err(bad());
            }
            let job = match args[1].as_str() {
                "submit" => {
                    let bytes = dg_analysis::read_bounded(Path::new(&args[3]), 32 * 1024)?;
                    let metadata: PreparedUpload =
                        serde_json::from_slice(&bytes).map_err(|_| bad())?;
                    metadata.validate()?;
                    let archive = receive(
                        &cfg.storage_root.join("submissions"),
                        File::open(&args[4])?,
                        Limits::default(),
                    )?;
                    let submission = store
                        .add_submission(owner, &metadata, &archive, now())
                        .await?;
                    store.enqueue(owner, &submission, None, now()).await?
                }
                "status" => store
                    .job(owner, &args[3])
                    .await?
                    .ok_or_else(|| ServiceError::new("NOT_FOUND", "Job not found"))?,
                _ => store.cancel(owner, &args[3], now()).await?,
            };
            println!("{}", serde_json::to_string_pretty(&job).map_err(|_| bad())?);
            Ok(())
        }
        _ => Err(bad()),
    }
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
