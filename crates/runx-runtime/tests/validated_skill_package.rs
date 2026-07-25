use std::fs;
use std::path::PathBuf;

use runx_runtime::load_validated_skill_package;

#[test]
fn validated_skill_package_loads_one_digest_bound_aggregate()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    fs::write(
        temp.path().join("SKILL.md"),
        "---\nname: aggregate\ndescription: one package truth\n---\n\n# Aggregate\n",
    )?;
    fs::write(
        temp.path().join("X.yaml"),
        "skill: aggregate\nrunners:\n  inspect:\n    type: agent\n",
    )?;

    let loaded = load_validated_skill_package(temp.path())?;

    assert_eq!(loaded.package.skill.name, "aggregate");
    assert_eq!(loaded.package.manual_markdown.lines().next(), Some("---"));
    assert!(loaded.package.manual_digest.starts_with("sha256:"));
    assert_eq!(loaded.package.source.files.len(), 2);
    Ok(())
}

#[cfg(unix)]
#[test]
fn validated_skill_package_rejects_symlinked_sources() -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir()?;
    fs::write(
        temp.path().join("SKILL.md"),
        "---\nname: aggregate\n---\n\n# Aggregate\n",
    )?;
    let outside = tempfile::NamedTempFile::new()?;
    symlink(outside.path(), temp.path().join("module.mjs"))?;

    let error = load_validated_skill_package(temp.path())
        .err()
        .ok_or("symlinked package unexpectedly loaded")?;

    assert!(error.to_string().contains("symbolic links"));
    Ok(())
}

#[test]
fn validated_skill_package_resolves_internal_profile_to_owning_manual()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let profile_dir = temp.path().join("graph/plan");
    fs::create_dir_all(&profile_dir)?;
    let manual = "---\nname: operator\ndescription: full operator context\n---\n\n# Operator\n\nPreserve this complete manual.\n";
    fs::write(temp.path().join("SKILL.md"), manual)?;
    fs::write(
        profile_dir.join("X.yaml"),
        "runners:\n  plan:\n    type: javascript\n    module: plan.mjs\n",
    )?;
    fs::write(
        profile_dir.join("plan.mjs"),
        "export default (inputs) => inputs;\n",
    )?;

    let loaded = load_validated_skill_package(&profile_dir)?;

    // Package roots are canonical; macOS tempdirs resolve through /private.
    assert_eq!(loaded.package_root, temp.path().canonicalize()?);
    assert_eq!(loaded.profile_path.as_deref(), Some("graph/plan/X.yaml"));
    assert!(loaded.manifest().is_some());
    assert_eq!(loaded.package.manual_markdown, manual);
    Ok(())
}

#[test]
fn official_skill_packages_validate_through_the_aggregate() -> Result<(), Box<dyn std::error::Error>>
{
    let skills_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("skills");
    let mut failures = Vec::new();
    let mut directories = fs::read_dir(&skills_root)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir() && path.join("SKILL.md").is_file())
        .collect::<Vec<_>>();
    directories.sort();

    for directory in directories {
        if let Err(error) = load_validated_skill_package(&directory) {
            let name = directory
                .file_name()
                .map(|value| value.to_string_lossy())
                .unwrap_or_default();
            failures.push(format!("{name}: {error}"));
        }
    }

    assert!(
        failures.is_empty(),
        "official skill packages must share one aggregate contract:\n{}",
        failures.join("\n")
    );
    Ok(())
}
