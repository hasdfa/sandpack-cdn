use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DefaultOnError};
use std::collections::BTreeMap;

use crate::{npm::package_data::PackageMetadata, utils::time::secs_since_epoch};

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
pub struct DocumentPackageDist {
    pub tarball: String,
}

#[serde_as]
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
pub struct DocumentPackageVersion {
    #[serde(default)]
    #[serde_as(deserialize_as = "DefaultOnError")]
    pub dependencies: Option<BTreeMap<String, String>>,
    #[serde(default, rename = "optionalDependencies")]
    #[serde_as(deserialize_as = "DefaultOnError")]
    pub optional_dependencies: Option<BTreeMap<String, String>>,
    pub dist: DocumentPackageDist,
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
pub struct RegistryDocument {
    #[serde(rename = "_id")]
    pub id: String,

    #[serde(default, rename = "_deleted")]
    pub deleted: bool,

    #[serde(rename = "dist-tags")]
    pub dist_tags: Option<BTreeMap<String, String>>,

    pub versions: Option<BTreeMap<String, DocumentPackageVersion>>,
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
pub struct MinimalPackageVersionData {
    pub tarball: String,
    pub dependencies: BTreeMap<String, String>,
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone, Default)]
pub struct MinimalPackageData {
    pub name: String,
    pub dist_tags: BTreeMap<String, String>,
    pub versions: BTreeMap<String, MinimalPackageVersionData>,
    // Seconds since the epoch. Rows written before this field existed lack
    // the key entirely; defaulting to None makes them deserialize instead of
    // erroring, which triggers the on-demand refresh in fetch_missing_pkg.
    #[serde(default)]
    pub last_updated: Option<u64>,
}

impl MinimalPackageData {
    pub fn from_registry_meta(raw: PackageMetadata) -> MinimalPackageData {
        let mut data = MinimalPackageData {
            name: raw.name,
            dist_tags: raw.dist_tags,
            versions: BTreeMap::new(),
            last_updated: Some(secs_since_epoch()),
        };
        for (key, value) in raw.versions {
            data.versions.insert(
                key,
                MinimalPackageVersionData {
                    tarball: value.dist.tarball,
                    dependencies: value.dependencies,
                },
            );
        }
        data
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::msgpack::serialize_msgpack;

    // The shape rows had before the last_updated field was added; DBs
    // populated by the old replication task still contain such rows.
    #[derive(Serialize)]
    struct LegacyMinimalPackageData {
        name: String,
        dist_tags: BTreeMap<String, String>,
        versions: BTreeMap<String, MinimalPackageVersionData>,
    }

    #[test]
    fn legacy_row_without_last_updated_decodes() {
        let mut versions = BTreeMap::new();
        versions.insert(
            "1.0.0".to_string(),
            MinimalPackageVersionData {
                tarball: "https://example.com/react-1.0.0.tgz".to_string(),
                dependencies: BTreeMap::new(),
            },
        );
        let legacy = LegacyMinimalPackageData {
            name: "react".to_string(),
            dist_tags: BTreeMap::new(),
            versions,
        };

        let bytes = serialize_msgpack(&legacy).unwrap();
        let decoded: MinimalPackageData = rmp_serde::from_slice(&bytes).unwrap();

        assert_eq!(decoded.name, "react");
        assert!(decoded.versions.contains_key("1.0.0"));
        // None marks the row as stale, so fetch_missing_pkg refreshes it.
        assert_eq!(decoded.last_updated, None);
    }
}
