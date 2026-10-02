use super::*;

/// Declare packages. Installing them is `mpm apply`'s job.
///
/// With `pin`, each is recorded at the version installed right now. Nothing
/// else in mpm writes a version into a manifest.
pub fn add(ctx: &Ctx, ids: &[String], packages: &[String], layer: &Layer, pin: bool) -> Result<()> {
    for id in dedup(ids) {
        add_one(ctx, &id, packages, layer, pin)?;
    }
    Ok(())
}

fn add_one(ctx: &Ctx, id: &str, packages: &[String], layer: &Layer, pin: bool) -> Result<()> {
    let manager = require(id)?;
    let mut specs = parse_specs(packages)?;

    if pin {
        specs = pin_to_installed(manager.as_ref(), &specs)?;
    }

    let path = ctx.layout.path_for(layer, id);
    let added = edit_manifest(&path, &specs, Edit::Declare)?;

    if added.is_empty() {
        println!("nothing to add; already declared");
        return Ok(());
    }
    println!("declared in {}", path.display());
    for spec in &added {
        println!("  + {spec}");
    }
    println!("{}", report::dim("run `mpm apply` to install"));
    Ok(())
}

fn pin_to_installed(manager: &dyn Manager, specs: &[PackageSpec]) -> Result<Vec<PackageSpec>> {
    let id = manager.id();

    if !manager.supports_pinning() {
        bail!("`{id}` cannot install a specific version, so there is nothing to pin");
    }
    if !manager::present(manager) {
        bail!("`{id}` is not installed on this machine, so no version can be read");
    }
    if let Some(spec) = specs.iter().find(|spec| spec.version.is_some()) {
        bail!("`{spec}` already names a version -- drop `--pin`, or drop the version");
    }

    let installed = installed_map(manager)
        .with_context(|| format!("could not list installed packages for `{id}`"))?;

    specs
        .iter()
        .map(|spec| {
            let Some(found) = installed.get(&spec.name) else {
                bail!(
                    "`{}` is not installed, so there is no version to pin",
                    spec.name
                );
            };
            let Some(version) = &found.version else {
                bail!("`{id}` does not report a version for `{}`", spec.name);
            };
            Ok(PackageSpec::pinned(&spec.name, version))
        })
        .collect()
}

/// The same operation as `add --pin`, reachable on its own so pinning is also
/// something you do *to* a package you already have.
pub fn pin(ctx: &Ctx, ids: &[String], packages: &[String], layer: &Layer) -> Result<()> {
    add(ctx, ids, packages, layer, true)
}

pub fn unpin(ctx: &Ctx, ids: &[String], packages: &[String], layer: &Layer) -> Result<()> {
    for id in dedup(ids) {
        unpin_one(ctx, &id, packages, layer)?;
    }
    Ok(())
}

fn unpin_one(ctx: &Ctx, id: &str, packages: &[String], layer: &Layer) -> Result<()> {
    require(id)?;
    let specs = parse_specs(packages)?;
    let path = ctx.layout.path_for(layer, id);
    let mut manifest = ManifestFile::load(&path)?;
    let mut changed = Vec::new();

    for spec in &specs {
        // Never introduces a package: unpinning an undeclared one would declare it.
        if !manifest.declares(&spec.name) {
            println!(
                "{} `{}` is not declared in {}",
                report::bold("note:"),
                spec.name,
                path.display()
            );
            continue;
        }
        if manifest.declare(&PackageSpec::new(&spec.name)) {
            changed.push(spec.name.clone());
        }
    }

    if changed.is_empty() {
        println!("nothing to unpin");
        return Ok(());
    }

    manifest.save()?;
    println!("unpinned in {}", path.display());
    for name in &changed {
        println!("  {name}");
    }
    Ok(())
}

pub fn remove(ctx: &Ctx, ids: &[String], packages: &[String], layer: &Layer) -> Result<()> {
    for id in dedup(ids) {
        remove_one(ctx, &id, packages, layer)?;
    }
    Ok(())
}

fn remove_one(ctx: &Ctx, id: &str, packages: &[String], layer: &Layer) -> Result<()> {
    let found = require_present(id)?;

    let specs = parse_specs(packages)?;
    let path = ctx.layout.path_for(layer, id);
    let names: Vec<String> = edit_manifest(&path, &specs, Edit::Withdraw)?
        .iter()
        .map(|spec| spec.name.clone())
        .collect();

    // Uninstall everything named, whether or not this layer declared it.
    let requested: Vec<String> = specs.iter().map(|spec| spec.name.clone()).collect();
    for command in found.uninstall_commands(&requested) {
        run_visibly(&command)?;
    }

    if names.is_empty() {
        println!(
            "{}",
            report::dim("was not declared in this layer; nothing changed in your manifest")
        );
    } else {
        println!("undeclared in {}", path.display());
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Edit {
    Declare,
    Withdraw,
}

fn parse_specs(packages: &[String]) -> Result<Vec<PackageSpec>> {
    if packages.is_empty() {
        bail!("name at least one package");
    }
    packages
        .iter()
        .map(|text| PackageSpec::parse(text))
        .collect::<Result<Vec<PackageSpec>>>()
        .context("a package is written `<name>` or, quoted, `<name> <version>`")
}

/// Apply an edit to one layer, returning the entries that actually changed.
fn edit_manifest(path: &Path, specs: &[PackageSpec], edit: Edit) -> Result<Vec<PackageSpec>> {
    let mut manifest = ManifestFile::load(path)?;
    let mut changed = Vec::new();

    for spec in specs {
        let touched = match edit {
            Edit::Declare => manifest.declare(spec),
            Edit::Withdraw => manifest.undeclare(&spec.name),
        };
        if touched {
            changed.push(spec.clone());
        }
    }

    if !changed.is_empty() {
        manifest.save()?;
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::fixture::*;
    use super::*;
    use std::fs;

    #[test]
    fn declaring_appends_and_is_idempotent() {
        let dir = TempDir::new("declare");
        let path = dir.0.join("cargo");
        seed(&path, "# tools\nvim\n");

        let specs = [PackageSpec::new("bat"), PackageSpec::new("vim")];
        let added = edit_manifest(&path, &specs, Edit::Declare).expect("edit");
        assert_eq!(
            added,
            vec![PackageSpec::new("bat")],
            "vim was already declared"
        );
        assert_eq!(
            fs::read_to_string(&path).expect("read"),
            "# tools\nvim\nbat\n"
        );

        let again = edit_manifest(&path, &[PackageSpec::new("bat")], Edit::Declare).expect("edit");
        assert!(again.is_empty());
    }

    #[test]
    fn withdrawing_removes_only_the_named_entry() {
        let dir = TempDir::new("withdrawing");
        let path = dir.0.join("cargo");
        seed(&path, "# tools\nvim\nbat\n");

        let removed =
            edit_manifest(&path, &[PackageSpec::new("bat")], Edit::Withdraw).expect("edit");
        assert_eq!(removed, vec![PackageSpec::new("bat")]);
        assert_eq!(fs::read_to_string(&path).expect("read"), "# tools\nvim\n");
    }

    #[test]
    fn a_malformed_package_argument_is_rejected() {
        let error = parse_specs(&["ripgrep 14.1.0 oops".into()]).expect_err("must reject");
        assert!(format!("{error:#}").contains("<name> <version>"));
    }

    #[test]
    fn pinning_records_the_installed_version() {
        let fake = Fake {
            installed: "ripgrep 14.0.0-1\nbat 0.24.0-1\n",
            pinning: true,
        };
        let pinned = pin_to_installed(&fake, &[PackageSpec::new("ripgrep")]).expect("pins");
        assert_eq!(pinned, vec![PackageSpec::pinned("ripgrep", "14.0.0-1")]);
    }

    #[test]
    fn pinning_what_is_not_installed_is_an_error() {
        let fake = Fake {
            installed: "ripgrep 14.0.0-1\n",
            pinning: true,
        };
        let error = pin_to_installed(&fake, &[PackageSpec::new("nano")]).expect_err("must reject");
        assert!(error.to_string().contains("not installed"));
    }

    #[test]
    fn pinning_a_manager_that_cannot_pin_is_an_error() {
        let fake = Fake {
            installed: "ripgrep 14.0.0-1\n",
            pinning: false,
        };
        let error =
            pin_to_installed(&fake, &[PackageSpec::new("ripgrep")]).expect_err("must reject");
        assert!(error.to_string().contains("nothing to pin"));
    }

    #[test]
    fn pinning_something_already_versioned_is_a_contradiction() {
        let fake = Fake {
            installed: "ripgrep 14.0.0-1\n",
            pinning: true,
        };
        let error = pin_to_installed(&fake, &[PackageSpec::pinned("ripgrep", "13.0.0-1")])
            .expect_err("must reject");
        assert!(error.to_string().contains("already names a version"));
    }
}
