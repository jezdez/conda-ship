use std::collections::{BTreeMap, BTreeSet};

use jiff::Timestamp;
use miette::{Context, IntoDiagnostic};
use rattler_conda_types::{PackageName, PackageUrl, Platform};
use rattler_lock::CondaPackageData;

use super::BundleLayout;
use super::project::package_record;

const CYCLONEDX_SCHEMA: &str = "https://cyclonedx.org/schema/bom-1.7.schema.json";

#[derive(serde::Serialize)]
struct CycloneDxBom {
    #[serde(rename = "$schema")]
    schema: &'static str,
    #[serde(rename = "bomFormat")]
    bom_format: &'static str,
    #[serde(rename = "specVersion")]
    spec_version: &'static str,
    version: u8,
    metadata: Metadata,
    components: Vec<Component>,
    dependencies: Vec<Dependency>,
    compositions: Vec<Composition>,
}

#[derive(serde::Serialize)]
struct Metadata {
    timestamp: String,
    lifecycles: Vec<Lifecycle>,
    tools: Tools,
    component: Component,
}

#[derive(serde::Serialize)]
struct Lifecycle {
    phase: &'static str,
}

#[derive(serde::Serialize)]
struct Tools {
    components: Vec<Component>,
}

#[derive(serde::Serialize)]
struct Component {
    #[serde(rename = "type")]
    component_type: &'static str,
    #[serde(rename = "bom-ref")]
    bom_ref: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scope: Option<&'static str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    hashes: Vec<Hash>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    licenses: Vec<NamedLicense>,
    #[serde(skip_serializing_if = "Option::is_none")]
    purl: Option<String>,
    #[serde(rename = "externalReferences", skip_serializing_if = "Vec::is_empty")]
    external_references: Vec<ExternalReference>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    properties: Vec<Property>,
}

#[derive(serde::Serialize)]
struct Hash {
    alg: &'static str,
    content: String,
}

#[derive(serde::Serialize)]
struct NamedLicense {
    license: License,
}

#[derive(serde::Serialize)]
struct License {
    name: String,
}

#[derive(serde::Serialize)]
struct ExternalReference {
    #[serde(rename = "type")]
    reference_type: &'static str,
    url: String,
}

#[derive(serde::Serialize)]
struct Property {
    name: &'static str,
    value: String,
}

#[derive(serde::Serialize)]
struct Dependency {
    #[serde(rename = "ref")]
    reference: String,
    #[serde(rename = "dependsOn")]
    depends_on: Vec<String>,
}

#[derive(serde::Serialize)]
struct Composition {
    aggregate: &'static str,
    assemblies: Vec<String>,
    dependencies: Vec<String>,
}

struct PackageComponent {
    name: String,
    component: Component,
    dependencies: Vec<String>,
}

pub(crate) fn creation_timestamp() -> miette::Result<String> {
    let timestamp = match std::env::var("SOURCE_DATE_EPOCH") {
        Ok(value) => {
            let seconds = value
                .parse::<i64>()
                .into_diagnostic()
                .with_context(|| format!("SOURCE_DATE_EPOCH must be an integer, got {value:?}"))?;
            Timestamp::from_second(seconds)
                .into_diagnostic()
                .context("SOURCE_DATE_EPOCH is outside the supported timestamp range")?
        }
        Err(std::env::VarError::NotPresent) => Timestamp::now(),
        Err(error) => {
            return Err(error)
                .into_diagnostic()
                .context("failed to read SOURCE_DATE_EPOCH");
        }
    };
    Ok(timestamp.to_string())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_cyclonedx_sbom(
    runtime_name: &str,
    artifact_name: &str,
    runtime_version: &str,
    layout: BundleLayout,
    platform: Platform,
    binary_filename: &str,
    packages: &[&CondaPackageData],
    timestamp: String,
) -> miette::Result<String> {
    let root_ref = format!("runtime:{artifact_name}@{runtime_version}?platform={platform}");
    let root = Component {
        component_type: "application",
        bom_ref: root_ref.clone(),
        name: artifact_name.to_string(),
        version: Some(runtime_version.to_string()),
        scope: Some("required"),
        hashes: Vec::new(),
        licenses: Vec::new(),
        purl: None,
        external_references: Vec::new(),
        properties: vec![
            Property {
                name: "conda-ship:runtime:name",
                value: runtime_name.to_string(),
            },
            Property {
                name: "conda-ship:artifact:filename",
                value: binary_filename.to_string(),
            },
            Property {
                name: "conda-ship:artifact:layout",
                value: layout.as_str().to_string(),
            },
            Property {
                name: "conda-ship:target:platform",
                value: platform.to_string(),
            },
            Property {
                name: "conda-ship:sbom:scope",
                value: "resolved-conda-packages".to_string(),
            },
        ],
    };

    let mut package_components = packages
        .iter()
        .map(|package| package_component(package, platform))
        .collect::<miette::Result<Vec<_>>>()?;
    package_components.sort_by(|a, b| a.component.bom_ref.cmp(&b.component.bom_ref));

    let references_by_name: BTreeMap<_, _> = package_components
        .iter()
        .map(|package| (package.name.as_str(), package.component.bom_ref.as_str()))
        .collect();
    let mut depended_on = BTreeSet::new();
    let mut dependencies = Vec::with_capacity(package_components.len() + 1);
    for package in &package_components {
        let mut depends_on = package
            .dependencies
            .iter()
            .filter_map(|name| references_by_name.get(name.as_str()).copied())
            .map(str::to_string)
            .collect::<Vec<_>>();
        depends_on.sort();
        depends_on.dedup();
        depended_on.extend(depends_on.iter().cloned());
        dependencies.push(Dependency {
            reference: package.component.bom_ref.clone(),
            depends_on,
        });
    }

    let mut root_dependencies = package_components
        .iter()
        .map(|package| package.component.bom_ref.clone())
        .filter(|reference| !depended_on.contains(reference))
        .collect::<Vec<_>>();
    if root_dependencies.is_empty() {
        root_dependencies = package_components
            .iter()
            .map(|package| package.component.bom_ref.clone())
            .collect();
    } else {
        let dependency_edges: BTreeMap<_, _> = dependencies
            .iter()
            .map(|dependency| (dependency.reference.clone(), dependency.depends_on.clone()))
            .collect();
        let mut reachable = BTreeSet::new();
        let add_reachable = |seeds: Vec<String>, reachable: &mut BTreeSet<String>| {
            let mut pending = seeds;
            while let Some(reference) = pending.pop() {
                if reachable.insert(reference.clone())
                    && let Some(depends_on) = dependency_edges.get(&reference)
                {
                    pending.extend(depends_on.iter().cloned());
                }
            }
        };
        add_reachable(root_dependencies.clone(), &mut reachable);
        for package in &package_components {
            let reference = &package.component.bom_ref;
            if !reachable.contains(reference) {
                root_dependencies.push(reference.clone());
                add_reachable(vec![reference.clone()], &mut reachable);
            }
        }
    }
    root_dependencies.sort();
    dependencies.push(Dependency {
        reference: root_ref.clone(),
        depends_on: root_dependencies,
    });
    dependencies.sort_by(|a, b| a.reference.cmp(&b.reference));

    let bom = CycloneDxBom {
        schema: CYCLONEDX_SCHEMA,
        bom_format: "CycloneDX",
        spec_version: "1.7",
        version: 1,
        metadata: Metadata {
            timestamp,
            lifecycles: vec![Lifecycle {
                phase: "post-build",
            }],
            tools: Tools {
                components: vec![Component {
                    component_type: "application",
                    bom_ref: format!("tool:conda-ship@{}", env!("CARGO_PKG_VERSION")),
                    name: "conda-ship".to_string(),
                    version: Some(env!("CARGO_PKG_VERSION").to_string()),
                    scope: None,
                    hashes: Vec::new(),
                    licenses: Vec::new(),
                    purl: None,
                    external_references: Vec::new(),
                    properties: Vec::new(),
                }],
            },
            component: root,
        },
        components: package_components
            .into_iter()
            .map(|package| package.component)
            .collect(),
        dependencies,
        compositions: vec![Composition {
            aggregate: "incomplete",
            assemblies: vec![root_ref.clone()],
            dependencies: vec![root_ref],
        }],
    };

    let mut content = serde_json::to_string_pretty(&bom)
        .into_diagnostic()
        .context("failed to render CycloneDX SBOM")?;
    content.push('\n');
    Ok(content)
}

fn package_component(
    package: &CondaPackageData,
    platform: Platform,
) -> miette::Result<PackageComponent> {
    let record = package_record(package)?;
    let name = record.name.as_normalized().to_string();
    let version = record.version.to_string();
    let subdir = if record.subdir.is_empty() {
        platform.to_string()
    } else {
        record.subdir.clone()
    };
    let binary = package.as_binary();
    let channel = binary
        .and_then(|package| package.channel.as_ref())
        .map(ToString::to_string);
    let package_filename = binary.map(|package| package.file_name.to_string());
    let package_type = package_filename.as_deref().and_then(|filename| {
        if filename.ends_with(".conda") {
            Some("conda")
        } else if filename.ends_with(".tar.bz2") {
            Some("tar.bz2")
        } else {
            None
        }
    });

    let mut purl = PackageUrl::builder("conda".to_string(), name.clone())
        .with_version(version.clone())
        .with_qualifier("build", record.build.clone())
        .into_diagnostic()
        .context("failed to add conda build to package URL")?
        .with_qualifier("subdir", subdir.clone())
        .into_diagnostic()
        .context("failed to add conda subdir to package URL")?;
    if let Some(channel) = channel.as_deref() {
        purl = purl
            .with_qualifier("channel", channel)
            .into_diagnostic()
            .context("failed to add conda channel to package URL")?;
    }
    if let Some(package_type) = package_type {
        purl = purl
            .with_qualifier("type", package_type)
            .into_diagnostic()
            .context("failed to add conda archive type to package URL")?;
    }
    let purl = purl
        .build()
        .into_diagnostic()
        .context("failed to build conda package URL")?
        .to_string();

    let mut hashes = Vec::new();
    if let Some(hash) = record.sha256.as_ref() {
        hashes.push(Hash {
            alg: "SHA-256",
            content: crate::hash::hex(hash.as_slice()),
        });
    }
    if let Some(hash) = record.md5.as_ref() {
        hashes.push(Hash {
            alg: "MD5",
            content: crate::hash::hex(hash.as_slice()),
        });
    }
    let licenses = record
        .license
        .as_ref()
        .filter(|license| !license.is_empty())
        .map(|license| {
            vec![NamedLicense {
                license: License {
                    name: license.clone(),
                },
            }]
        })
        .unwrap_or_default();
    let external_references = package
        .location()
        .as_url()
        .map(|url| {
            vec![ExternalReference {
                reference_type: "distribution",
                url: url.to_string(),
            }]
        })
        .unwrap_or_default();
    let mut properties = vec![
        Property {
            name: "conda:package:build",
            value: record.build.clone(),
        },
        Property {
            name: "conda:package:build-number",
            value: record.build_number.to_string(),
        },
        Property {
            name: "conda:package:subdir",
            value: subdir,
        },
    ];
    if let Some(channel) = channel {
        properties.push(Property {
            name: "conda:package:channel",
            value: channel,
        });
    }
    if let Some(filename) = package_filename {
        properties.push(Property {
            name: "conda:package:filename",
            value: filename,
        });
    }
    if let Some(size) = record.size {
        properties.push(Property {
            name: "conda:package:size",
            value: size.to_string(),
        });
    }

    Ok(PackageComponent {
        name,
        component: Component {
            component_type: "library",
            bom_ref: purl.clone(),
            name: record.name.as_normalized().to_string(),
            version: Some(version),
            scope: Some("required"),
            hashes,
            licenses,
            purl: Some(purl),
            external_references,
            properties,
        },
        dependencies: record
            .depends
            .iter()
            .map(|dependency| {
                PackageName::from_matchspec_str_unchecked(dependency)
                    .as_normalized()
                    .to_string()
            })
            .collect(),
    })
}
