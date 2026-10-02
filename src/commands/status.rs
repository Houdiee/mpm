use super::*;

/// Returns true when the machine does not match its manifests.
pub fn status(ctx: &Ctx, requested: &[String]) -> Result<bool> {
    let ids = selected(ctx, requested)?;
    if ids.is_empty() {
        println!("{}", nothing_managed(ctx));
        return Ok(false);
    }

    let mut results = Vec::new();
    let mut failed = false;

    for (id, result) in reconcile_all(ctx, &ids) {
        match result {
            Ok(changes) => {
                let text = report::render(&changes);
                if !text.is_empty() {
                    println!("{text}");
                }
                results.push(changes);
            }
            Err(error) => {
                eprintln!("{}: {error:#}", report::bold(&id));
                failed = true;
            }
        }
    }

    // Never claim the machine is clean while a manager could not be read: an
    // unreadable manager is unknown state, not matching state.
    if failed {
        let unreadable = ids.len() - results.len();
        let rest = if results.is_empty() {
            String::new()
        } else {
            format!("; the rest: {}", report::summarize(&results))
        };
        println!(
            "{}",
            report::problem(&format!("{unreadable} manager(s) could not be read{rest}"))
        );
    } else {
        println!("{}", report::summarize(&results));
    }

    Ok(results.iter().any(Reconciliation::has_work) || failed)
}
