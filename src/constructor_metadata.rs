//! Constructor-compatible conda prefix metadata.

use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use miette::{Context, IntoDiagnostic};
use rattler_conda_types::{Platform, RepoDataRecord};

use crate::{install, policy};

#[derive(serde::Serialize)]
struct InstallerInfo<'a> {
    name: &'a str,
    version: &'a str,
    platform: String,
    #[serde(rename = "type")]
    installer_type: &'a str,
}

pub(crate) fn write_prefix_metadata(
    prefix: &Path,
    lock_content: &str,
    requested_specs: &[String],
) -> miette::Result<()> {
    let (platform, records) = install::lockfile_records_for_current_platform(lock_content)?;
    write_prefix_metadata_from_records(prefix, platform, &records, requested_specs)
}

fn write_prefix_metadata_from_records(
    prefix: &Path,
    platform: Platform,
    records: &[RepoDataRecord],
    requested_specs: &[String],
) -> miette::Result<()> {
    let conda_meta = prefix.join("conda-meta");
    std::fs::create_dir_all(&conda_meta)
        .into_diagnostic()
        .with_context(|| format!("failed to create {}", policy::path_for_display(&conda_meta)))?;

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string());

    let history_path = conda_meta.join("history");
    std::fs::write(
        &history_path,
        render_history(records, requested_specs, &timestamp)?,
    )
    .into_diagnostic()
    .with_context(|| {
        format!(
            "failed to write {}",
            policy::path_for_display(&history_path)
        )
    })?;
    eprintln!("   Wrote {}", policy::path_for_display(&history_path));

    let explicit_path = conda_meta.join("initial-state.explicit.txt");
    std::fs::write(
        &explicit_path,
        render_initial_state_explicit(platform, records),
    )
    .into_diagnostic()
    .with_context(|| {
        format!(
            "failed to write {}",
            policy::path_for_display(&explicit_path)
        )
    })?;
    eprintln!("   Wrote {}", policy::path_for_display(&explicit_path));

    write_configured_installer_info(prefix, platform)?;

    Ok(())
}

fn write_configured_installer_info(prefix: &Path, platform: Platform) -> miette::Result<()> {
    let Some(installer_type) = policy::installer() else {
        return Ok(());
    };
    write_installer_info(
        prefix,
        policy::runtime_name(),
        policy::runtime_version(),
        platform,
        installer_type,
    )
}

fn write_installer_info(
    prefix: &Path,
    name: &str,
    version: &str,
    platform: Platform,
    installer_type: &str,
) -> miette::Result<()> {
    let path = prefix.join(".installer.info");
    let contents = serde_json::to_string(&InstallerInfo {
        name,
        version,
        platform: platform.to_string(),
        installer_type,
    })
    .into_diagnostic()
    .context("failed to render Constructor installer metadata")?;
    std::fs::write(&path, contents)
        .into_diagnostic()
        .with_context(|| format!("failed to write {}", policy::path_for_display(&path)))?;
    eprintln!("   Wrote {}", policy::path_for_display(&path));
    Ok(())
}

fn render_history(
    records: &[RepoDataRecord],
    requested_specs: &[String],
    timestamp: &str,
) -> miette::Result<String> {
    let mut dists: Vec<_> = records.iter().map(history_dist).collect();
    dists.sort();

    let mut content = format!("==> {timestamp} <==\n");
    content.push_str(&format!(
        "# cmd: {} [automatic bootstrap]\n",
        policy::command_name()
    ));
    for dist in dists {
        content.push('+');
        content.push_str(&dist);
        content.push('\n');
    }
    if !requested_specs.is_empty() {
        let specs = serde_json::to_string(requested_specs)
            .into_diagnostic()
            .context("failed to render requested specs for conda history")?;
        content.push_str("# update specs: ");
        content.push_str(&specs);
        content.push('\n');
    }
    content.push('\n');
    Ok(content)
}

fn render_initial_state_explicit(platform: Platform, records: &[RepoDataRecord]) -> String {
    let mut explicit_lines: Vec<_> = records.iter().map(explicit_url).collect();
    explicit_lines.sort();

    let mut content = "\
# This file may be used to create an environment using:
# $ conda create --name <env> --file <this file>
"
    .to_string();
    content.push_str(&format!("# platform: {platform}\n"));
    content.push_str("@EXPLICIT\n");
    for line in explicit_lines {
        content.push_str(&line);
        content.push('\n');
    }
    content
}

fn history_dist(record: &RepoDataRecord) -> String {
    let subdir = record.package_record.subdir.as_str();
    let channel = record
        .channel
        .as_deref()
        .filter(|channel| !channel.is_empty())
        .map(|channel| channel_root(channel, subdir))
        .unwrap_or_else(|| channel_root_from_url(record, subdir));

    format!("{channel}/{subdir}::{}", record.identifier.identifier)
}

fn channel_root(channel: &str, subdir: &str) -> String {
    let channel = channel.trim_end_matches('/');
    let suffix = format!("/{subdir}");
    channel.strip_suffix(&suffix).unwrap_or(channel).to_string()
}

fn channel_root_from_url(record: &RepoDataRecord, subdir: &str) -> String {
    let url = record.url.as_str().trim_end_matches('/');
    let channel_with_subdir = url
        .rsplit_once('/')
        .map(|(prefix, _)| prefix)
        .unwrap_or(url);
    channel_root(channel_with_subdir, subdir)
}

fn explicit_url(record: &RepoDataRecord) -> String {
    let mut url = record.url.clone();
    url.set_fragment(None);
    let mut url = url.to_string();
    if let Some(hash) = record.package_record.sha256.as_ref() {
        url.push_str("#sha256:");
        url.push_str(&crate::hash::hex(hash.as_slice()));
    } else if let Some(hash) = record.package_record.md5.as_ref() {
        url.push('#');
        url.push_str(&crate::hash::hex(hash.as_slice()));
    }
    url
}

#[cfg(test)]
mod tests {
    use super::*;
    use rattler_conda_types::{
        PackageName, PackageRecord, VersionWithSource,
        package::{CondaArchiveIdentifier, DistArchiveIdentifier},
    };
    use rstest::rstest;
    use std::str::FromStr;
    use tempfile::TempDir;

    const SHA256: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const MD5: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn make_record(
        name: &str,
        subdir: &str,
        channel: Option<&str>,
        checksum_field: &str,
        checksum: &str,
    ) -> RepoDataRecord {
        let mut record = PackageRecord::new(
            PackageName::new_unchecked(name),
            VersionWithSource::from_str("1.0").unwrap(),
            "0".to_string(),
        );
        record.subdir = subdir.to_string();
        let mut record_json = serde_json::to_value(&record).unwrap();
        record_json[checksum_field] = serde_json::json!(checksum);
        let record = serde_json::from_value(record_json).unwrap();
        let filename = format!("{name}-1.0-0.conda");

        RepoDataRecord {
            package_record: record,
            identifier: DistArchiveIdentifier::from(
                filename.parse::<CondaArchiveIdentifier>().unwrap(),
            ),
            url: format!("https://conda.anaconda.org/conda-forge/{subdir}/{filename}")
                .parse()
                .unwrap(),
            channel: channel.map(ToString::to_string),
        }
    }

    #[rstest]
    #[case::channel_url(Some("https://conda.anaconda.org/conda-forge"))]
    #[case::channel_url_with_subdir(Some("https://conda.anaconda.org/conda-forge/noarch/"))]
    #[case::url_fallback(None)]
    fn test_render_history_matches_conda_history_shape(#[case] channel: Option<&str>) {
        let records = vec![make_record(
            "conda-spawn",
            "noarch",
            channel,
            "sha256",
            SHA256,
        )];

        let history = render_history(&records, &["conda-spawn".to_string()], "123").unwrap();

        assert_eq!(
            history,
            "\
==> 123 <==
# cmd: cs-template [automatic bootstrap]
+https://conda.anaconda.org/conda-forge/noarch::conda-spawn-1.0-0
# update specs: [\"conda-spawn\"]

"
        );
    }

    #[rstest]
    #[case::sha256(
        "sha256",
        SHA256,
        "#sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    )]
    #[case::md5("md5", MD5, "#bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")]
    fn test_explicit_url_adds_checksum_fragment(
        #[case] checksum_field: &str,
        #[case] checksum: &str,
        #[case] expected_fragment: &str,
    ) {
        let record = make_record(
            "conda",
            "linux-64",
            Some("conda-forge"),
            checksum_field,
            checksum,
        );

        assert_eq!(
            explicit_url(&record),
            format!(
                "https://conda.anaconda.org/conda-forge/linux-64/conda-1.0-0.conda{expected_fragment}"
            )
        );
    }

    #[test]
    fn test_render_initial_state_explicit_includes_header_and_sorted_urls() {
        let records = vec![
            make_record(
                "conda-spawn",
                "noarch",
                Some("conda-forge"),
                "sha256",
                SHA256,
            ),
            make_record("conda", "linux-64", Some("conda-forge"), "md5", MD5),
        ];

        let explicit = render_initial_state_explicit(Platform::Linux64, &records);

        assert_eq!(
            explicit,
            format!(
                "\
# This file may be used to create an environment using:
# $ conda create --name <env> --file <this file>
# platform: linux-64
@EXPLICIT
https://conda.anaconda.org/conda-forge/linux-64/conda-1.0-0.conda#{MD5}
https://conda.anaconda.org/conda-forge/noarch/conda-spawn-1.0-0.conda#sha256:{SHA256}
"
            )
        );
    }

    #[test]
    fn test_write_prefix_metadata_from_records_writes_constructor_files() {
        let tmp = TempDir::new().unwrap();
        let records = vec![make_record(
            "conda",
            "linux-64",
            Some("https://conda.anaconda.org/conda-forge/linux-64/"),
            "sha256",
            SHA256,
        )];

        write_prefix_metadata_from_records(
            tmp.path(),
            Platform::Linux64,
            &records,
            &["conda".to_string()],
        )
        .unwrap();

        let history =
            std::fs::read_to_string(tmp.path().join("conda-meta").join("history")).unwrap();
        assert!(
            history.contains("+https://conda.anaconda.org/conda-forge/linux-64::conda-1.0-0"),
            "{history}"
        );
        assert!(history.contains("# update specs: [\"conda\"]"), "{history}");

        let explicit = std::fs::read_to_string(
            tmp.path()
                .join("conda-meta")
                .join("initial-state.explicit.txt"),
        )
        .unwrap();
        assert!(explicit.contains("# platform: linux-64"), "{explicit}");
        assert!(
            explicit.contains(&format!(
                "https://conda.anaconda.org/conda-forge/linux-64/conda-1.0-0.conda#sha256:{SHA256}"
            )),
            "{explicit}"
        );
        assert!(!tmp.path().join(".installer.info").exists());
    }

    #[test]
    fn test_write_installer_info_matches_constructor_json() {
        let tmp = TempDir::new().unwrap();

        write_installer_info(
            tmp.path(),
            "Demo Distribution",
            "1.2.3",
            Platform::Linux64,
            "homebrew",
        )
        .unwrap();

        let contents = std::fs::read_to_string(tmp.path().join(".installer.info")).unwrap();
        assert_eq!(
            contents,
            r#"{"name":"Demo Distribution","version":"1.2.3","platform":"linux-64","type":"homebrew"}"#
        );
    }

    #[test]
    fn test_write_prefix_metadata_uses_lockfile_records() {
        let tmp = TempDir::new().unwrap();
        let platform = Platform::current();
        let url = format!("https://conda.anaconda.org/conda-forge/{platform}/conda-1.0-0.conda");
        let lock_content = format!(
            r#"
---
version: 6
environments:
  default:
    channels:
      - url: https://conda.anaconda.org/conda-forge
    packages:
      {platform}:
        - conda: {url}
packages:
  - conda: {url}
    sha256: {SHA256}
"#
        );

        write_prefix_metadata(tmp.path(), &lock_content, &["conda".to_string()]).unwrap();

        let history =
            std::fs::read_to_string(tmp.path().join("conda-meta").join("history")).unwrap();
        assert!(
            history.contains(&format!(
                "+https://conda.anaconda.org/conda-forge/{platform}::conda-1.0-0"
            )),
            "{history}"
        );

        let explicit = std::fs::read_to_string(
            tmp.path()
                .join("conda-meta")
                .join("initial-state.explicit.txt"),
        )
        .unwrap();
        assert!(
            explicit.contains(&format!("# platform: {platform}")),
            "{explicit}"
        );
        assert!(
            explicit.contains(&format!("{url}#sha256:{SHA256}")),
            "{explicit}"
        );
    }
}
