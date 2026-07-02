use crate::npm_replicator::registry::NpmRocksDB;
use dotenv::dotenv;
use std::env;
use std::net::SocketAddr;
use warp::http::header::{HeaderMap, HeaderValue};
use warp::Filter;

mod app_error;
mod cached;
mod npm;
mod npm_replicator;
mod package;
mod router;
mod setup_tracing;
mod utils;

#[tokio::main]
async fn main() -> Result<(), std::io::Error> {
    dotenv().ok();
    setup_tracing::setup_tracing();

    let port = match env::var("PORT") {
        Ok(var) => var,
        Err(_) => String::from("8080"),
    }
    .parse::<u16>()
    .unwrap();

    // Setup npm db
    let npm_registry_path =
        env::var("NPM_ROCKS_DB").expect("NPM_ROCKS_DB env variable should be set");
    println!("Creating npm rocks db at {}", npm_registry_path);
    let npm_fs_db = NpmRocksDB::new(&npm_registry_path);

    // npm_replicator::replication_task::spawn_sync_thread(npm_fs_db.clone());

    // cors headers
    let mut headers = HeaderMap::new();
    headers.insert("Access-Control-Allow-Origin", HeaderValue::from_static("*"));
    headers.insert(
        "Access-Control-Allow-Headers",
        HeaderValue::from_static("*"),
    );
    headers.insert(
        "Access-Control-Allow-Methods",
        HeaderValue::from_static("GET, POST, OPTIONS"),
    );
    let cors_headers_filter = warp::reply::with::headers(headers);

    let filter = router::routes::routes(npm_fs_db)
        .with(warp::trace::request())
        .with(cors_headers_filter)
        .with(warp::compression::gzip());

    let addr: SocketAddr = ([0, 0, 0, 0], port).into();
    let (bound_addr, server) =
        warp::serve(filter).bind_with_graceful_shutdown(addr, shutdown_signal());
    println!("Server running on {}", bound_addr);
    server.await;

    Ok(())
}

// Resolves on SIGINT (what fly.toml sends) or SIGTERM; in-flight requests
// drain before the process exits, fly force-kills stragglers after 5s.
async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut sigterm = signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {},
        _ = sigterm.recv() => {},
    }
    println!("Shutdown signal received, draining connections...");
}
