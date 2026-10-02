use super::apply::run_in_parallel;
use super::*;

/// Bring declared packages up to date, leaving pinned ones exactly where they are.
///
/// The one upgrade only mpm can run: no package manager knows which of its
/// packages your manifests pin.
pub fn upgrade(ctx: &Ctx, requested: &[String], opts: &ApplyOpts) -> Result<()> {
    let ids = selected(ctx, requested)?;
    if ids.is_empty() {
        println!("{}", nothing_managed(ctx));
        return Ok(());
    }

    let mut queued = Vec::new();
    let mut held = Vec::new();
    let mut fully_pinned = Vec::new();
    let mut not_upgradable = Vec::new();

    for id in ids {
        let found = require(&id)?;
        let declared = ctx.layout.resolve(&id, &ctx.host)?.declared;

        let mut pinned = Vec::new();
        let mut unpinned = Vec::new();
        for spec in declared.values() {
            if spec.version.is_some() {
                pinned.push(spec.name.clone());
            } else {
                unpinned.push(spec.name.clone());
            }
        }

        let commands = found.upgrade_commands(&unpinned, &pinned);
        if commands.is_empty() {
            if declared.is_empty() {
                // Nothing declared; `status` is where that gets reported.
            } else if unpinned.is_empty() {
                fully_pinned.push(id);
            } else {
                not_upgradable.push(id);
            }
            continue;
        }
        if !pinned.is_empty() {
            held.push((id.clone(), pinned));
        }
        queued.push((id, commands));
    }

    if queued.is_empty() {
        explain_nothing_to_do(&fully_pinned, &not_upgradable);
        return Ok(());
    }

    for (id, commands) in &queued {
        println!("{}", report::bold(id));
        for command in commands {
            println!("  {} {}", report::dim("$"), command.display());
        }
    }
    println!();

    if !held.is_empty() {
        let total: usize = held.iter().map(|(_, names)| names.len()).sum();
        println!(
            "{} {total} pinned package(s) left alone:",
            report::bold("note:")
        );
        for (id, names) in &held {
            println!(
                "  {} {}",
                report::dim(&format!("{id}:")),
                report::dim(&names.join(" "))
            );
        }
        println!();
    }

    if opts.dry_run {
        println!("{}", report::dim("dry run: nothing was changed"));
        return Ok(());
    }
    if !opts.yes && !report::confirm("Upgrade?")? {
        println!("{}", report::dim("aborted"));
        return Ok(());
    }

    run_in_parallel(queued)
}

/// Nothing to upgrade because everything is pinned is a success; nothing to
/// upgrade because mpm cannot drive this manager is a dead end. Say which.
fn explain_nothing_to_do(fully_pinned: &[String], not_upgradable: &[String]) {
    if fully_pinned.is_empty() && not_upgradable.is_empty() {
        println!("{}", report::dim("nothing is declared for those managers"));
        return;
    }

    for id in fully_pinned {
        println!(
            "{} {}",
            report::bold(id),
            report::dim("every declared package is pinned; nothing to upgrade")
        );
    }
    for id in not_upgradable {
        println!(
            "{} {}",
            report::bold(id),
            report::dim("mpm has no upgrade for this manager")
        );
    }
}
