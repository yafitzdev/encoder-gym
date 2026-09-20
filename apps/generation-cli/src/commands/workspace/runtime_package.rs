//! Project-contained Nomos runtime publication and resolution.

use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, ensure};
use chrono::Utc;
use encoder_experiment_core::{domain::ExternalProjectSnapshot, ports::EncoderTaskBackend};
use encoder_experiment_nomos::NomosBackend;
use project_workspace_core::{
    AdapterBinding, BoundIdentity, FileIdentity, ManagedRuntimePackage, RuntimeKind,
    RuntimePackageInventory, ScientificBinding,
};
use sha2::{Digest, Sha256};

use super::inspect_python_runtime;

const PACKAGE_MANIFEST: &str = "runtime-package.json";

#[derive(Debug, Clone)]
pub(super) struct PythonPackageSource {
    base_root: Option<PathBuf>,
    environment_root: Option<PathBuf>,
    site_roots: Vec<PathBuf>,
    selected_executable: PathBuf,
    base_executable: Option<PathBuf>,
}

impl PythonPackageSource {
    pub(super) fn standalone(executable: &Path) -> anyhow::Result<Self> {
        Ok(Self {
            base_root: None,
            environment_root: None,
            site_roots: vec![],
            selected_executable: executable.canonicalize()?,
            base_executable: None,
        })
    }

    pub(super) fn from_inspection(
        selected_executable: &Path,
        prefix: &str,
        base_prefix: &str,
        base_executable: &str,
        purelib: &str,
        platlib: &str,
    ) -> anyhow::Result<Self> {
        let selected_executable = selected_executable.canonicalize()?;
        let environment_root = canonical_plain(Path::new(prefix))?;
        let base_root = canonical_plain(Path::new(base_prefix))?;
        let reported_base_executable = canonical_plain(Path::new(base_executable))?;
        ensure!(
            selected_executable.starts_with(&environment_root)
                || selected_executable.starts_with(&base_root),
            "The selected Python executable is outside its reported runtime roots."
        );
        ensure!(
            reported_base_executable.starts_with(&base_root) && reported_base_executable.is_file(),
            "Python reported a base executable outside its base runtime."
        );
        let mut site_roots = Vec::new();
        for value in [purelib, platlib] {
            let root = canonical_plain(Path::new(value))?;
            ensure!(
                root.starts_with(&environment_root),
                "Python site-packages escaped its selected environment."
            );
            if !site_roots.contains(&root) {
                site_roots.push(root);
            }
        }
        Ok(Self {
            base_root: Some(base_root),
            environment_root: Some(environment_root),
            site_roots,
            selected_executable,
            base_executable: Some(reported_base_executable),
        })
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ManagedPackagePreview {
    action: &'static str,
    destination: &'static str,
    runtime_files: usize,
    runtime_bytes: u64,
    python_files: usize,
    python_bytes: u64,
    total_bytes: u64,
    self_contained: bool,
}

#[derive(Debug)]
pub(super) struct PublishedNomosPackage {
    pub(super) package: ManagedRuntimePackage,
    pub(super) runtime_location: String,
    pub(super) executable: String,
}

#[derive(Debug)]
pub(super) struct ResolvedNomosPackage {
    pub(super) runtime: PathBuf,
    pub(super) executable: PathBuf,
}

#[derive(Debug, Clone)]
struct PlannedFile {
    source: PathBuf,
    target: String,
    bytes: u64,
}

#[derive(Debug)]
struct PackagePlan {
    runtime: Vec<PlannedFile>,
    python: Vec<PlannedFile>,
}

impl PackagePlan {
    fn preview(&self) -> anyhow::Result<ManagedPackagePreview> {
        let runtime_bytes = sum_bytes(&self.runtime)?;
        let python_bytes = sum_bytes(&self.python)?;
        Ok(ManagedPackagePreview {
            action: "publish_managed_runtime_package",
            destination: "runtimes/nomos/<package-sha256>",
            runtime_files: self.runtime.len(),
            runtime_bytes,
            python_files: self.python.len(),
            python_bytes,
            total_bytes: runtime_bytes
                .checked_add(python_bytes)
                .context("Managed package size overflow")?,
            self_contained: true,
        })
    }
}

pub(super) fn preview(
    runtime: &Path,
    python: &PythonPackageSource,
) -> anyhow::Result<ManagedPackagePreview> {
    plan(runtime, python)?.preview()
}

pub(super) async fn publish(
    project_root: &Path,
    runtime_root: &Path,
    python_source: &PythonPackageSource,
    adapter: AdapterBinding,
    project: &ExternalProjectSnapshot,
) -> anyhow::Result<PublishedNomosPackage> {
    let project_root = canonical_plain(project_root)?;
    let packages_root = project_root.join("runtimes/nomos");
    fs::create_dir_all(&packages_root)?;
    ensure!(canonical_plain(&packages_root)?.starts_with(&project_root));
    let package_plan = plan(runtime_root, python_source)?;
    let staging = tempfile::Builder::new()
        .prefix(".runtime-package-")
        .tempdir_in(&packages_root)?;
    let runtime_files = copy_files(staging.path(), &package_plan.runtime)?;
    let python_files = copy_files(staging.path(), &package_plan.python)?;
    let runtime_inventory = RuntimePackageInventory::new(runtime_files)?;
    let python_inventory = RuntimePackageInventory::new(python_files)?;
    let staged_runtime = staging.path().join("workspace");
    let executable_relative = packaged_executable(python_source)?;
    let staged_executable = staging
        .path()
        .join(path_from_portable(&executable_relative));

    let staged_inspection = inspect_python_runtime(&staged_executable, &staged_runtime).await?;
    ensure!(
        staged_inspection.ready,
        "The copied Python runtime failed its offline capability check."
    );
    if python_source.base_root.is_some() {
        verify_python_containment(&staged_executable, &staged_runtime, staging.path()).await?;
    }
    let staged_backend = NomosBackend::open(&staged_runtime, &staged_executable)?
        .with_baseline_model(project.baseline_model.clone())?;
    staged_backend
        .verify_current_snapshot(project.clone())
        .await?;
    let identity = staged_backend.identity();
    ensure!(
        identity.name == adapter.key
            && identity.protocol_version == adapter.protocol
            && identity.configuration_fingerprint == adapter.configuration_fingerprint,
        "The packaged Nomos adapter identity differs from the verified source."
    );

    let package = ManagedRuntimePackage::new(
        adapter,
        BoundIdentity {
            id: project.id.to_string(),
            fingerprint: project.fingerprint.clone(),
        },
        project.source_revision.clone(),
        project.source_fingerprint.clone(),
        "workspace",
        executable_relative.clone(),
        runtime_inventory,
        python_inventory,
        Utc::now(),
    )?;
    write_new(
        &staging.path().join(PACKAGE_MANIFEST),
        &serde_json::to_vec_pretty(&package)?,
    )?;
    let hex = package
        .fingerprint
        .strip_prefix("sha256:")
        .context("Managed package fingerprint is malformed")?;
    let destination = packages_root.join(hex);
    if destination.exists() {
        let existing = read_manifest(&destination)?;
        ensure!(
            existing.fingerprint == package.fingerprint,
            "A different managed package already occupies this content identity."
        );
        verify_functional(&destination, &existing, project).await?;
    } else {
        let staging_path = staging.keep();
        fs::rename(&staging_path, &destination).with_context(|| {
            format!(
                "Could not atomically publish managed runtime package {}",
                destination.display()
            )
        })?;
    }
    let package_relative = portable(
        destination
            .strip_prefix(&project_root)
            .context("Managed package escaped the project")?,
    )?;
    Ok(PublishedNomosPackage {
        runtime_location: format!("{package_relative}/{}", package.workspace),
        executable: format!("{package_relative}/{}", package.executable),
        package,
    })
}

pub(super) fn resolve(
    project_root: &Path,
    binding: &ScientificBinding,
    deep: bool,
) -> anyhow::Result<ResolvedNomosPackage> {
    ensure!(
        binding.runtime.kind == RuntimeKind::Managed,
        "Runtime binding is not a managed package."
    );
    let root = canonical_plain(project_root)?;
    let runtime_relative = Path::new(&binding.runtime.location);
    let package_relative = runtime_relative
        .parent()
        .context("Managed runtime location has no package root")?;
    let package_root = canonical_plain(&root.join(package_relative))?;
    ensure!(
        package_root.starts_with(&root),
        "Managed runtime package escapes the project."
    );
    let package = read_manifest(&package_root)?;
    let expected = binding
        .runtime
        .package
        .as_ref()
        .context("Managed runtime binding has no package identity")?;
    ensure!(
        &package.identity() == expected
            && package.adapter == binding.adapter
            && package.project_snapshot == binding.runtime.project_snapshot,
        "Managed runtime package identity differs from its scientific binding."
    );
    let hex = package
        .fingerprint
        .strip_prefix("sha256:")
        .context("Managed package fingerprint is malformed")?;
    ensure!(
        package_root.file_name().and_then(|value| value.to_str()) == Some(hex),
        "Managed runtime package directory does not match its content identity."
    );
    let package_portable = portable(package_relative)?;
    ensure!(
        binding.runtime.location == format!("{package_portable}/{}", package.workspace)
            && binding.runtime.executable.as_deref()
                == Some(format!("{package_portable}/{}", package.executable).as_str()),
        "Managed runtime paths differ from the package manifest."
    );
    if deep {
        verify_inventory(&package_root, &package.runtime)?;
        verify_inventory(&package_root, &package.python)?;
    }
    let runtime = canonical_plain(&package_root.join(path_from_portable(&package.workspace)))?;
    let executable = canonical_plain(&package_root.join(path_from_portable(&package.executable)))?;
    ensure!(
        runtime.starts_with(&package_root)
            && runtime.is_dir()
            && executable.starts_with(&package_root)
            && executable.is_file(),
        "Managed runtime package is incomplete."
    );
    Ok(ResolvedNomosPackage {
        runtime,
        executable,
    })
}

async fn verify_functional(
    package_root: &Path,
    package: &ManagedRuntimePackage,
    project: &ExternalProjectSnapshot,
) -> anyhow::Result<()> {
    let runtime = package_root.join(path_from_portable(&package.workspace));
    let executable = package_root.join(path_from_portable(&package.executable));
    let inspection = inspect_python_runtime(&executable, &runtime).await?;
    ensure!(
        inspection.ready,
        "Existing managed Python package is incomplete."
    );
    if package.python.files.len() > 1 {
        verify_python_containment(&executable, &runtime, package_root).await?;
    }
    NomosBackend::open(runtime, executable)?
        .with_baseline_model(project.baseline_model.clone())?
        .verify_current_snapshot(project.clone())
        .await?;
    Ok(())
}

#[derive(serde::Deserialize)]
struct PythonContainmentReport {
    paths: Vec<String>,
    modules: BTreeMap<String, Vec<String>>,
}

async fn verify_python_containment(
    executable: &Path,
    runtime: &Path,
    package_root: &Path,
) -> anyhow::Result<()> {
    const SCRIPT: &str = r#"import importlib.util,json,os,sys
names=['torch','sentence_transformers','transformers','datasets','accelerate','numpy','sklearn','psutil','onnxruntime_genai']
mods={}
for name in names:
 spec=importlib.util.find_spec(name); values=[]
 if spec is not None:
  if spec.origin not in (None,'built-in','frozen'): values.append(spec.origin)
  if spec.submodule_search_locations is not None: values.extend(list(spec.submodule_search_locations))
 mods[name]=values
print(json.dumps({'paths':sys.path,'modules':mods}))"#;
    let output = tokio::time::timeout(
        Duration::from_secs(15),
        tokio::process::Command::new(executable)
            .args(["-B", "-c", SCRIPT])
            .current_dir(runtime)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("The packaged Python path check exceeded 15 seconds."))?
    .map_err(|error| anyhow::anyhow!("Could not start the packaged Python runtime: {error}"))?;
    ensure!(
        output.status.success(),
        "The packaged Python path check failed: {}",
        super::plain_error(&String::from_utf8_lossy(&output.stderr))
    );
    let report: PythonContainmentReport = serde_json::from_slice(&output.stdout)
        .context("The packaged Python path check returned unreadable output")?;
    let root = canonical_plain(package_root)?;
    for value in report
        .paths
        .iter()
        .chain(report.modules.values().flatten())
        .filter(|value| !value.trim().is_empty())
    {
        let path = Path::new(value);
        if !path.is_absolute() || !path.exists() {
            continue;
        }
        let resolved = canonical_plain(path)?;
        ensure!(
            resolved.starts_with(&root),
            "Packaged Python still resolves an external dependency: {}",
            path.display()
        );
    }
    Ok(())
}

fn plan(runtime: &Path, python: &PythonPackageSource) -> anyhow::Result<PackagePlan> {
    let runtime = canonical_plain(runtime)?;
    let mut runtime_files = BTreeMap::new();
    walk(
        &runtime,
        &runtime,
        "workspace",
        WalkPolicy::Runtime,
        &mut runtime_files,
    )?;
    let mut python_files = BTreeMap::new();
    match (&python.base_root, &python.base_executable) {
        (Some(base), Some(executable)) => {
            let exclude_site = python
                .environment_root
                .as_ref()
                .is_some_and(|environment| environment != base);
            walk(
                base,
                base,
                "python/runtime",
                WalkPolicy::PythonBase { exclude_site },
                &mut python_files,
            )?;
            if exclude_site {
                let environment = python.environment_root.as_ref().expect("checked");
                for site in &python.site_roots {
                    let relative = site
                        .strip_prefix(environment)
                        .context("Python site-packages escaped the selected environment")?;
                    let target = format!("python/runtime/{}", portable(relative)?);
                    walk(
                        site,
                        site,
                        &target,
                        WalkPolicy::PythonOverlay,
                        &mut python_files,
                    )?;
                }
            }
            ensure!(
                executable.starts_with(base),
                "Python base executable escaped its runtime root."
            );
        }
        (None, None) => {
            let name = python
                .selected_executable
                .file_name()
                .and_then(|value| value.to_str())
                .context("Selected executable name is not UTF-8")?;
            insert_file(
                &python.selected_executable,
                &format!("python/runtime/{name}"),
                &mut python_files,
            )?;
        }
        _ => anyhow::bail!("Python package source is incomplete."),
    }
    ensure!(!runtime_files.is_empty(), "Nomos runtime package is empty.");
    ensure!(!python_files.is_empty(), "Python runtime package is empty.");
    Ok(PackagePlan {
        runtime: runtime_files.into_values().collect(),
        python: python_files.into_values().collect(),
    })
}

fn packaged_executable(source: &PythonPackageSource) -> anyhow::Result<String> {
    if let (Some(base), Some(executable)) = (&source.base_root, &source.base_executable) {
        return Ok(format!(
            "python/runtime/{}",
            portable(
                executable
                    .strip_prefix(base)
                    .context("Python base executable escaped its runtime root")?
            )?
        ));
    }
    let name = source
        .selected_executable
        .file_name()
        .and_then(|value| value.to_str())
        .context("Selected executable name is not UTF-8")?;
    Ok(format!("python/runtime/{name}"))
}

#[derive(Clone, Copy)]
enum WalkPolicy {
    Runtime,
    PythonBase { exclude_site: bool },
    PythonOverlay,
}

fn walk(
    root: &Path,
    directory: &Path,
    target_root: &str,
    policy: WalkPolicy,
    files: &mut BTreeMap<String, PlannedFile>,
) -> anyhow::Result<()> {
    let mut entries = fs::read_dir(directory)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let metadata = plain(&path)?;
        let relative = path.strip_prefix(root)?;
        if skip(relative, &metadata, policy) {
            continue;
        }
        if metadata.is_dir() {
            walk(root, &path, target_root, policy, files)?;
        } else {
            let suffix = portable(relative)?;
            let target = if suffix.is_empty() {
                target_root.to_owned()
            } else {
                format!("{target_root}/{suffix}")
            };
            insert_file(&path, &target, files)?;
        }
    }
    Ok(())
}

fn skip(relative: &Path, metadata: &fs::Metadata, policy: WalkPolicy) -> bool {
    let components = relative
        .iter()
        .filter_map(|value| value.to_str())
        .collect::<Vec<_>>();
    let name = components.last().copied().unwrap_or_default();
    let cache = components.iter().any(|value| {
        matches!(
            value.to_ascii_lowercase().as_str(),
            "__pycache__" | ".pytest_cache" | ".ruff_cache" | ".mypy_cache"
        )
    });
    if cache || (!metadata.is_dir() && matches_extension(name, &["pyc", "pyo"])) {
        return true;
    }
    match policy {
        WalkPolicy::Runtime => {
            components
                .first()
                .is_some_and(|value| value.eq_ignore_ascii_case(".venv"))
                || components.len() >= 2
                    && components[0].eq_ignore_ascii_case(".git")
                    && components[1].eq_ignore_ascii_case("logs")
                || components.len() == 1
                    && (name.ends_with(".sqlite")
                        || name.ends_with(".sqlite-wal")
                        || name.ends_with(".sqlite-shm")
                        || name == "native-invocations.log")
        }
        WalkPolicy::PythonBase { exclude_site } => {
            exclude_site
                && components.len() >= 2
                && components[0].eq_ignore_ascii_case("lib")
                && components[1].eq_ignore_ascii_case("site-packages")
        }
        WalkPolicy::PythonOverlay => false,
    }
}

fn matches_extension(name: &str, extensions: &[&str]) -> bool {
    Path::new(name)
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| {
            extensions
                .iter()
                .any(|item| value.eq_ignore_ascii_case(item))
        })
}

fn insert_file(
    source: &Path,
    target: &str,
    files: &mut BTreeMap<String, PlannedFile>,
) -> anyhow::Result<()> {
    project_workspace_core::validate_relative(target)?;
    let metadata = plain(source)?;
    ensure!(metadata.is_file(), "Package source is not a regular file.");
    let planned = PlannedFile {
        source: source.to_path_buf(),
        target: target.into(),
        bytes: metadata.len(),
    };
    if let Some(existing) = files.get(target) {
        ensure!(
            existing.source == planned.source && existing.bytes == planned.bytes,
            "Two package sources map to the same managed path: {target}"
        );
    } else {
        files.insert(target.into(), planned);
    }
    Ok(())
}

fn copy_files(root: &Path, files: &[PlannedFile]) -> anyhow::Result<Vec<FileIdentity>> {
    let mut identities = Vec::with_capacity(files.len());
    for file in files {
        let target = root.join(path_from_portable(&file.target));
        fs::create_dir_all(target.parent().context("Package file has no parent")?)?;
        let identity = copy_file(file, &target)?;
        if !file.target.eq_ignore_ascii_case("workspace/.git/index") {
            identities.push(identity);
        }
    }
    Ok(identities)
}

fn copy_file(file: &PlannedFile, target: &Path) -> anyhow::Result<FileIdentity> {
    ensure!(
        plain(&file.source)?.len() == file.bytes,
        "Package source changed before copying."
    );
    let mut input = File::open(&file.source)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)?;
    let mut hasher = Sha256::new();
    let mut copied = 0_u64;
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        output.write_all(&buffer[..read])?;
        hasher.update(&buffer[..read]);
        copied = copied
            .checked_add(u64::try_from(read)?)
            .context("Package size overflow")?;
    }
    output.sync_all()?;
    ensure!(
        copied == file.bytes && plain(&file.source)?.len() == file.bytes,
        "Package source changed while it was copied."
    );
    Ok(FileIdentity {
        path: file.target.clone(),
        bytes: copied,
        fingerprint: format!("sha256:{:x}", hasher.finalize()),
    })
}

fn verify_inventory(root: &Path, inventory: &RuntimePackageInventory) -> anyhow::Result<()> {
    for identity in &inventory.files {
        let path = root.join(path_from_portable(&identity.path));
        let observed = hash_file(&path, &identity.path)?;
        ensure!(
            &observed == identity,
            "Managed runtime package file changed: {}",
            identity.path
        );
    }
    Ok(())
}

fn hash_file(path: &Path, relative: &str) -> anyhow::Result<FileIdentity> {
    let metadata = plain(path)?;
    ensure!(metadata.is_file(), "Expected a package file: {relative}");
    let mut input = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        bytes = bytes
            .checked_add(u64::try_from(read)?)
            .context("Package size overflow")?;
    }
    Ok(FileIdentity {
        path: relative.into(),
        bytes,
        fingerprint: format!("sha256:{:x}", hasher.finalize()),
    })
}

fn read_manifest(root: &Path) -> anyhow::Result<ManagedRuntimePackage> {
    let path = root.join(PACKAGE_MANIFEST);
    let metadata = plain(&path)?;
    ensure!(
        metadata.is_file() && metadata.len() <= 32 * 1_048_576,
        "Managed runtime package manifest is invalid."
    );
    let package: ManagedRuntimePackage = serde_json::from_reader(File::open(&path)?)
        .context("Managed runtime package manifest is invalid")?;
    package.validate()?;
    Ok(package)
}

fn write_new(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let mut output = OpenOptions::new().write(true).create_new(true).open(path)?;
    output.write_all(bytes)?;
    output.sync_all()?;
    Ok(())
}

fn sum_bytes(files: &[PlannedFile]) -> anyhow::Result<u64> {
    files.iter().try_fold(0_u64, |sum, file| {
        sum.checked_add(file.bytes)
            .context("Managed package size overflow")
    })
}

fn canonical_plain(path: &Path) -> anyhow::Result<PathBuf> {
    let absolute = std::path::absolute(path)?;
    for ancestor in absolute.ancestors() {
        plain(ancestor)?;
    }
    Ok(fs::canonicalize(absolute)?)
}

fn plain(path: &Path) -> anyhow::Result<fs::Metadata> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("Cannot read package path {}", path.display()))?;
    ensure!(
        !metadata.file_type().is_symlink(),
        "Runtime packages cannot contain symbolic links: {}",
        path.display()
    );
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(
            metadata.file_attributes() & 0x400 == 0,
            "Runtime packages cannot contain reparse points: {}",
            path.display()
        );
    }
    ensure!(
        metadata.is_file() || metadata.is_dir(),
        "Unsupported package file type: {}",
        path.display()
    );
    Ok(metadata)
}

fn portable(path: &Path) -> anyhow::Result<String> {
    let value = path
        .to_str()
        .context("Managed runtime path is not UTF-8")?
        .replace('\\', "/");
    if value.is_empty() {
        return Ok(value);
    }
    project_workspace_core::validate_relative(&value)?;
    Ok(value)
}

fn path_from_portable(value: &str) -> PathBuf {
    value.split('/').collect::<PathBuf>()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn runtime_plan_omits_transient_state_but_keeps_execution_inputs() {
        let temp = tempfile::tempdir().unwrap();
        let runtime = temp.path().join("source");
        fs::create_dir_all(runtime.join("nomos/__pycache__")).unwrap();
        fs::create_dir_all(runtime.join("data")).unwrap();
        fs::write(runtime.join("nomos/code.py"), "source").unwrap();
        fs::write(runtime.join("nomos/__pycache__/code.pyc"), "cache").unwrap();
        fs::write(runtime.join("data/train.jsonl"), "row").unwrap();
        fs::write(runtime.join("old.sqlite"), "state").unwrap();
        let executable = temp.path().join("python.exe");
        fs::write(&executable, "runtime").unwrap();
        let source = PythonPackageSource::standalone(&executable).unwrap();
        let planned = plan(&runtime, &source).unwrap();
        let names = planned
            .runtime
            .iter()
            .map(|value| value.target.as_str())
            .collect::<BTreeSet<_>>();
        assert!(names.contains("workspace/nomos/code.py"));
        assert!(names.contains("workspace/data/train.jsonl"));
        assert!(!names.iter().any(|value| value.contains("__pycache__")));
        assert!(!names.contains("workspace/old.sqlite"));
    }
}
