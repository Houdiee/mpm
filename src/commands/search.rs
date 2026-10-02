use super::*;

/// Run each manager's own search and show what it printed.
///
/// Nothing is parsed: a search result is prose. What mpm adds is the question
/// only it can answer -- of the managers you have, which ones have this?
pub fn search(_ctx: &Ctx, requested: &[String], query: &str) -> Result<()> {
    // Every manager present, not only managed ones: searching is how you decide
    // where to install something from, which may be a manager you do not yet use.
    let ids: Vec<String> = if requested.is_empty() {
        manager::ALL
            .iter()
            .copied()
            .filter(|id| present_by_id(id))
            .map(str::to_string)
            .collect()
    } else {
        dedup(requested)
            .into_iter()
            .map(|id| require_present(&id).map(|_| id))
            .collect::<Result<_>>()?
    };

    if ids.is_empty() {
        println!("none of the supported package managers were found on this machine");
        return Ok(());
    }

    let mut searched = 0;
    for id in ids {
        let found = require(&id)?;
        let Some(command) = found.search_command(query) else {
            continue;
        };

        let (text, result) = command.run_captured();
        println!("{}", report::bold(&id));
        if let Err(error) = result {
            // A manager with nothing to say exits non-zero on most of these, so
            // this is a note rather than a failure of the whole search.
            println!("  {}", report::dim(&format!("{error:#}")));
        }
        for line in text.lines() {
            println!("  {line}");
        }
        println!();
        searched += 1;
    }

    if searched == 0 {
        println!(
            "{}",
            report::dim("none of those managers has a search of its own")
        );
    }
    Ok(())
}
