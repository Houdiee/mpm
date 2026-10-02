use super::*;

/// Report packages a manifest declares that have a newer version available.
///
/// Exits zero either way: an entry without a version means "any version", so a
/// newer one existing is not a fault. This is for deciding whether to bump a pin.
pub fn outdated(ctx: &Ctx, requested: &[String]) -> Result<()> {
    let ids = selected(ctx, requested)?;
    if ids.is_empty() {
        println!("{}", nothing_managed(ctx));
        return Ok(());
    }

    let mut asked = 0;
    let mut pinned_behind = 0;

    for id in ids {
        let found = require(&id)?;
        let Some(command) = found.outdated_command() else {
            continue;
        };
        asked += 1;

        // Most of these exit non-zero when something is upgradable, so the exit
        // status says nothing useful; the output does.
        let (text, _) = command.run_captured();
        let available = found.parse_outdated(&text);
        if available.is_empty() {
            continue;
        }

        let declared = ctx.layout.resolve(&id, &ctx.host)?.declared;
        let mut lines = Vec::new();

        for update in &available {
            let Some(wanted) = declared.get(&update.name) else {
                continue;
            };
            match &wanted.version {
                Some(pinned) if Some(pinned) != update.version.as_ref() => {
                    pinned_behind += 1;
                    lines.push(format!(
                        "  {} {} {} {}",
                        report::bold(&update.name),
                        report::dim(pinned),
                        report::dim("->"),
                        update.version.as_deref().unwrap_or("?")
                    ));
                }
                Some(_) => {}
                None => lines.push(format!(
                    "  {} {}",
                    update.name,
                    report::dim(&format!(
                        "-> {} (unpinned; an ordinary upgrade picks this up)",
                        update.version.as_deref().unwrap_or("?")
                    ))
                )),
            }
        }

        if !lines.is_empty() {
            println!("{}", report::bold(&id));
            for line in lines {
                println!("{line}");
            }
            println!();
        }
    }

    if asked == 0 {
        println!(
            "{}",
            report::dim("none of those managers can report what is outdated")
        );
    } else if pinned_behind == 0 {
        println!("{}", report::dim("no pinned package is behind"));
    } else {
        println!(
            "{pinned_behind} pinned package(s) behind. {}",
            report::dim("`mpm upgrade --pins` bumps them")
        );
    }
    Ok(())
}
