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
        results.push(result.with_context(|| format!("refusing to apply: `{id}` could not be inspected"))?);
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

    // A version mpm cannot obtain must stop the run before the prompt, not half
    // way through it.
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

    run_in_parallel(queued)
}

/// Managers share nothing, so `npm` never waits on `pacman`. Workers capture
/// their output rather than writing to the terminal, because concurrent package
/// managers would otherwise interleave into noise.
fn run_in_parallel(queued: Vec<(String, Vec<Invocation>)>) -> Result<()> {
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
            let _ = completions.send(Completion { manager: id, transcript, result });
        });
    }
    drop(completions);

    let mut failures = Vec::new();
    for completion in finished {
        match completion.result {
            Ok(()) => {
                println!("{} {}", report::bold(&completion.manager), report::dim("done"));
                print!("{}", report::dim(&completion.transcript));
            }
            Err(error) => {
                println!("{} {}", report::bold(&completion.manager), report::problem("failed"));
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
    let needs_root =
        queued.iter().flat_map(|(_, commands)| commands).any(|command| command.needs_root);
    if !needs_root {
        return Ok(());
    }
    Invocation::new("true").as_root().run().context("could not obtain root")
}

fn indent(output: &str) -> String {
    output.lines().map(|line| format!("    {line}\n")).collect()
}

/// Separated from execution so a version mpm cannot obtain is discovered
/// *before* the prompt, not half way through a run that has changed the system.
fn commands_for(changes: &Reconciliation) -> Result<Vec<Invocation>> {
    let manager = require(&changes.manager)?;

    // Install first: a package moving between managers is never briefly absent,
    // and an interrupted run leaves more installed rather than less.
    let mut commands = manager.install(&changes.to_install())?;
    commands.extend(manager.uninstall(&changes.remove));
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
