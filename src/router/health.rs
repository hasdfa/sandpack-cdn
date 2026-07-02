use warp::{hyper::StatusCode, Filter, Rejection, Reply};

use crate::npm_replicator::registry::NpmRocksDB;

use super::routes::with_data;

pub async fn health_route_handler(npm_db: NpmRocksDB) -> Result<impl Reply, Rejection> {
    // A real point-get against RocksDB; an empty database returns Ok(0) so
    // fresh deploys stay healthy, but a broken/unreadable one reports 500.
    let db_check = tokio::task::spawn_blocking(move || npm_db.get_last_seq()).await;
    let reply = match db_check {
        Ok(Ok(_last_seq)) => warp::reply::with_status("ok", StatusCode::OK),
        _ => warp::reply::with_status("db unavailable", StatusCode::INTERNAL_SERVER_ERROR),
    };
    Ok(reply)
}

pub fn health_route(
    npm_db: NpmRocksDB,
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path!("health")
        .and(warp::get())
        .and(with_data(npm_db))
        .and_then(health_route_handler)
}
