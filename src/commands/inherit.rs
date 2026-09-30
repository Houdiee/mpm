use super::*;


/// Put what is installed under management, merging with what the manifest says.
pub fn inherit(ctx: &Ctx, requested: &[String], layer: &Layer) -> Result<()> {
    // Every manager present, not only managed ones: inheriting is how a manager
    // becomes managed.
    let ids: Vec<String> = if requested.is_empty() {
        manager::ALL.iter().copied().filter(|id| present_by_id(id)).map(str::to_string).collect()
    } else {
        dedup(requested).into_iter().map(|id| require_present(&id).map(|_| id)).collect::<Result<_>>()?
    };

    if ids.is_empty() {
        println!("none of the supported package managers were found on this machine");
        return Ok(());
    }

    for id in ids {
        let found = require(&id)?;
        let path = ctx.layout.path_for(layer, &id);
        let resolved = ctx.layout.resolve(&id, &ctx.host)?;
        let installed = installed_map(found.as_ref())
            .with_context(|| format!("could not list installed packages for `{id}`"))?;

        let mut manifest = ManifestFile::load(&path)?;
        let existed = manifest.exists();
        let mut added = Vec::new();

        for name in installed.keys() {
            // Declared by any layer already: adding it here would duplicate it.
            if resolved.declared.contains_key(name) {
                continue;
            }
            // Without a version on purpose: pinning everything on inheriting would
            // freeze the whole system at today's versions.
            if manifest.declare(&PackageSpec::new(name)) {
                added.push(name.clone());
            }
        }

        if !added.is_empty() || !existed {
            manifest.save()?;
        }

        match added.len() {
            0 if existed => println!("{} already up to date", report::bold(&id)),
            0 => println!("{} now managed at {}", report::bold(&id), path.display()),
            count => {
                println!("{} +{count} into {}", report::bold(&id), path.display());
                for name in &added {
                    println!("  {name}");
                }
            }
        }
    }
    Ok(())
}

