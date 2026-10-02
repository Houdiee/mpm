use super::*;

pub fn apply(ctx: &Ctx, requested: &[String], opts: &ApplyOpts) -> Result<()> {
    let ids = selected(ctx, requested)?;
    if ids.is_empty() {
        println!("{}", nothing_managed(ctx));
        return Ok(());
    }

    // Acting on a half-known system is how the wrong things get uninstalled.
    let mut results = Vec::new();
    for (id, result) in reconcile_all(ctx, &ids) {
        results.push(
            result.with_context(|| format!("refusing to apply: `{id}` could not be inspected"))?,
        );
    }

    for changes in &results {
        let text = report::render(changes);
        if !text.is_empty() {
            println!("{text}");
        }
    }

    println!("{}", report::summarize(&results));
    if !results.iter().any(Reconciliation::has_work) {
        return Ok(());
    }

    // Build every command before prompting, so a manifest mpm cannot carry out
    // stops the run before it has changed anything.
    let mut queued = Vec::new();
    for changes in &results {
        if !changes.has_work() {
            continue;
        }
        let commands = commands_for(changes)
            .with_context(|| format!("refusing to apply: `{}`", changes.manager))?;
        queued.push((changes.manager.clone(), commands));
    }

    if opts.dry_run {
        println!("{}", report::dim("dry run: nothing was changed"));
        return Ok(());
    }

    if !opts.yes && !report::confirm("Apply this plan?")? {
        println!("{}", report::dim("aborted"));
        return Ok(());
    }

    let touched: Vec<String> = queued.iter().map(|(id, _)| id.clone()).collect();
    run_in_parallel(queued)?;

    // A command can succeed and change nothing -- a manager that silently
    // declines to downgrade, a pin something else is holding. Reporting success
    // on a run that achieved nothing is the one failure mode mpm must not have.
    report_what_did_not_take(ctx, &touched)
}

/// Re-read installed state and fail if anything the run was supposed to change
/// did not change.
fn report_what_did_not_take(ctx: &Ctx, ids: &[String]) -> Result<()> {
    let mut stuck = Vec::new();

    for (id, result) in reconcile_all(ctx, ids) {
        // A manager that cannot be re-read was already reported by its worker.
        let Ok(changes) = result else { continue };
        if changes.has_work() {
            stuck.push((id, changes));
        }
    }

    if stuck.is_empty() {
        return Ok(());
    }

    println!();
    println!(
        "{} the following did not take effect:",
        report::problem("warning:")
    );
    for (id, changes) in &stuck {
        println!("  {}", report::bold(id));
        for spec in &changes.install {
            println!("    {} still not installed", spec.name);
        }
        for change in &changes.repin {
            println!(
                "    {} is still at {}, not {}",
                change.spec.name,
                change.installed,
                change.spec.version.as_deref().unwrap_or("?")
            );
        }
        for name in &changes.remove {
            println!("    {name} is still installed");
        }
    }
    // Most pinning managers give each package its own dependencies, so a version
    // that will not move usually does not exist. composer is the exception: its
    // global requires share one tree, so something else may be holding it.
    println!(
        "{}",
        report::dim(
            "Check that the declared version exists; `mpm status` will keep reporting this."
        )
    );
    bail!(
        "applied, but {} manager(s) did not reach the declared state",
        stuck.len()
    )
}

/// Managers share nothing, so `npm` never waits on `pacman`. Workers capture
/// their output rather than writing to the terminal, because concurrent package
/// managers would otherwise interleave into noise.
pub(super) fn run_in_parallel(queued: Vec<(String, Vec<Invocation>)>) -> Result<()> {
    preauthorize(&queued)?;

    let (completions, finished) = mpsc::channel();
    for (id, commands) in queued {
        let completions = completions.clone();
        thread::spawn(move || {
            let mut transcript = String::new();
            let mut result = Ok(());

            for command in &commands {
                transcript.push_str(&format!("  $ {}\n", command.display()));
                let (output, outcome) = command.run_captured();
                transcript.push_str(&indent(&output));
                if let Err(error) = outcome {
                    result = Err(error);
                    break;
                }
            }
            let _ = completions.send(Completion {
                manager: id,
                transcript,
                result,
            });
        });
    }
    drop(completions);

    let mut failures = Vec::new();
    for completion in finished {
        match completion.result {
            Ok(()) => {
                println!(
                    "{} {}",
                    report::bold(&completion.manager),
                    report::dim("done")
                );
                print!("{}", report::dim(&completion.transcript));
            }
            Err(error) => {
                println!(
                    "{} {}",
                    report::bold(&completion.manager),
                    report::problem("failed")
                );
                print!("{}", completion.transcript);
                eprintln!("{}: {error:#}", report::bold(&completion.manager));
                failures.push(completion.manager);
            }
        }
    }

    if !failures.is_empty() {
        failures.sort();
        bail!("failed to apply: {}", failures.join(", "));
    }
    Ok(())
}

struct Completion {
    manager: String,
    transcript: String,
    result: Result<()>,
}

/// Ask for the elevator's password once, before any worker starts.
///
/// Workers capture their output, so an elevator prompting inside one would
/// block with nothing on screen to explain why.
fn preauthorize(queued: &[(String, Vec<Invocation>)]) -> Result<()> {
    let needs_root = queued
        .iter()
        .flat_map(|(_, commands)| commands)
        .any(|command| command.needs_root);
    if !needs_root {
        return Ok(());
    }
    Invocation::new("true")
        .with_root()
        .run()
        .context("could not obtain root")
}

fn indent(output: &str) -> String {
    output.lines().map(|line| format!("    {line}\n")).collect()
}

/// Separate from execution so a manifest mpm cannot carry out is discovered
/// before the prompt.
fn commands_for(changes: &Reconciliation) -> Result<Vec<Invocation>> {
    let manager = require(&changes.manager)?;

    // Install first: a package moving between managers is never briefly absent,
    // and an interrupted run leaves more installed rather than less.
    let mut commands = manager.install_commands(&changes.to_install());
    commands.extend(manager.uninstall_commands(&changes.remove));
    Ok(commands)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_needing_root_asks_for_nothing() {
        let queued = vec![("cargo".to_string(), vec![Invocation::new("true")])];
        preauthorize(&queued).expect("no elevation, no prompt");
    }

    #[test]
    fn transcripts_are_indented_under_their_manager() {
        assert_eq!(indent("one\ntwo\n"), "    one\n    two\n");
        assert_eq!(indent(""), "");
    }
}
