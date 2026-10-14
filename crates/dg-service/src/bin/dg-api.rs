use dg_service::{
    api::{router, ApiConfig, ApiState},
    Result, ServiceError,
};
use std::path::Path;

async fn run() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 1 {
        return Err(ServiceError::new(
            "INVALID_ARGUMENTS",
            "Usage: dg-api /absolute/api.json",
        ));
    }
    let config = ApiConfig::load(Path::new(&args[0]))?;
    let bind = config.bind;
    let state = ApiState::open(config).await?;
    let listener = tokio::net::TcpListener::bind(bind).await?;
    eprintln!(
        "Prepared-project API listening on {}",
        listener.local_addr()?
    );
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    axum::serve(listener, router(state))
        .with_graceful_shutdown(async move {
            tokio::select! {_=tokio::signal::ctrl_c()=>{},_=terminate.recv()=>{}}
        })
        .await?;
    Ok(())
}
#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
