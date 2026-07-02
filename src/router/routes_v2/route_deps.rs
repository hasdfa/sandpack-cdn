use std::collections::HashSet;

use warp::{Filter, Rejection, Reply};

use crate::app_error::{AppResult, ServerError};
use crate::npm::dep_tree_builder::{DepRequest, DepTreeBuilder, ResolutionsMap};
use crate::npm_replicator::registry::NpmRocksDB;
use crate::package::process::parse_package_specifier_no_validation;
use crate::router::utils::decode_base64;

use super::super::custom_reply::CustomReply;
use super::super::error_reply::ErrorReply;
use super::super::routes::with_data;

fn parse_query(query: String) -> Result<HashSet<DepRequest>, ServerError> {
    let parts = query.split(';');
    let mut dep_requests: HashSet<DepRequest> = HashSet::new();
    for part in parts {
        let (name, version) = parse_package_specifier_no_validation(part)?;
        let versions = version.split(',');
        for version in versions {
            dep_requests.insert(DepRequest::from_name_version(
                name.clone(),
                version.to_string(),
            )?);
        }
    }
    Ok(dep_requests)
}

/// A resolution failure is retried by refreshing the offending package from
/// npm, but each package is only fetched once per request. Returns the package
/// to fetch, or None when the error should be surfaced to the client as-is
/// (e.g. a version that was never published stays PackageVersionNotFound).
fn pkg_to_fetch(err: &ServerError, already_fetched: &HashSet<String>) -> Option<String> {
    let pkg_name = match err {
        ServerError::PackageVersionNotFound(pkg_name, _) => pkg_name,
        ServerError::PackageNotFound(pkg_name) => pkg_name,
        _ => return None,
    };
    if pkg_name.is_empty() || already_fetched.contains(pkg_name) {
        return None;
    }
    Some(pkg_name.clone())
}

async fn get_reply(
    path: String,
    npm_db: NpmRocksDB,
    is_json: bool,
) -> Result<CustomReply, ServerError> {
    let decoded_query = decode_base64(&path)?;
    let dep_requests = parse_query(decoded_query)?;

    let mut res_map: Option<ResolutionsMap> = None;
    let mut fetched_pkgs: HashSet<String> = HashSet::new();
    let mut last_err: Option<ServerError> = None;
    for _idx in 0..100 {
        let cloned_dep_requests = dep_requests.clone();
        let cloned_npm_db = npm_db.clone();
        let result: AppResult<ResolutionsMap> = tokio::task::spawn_blocking(move || {
            let mut tree_builder = DepTreeBuilder::new(cloned_npm_db);
            tree_builder.resolve_tree(cloned_dep_requests)?;
            for (alias_key, alias_value) in tree_builder.aliases {
                if let Some(resolved_version) = tree_builder.resolutions.get(&alias_value) {
                    tree_builder
                        .resolutions
                        .insert(alias_key, resolved_version.clone());
                }
            }
            Ok(tree_builder.resolutions)
        })
        .await?;

        match result {
            Ok(data) => {
                res_map = Some(data);
                break;
            }

            Err(err) => {
                let pkg_name = match pkg_to_fetch(&err, &fetched_pkgs) {
                    Some(pkg_name) => pkg_name,
                    None => return Err(err),
                };
                npm_db.fetch_missing_pkg(&pkg_name).await?;
                fetched_pkgs.insert(pkg_name);
                last_err = Some(err);
            }
        }
    }

    if res_map.is_none() {
        // The retry loop was exhausted, surface the last resolution error.
        return Err(last_err.unwrap_or(ServerError::PackageNotFound("unknown".to_string())));
    }

    let mut reply = match is_json {
        true => CustomReply::json(&res_map)?,
        false => CustomReply::msgpack(&res_map)?,
    };
    let cache_ttl = 3600;
    reply.add_header(
        "Cache-Control",
        format!("public, max-age={}", cache_ttl).as_str(),
    );
    reply.add_header(
        "CDN-Cache-Control",
        format!("max-age={}", cache_ttl).as_str(),
    );
    Ok(reply)
}

async fn deps_route_handler(
    path: String,
    npm_db: NpmRocksDB,
    is_json: bool,
) -> Result<impl Reply, Rejection> {
    match get_reply(path, npm_db, is_json).await {
        Ok(reply) => Ok(reply),
        Err(err) => Ok(ErrorReply::from(err).as_reply()),
    }
}

fn json_route(
    npm_db: NpmRocksDB,
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path!("v2" / "json" / "deps" / String)
        .and(warp::get())
        .and(with_data(npm_db))
        .and(with_data(true))
        .and_then(deps_route_handler)
}

fn msgpack_route(
    npm_db: NpmRocksDB,
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path!("v2" / "deps" / String)
        .and(warp::get())
        .and(with_data(npm_db))
        .and(with_data(false))
        .and_then(deps_route_handler)
}

pub fn deps_route(
    npm_db: NpmRocksDB,
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    json_route(npm_db.clone()).or(msgpack_route(npm_db))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fetched(names: &[&str]) -> HashSet<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    // Regression test: a package whose requested version was never published
    // (e.g. @mui/icons-material@9.1.2) is refreshed from npm once, and when
    // that doesn't help the original PackageVersionNotFound error must be
    // surfaced instead of being masked as PackageNotFound.
    #[test]
    fn missing_version_is_fetched_once_then_surfaced() {
        let err = ServerError::PackageVersionNotFound(
            "@mui/icons-material".to_string(),
            "9.1.2".to_string(),
        );
        assert_eq!(
            pkg_to_fetch(&err, &fetched(&[])),
            Some("@mui/icons-material".to_string())
        );
        assert_eq!(pkg_to_fetch(&err, &fetched(&["@mui/icons-material"])), None);
    }

    #[test]
    fn missing_package_is_fetched_once_then_surfaced() {
        let err = ServerError::PackageNotFound("left-pad".to_string());
        assert_eq!(pkg_to_fetch(&err, &fetched(&[])), Some("left-pad".to_string()));
        assert_eq!(pkg_to_fetch(&err, &fetched(&["left-pad"])), None);
    }

    #[test]
    fn other_errors_are_not_retried() {
        assert_eq!(
            pkg_to_fetch(&ServerError::InvalidPackageSpecifier, &fetched(&[])),
            None
        );
        assert_eq!(
            pkg_to_fetch(&ServerError::PackageNotFound(String::new()), &fetched(&[])),
            None
        );
    }
}
