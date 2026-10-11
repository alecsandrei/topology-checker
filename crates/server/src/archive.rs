use anyhow::{Context, Result, bail};
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
};
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

pub const UPLOAD_LIMIT: u64 = 512 * 1024 * 1024;
pub const EXPANSION_LIMIT: u64 = 2 * 1024 * 1024 * 1024;
const ENTRY_LIMIT: usize = 10_000;

/// Validate a relative archive member path.
///
/// # Errors
/// Returns an error for traversal, absolute paths or platform-specific path escapes.
pub fn safe_path(name: &str) -> Result<PathBuf> {
    if name.contains('\\') || name.contains(':') || name.contains('\0') {
        bail!("invalid archive path");
    }
    let path = Path::new(name);
    if path
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
        || name.is_empty()
    {
        bail!("unsafe archive path");
    }
    Ok(path.to_path_buf())
}
/// Extract only expected local vector files, never links or virtual GDAL paths.
/// Extract an archive of supported vector files or project sources.
///
/// # Errors
/// Returns an error for unsafe content, expansion limits or archive and filesystem failures.
pub fn extract(path: &Path, destination: &Path, project: bool) -> Result<Vec<PathBuf>> {
    let mut remaining = EXPANSION_LIMIT;
    extract_with_budget(path, destination, project, &mut remaining)
}

/// Extract an archive while sharing the expansion allowance with nested sources.
///
/// # Errors
/// Returns an error for unsafe content, exhausted expansion budget or I/O failures.
pub fn extract_with_budget(
    path: &Path,
    destination: &Path,
    project: bool,
    remaining: &mut u64,
) -> Result<Vec<PathBuf>> {
    let mut archive = ZipArchive::new(File::open(path)?)?;
    if archive.len() > ENTRY_LIMIT {
        bail!("archive contains too many entries");
    }
    let mut seen = HashSet::new();
    let mut output = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let relative = safe_path(entry.name().trim_end_matches('/'))?;
        if entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
            bail!("archive links are not supported");
        }
        if entry.is_dir() {
            continue;
        }
        if !seen.insert(relative.clone()) {
            bail!("duplicate archive path");
        }
        let ext = relative
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let supported = if project {
            relative == Path::new("manifest.json")
                || topology_checker_io::registry::project_extension(&ext)
        } else {
            matches!(
                ext.as_str(),
                "shp" | "shx" | "dbf" | "prj" | "cpg" | "qix" | "sbn" | "sbx"
            )
        };
        if !supported {
            bail!("unsupported archive content: {}", relative.display());
        }
        *remaining = remaining
            .checked_sub(entry.size())
            .context("archive expands beyond its 2 GiB allowance")?;
        let target = destination.join(&relative);
        fs::create_dir_all(target.parent().unwrap())?;
        let mut file = File::create(&target)?;
        let size = entry.size();
        let copied = io::copy(
            &mut Read::by_ref(&mut entry).take(size.saturating_add(1)),
            &mut file,
        )?;
        if copied != entry.size() {
            bail!("archive entry length mismatch");
        }
        output.push(relative);
    }
    Ok(output)
}

/// Pack a versioned project manifest and original source datasets.
///
/// # Errors
/// Returns an error for unsafe member paths or filesystem and archive write failures.
pub fn pack(output: &Path, manifest: &[u8], files: &[(PathBuf, PathBuf)]) -> Result<()> {
    pack_with_limit(output, manifest, files, UPLOAD_LIMIT)
}

fn pack_with_limit(
    output: &Path,
    manifest: &[u8],
    files: &[(PathBuf, PathBuf)],
    limit: u64,
) -> Result<()> {
    if files.len() >= ENTRY_LIMIT {
        bail!("project contains too many source files to reopen");
    }
    let mut zip = ZipWriter::new(File::create(output)?);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    zip.start_file("manifest.json", options)?;
    zip.write_all(manifest)?;
    for (name, path) in files {
        safe_path(name.to_str().context("invalid filename")?)?;
        zip.start_file(name.to_str().unwrap(), options)?;
        io::copy(&mut File::open(path)?, &mut zip)?;
    }
    if zip.finish()?.metadata()?.len() > limit {
        bail!("project archive exceeds the 512 MiB upload limit; save fewer datasets together");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_traversal_absolute_paths_backslashes_and_duplicate_members() {
        for name in [
            "../escape.shp",
            "/escape.shp",
            "folder/../../escape.shp",
            "C:/escape.shp",
            "folder\\escape.shp",
        ] {
            assert!(safe_path(name).is_err());
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("unsafe.zip");
        let mut writer = ZipWriter::new(File::create(&path).unwrap());
        writer
            .start_file("../escape.shp", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"bad").unwrap();
        writer.finish().unwrap();
        assert!(extract(&path, dir.path(), false).is_err());
    }
    #[test]
    fn rejects_unsupported_content_and_links() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("unsupported.zip");
        let mut writer = ZipWriter::new(File::create(&path).unwrap());
        writer
            .start_file("script.exe", SimpleFileOptions::default())
            .unwrap();
        writer.finish().unwrap();
        assert!(extract(&path, dir.path(), false).is_err());
        let path = dir.path().join("link.zip");
        let mut writer = ZipWriter::new(File::create(&path).unwrap());
        writer
            .add_symlink("link.shp", "/etc/passwd", SimpleFileOptions::default())
            .unwrap();
        writer.finish().unwrap();
        assert!(extract(&path, dir.path(), false).is_err());
    }

    #[test]
    fn expansion_budget_is_shared_across_archives() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("data.zip");
        let mut zip = ZipWriter::new(File::create(&path).unwrap());
        zip.start_file("data.shp", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&[0_u8; 100]).unwrap();
        zip.finish().unwrap();
        let mut remaining = 199;
        extract_with_budget(&path, directory.path(), false, &mut remaining).unwrap();
        assert_eq!(remaining, 99);
        assert!(extract_with_budget(&path, directory.path(), false, &mut remaining).is_err());
    }

    #[test]
    fn saved_projects_must_fit_the_reopening_upload_limit() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("project.zip");
        let source = directory.path().join("source.geojson");
        fs::write(&source, b"{\"type\":\"FeatureCollection\",\"features\":[]}").unwrap();
        let files = [(PathBuf::from("sources/example/source.geojson"), source)];
        let manifest = b"{\"version\":1}";
        pack_with_limit(&output, manifest, &files, 4096).unwrap();
        let mut zip = ZipArchive::new(File::open(&output).unwrap()).unwrap();
        assert_eq!(zip.len(), 2);
        let mut original = Vec::new();
        zip.by_name("sources/example/source.geojson")
            .unwrap()
            .read_to_end(&mut original)
            .unwrap();
        assert_eq!(original, fs::read(&files[0].1).unwrap());
        drop(zip);
        assert!(
            pack_with_limit(&output, manifest, &files, 128)
                .unwrap_err()
                .to_string()
                .contains("upload limit")
        );
    }
}
