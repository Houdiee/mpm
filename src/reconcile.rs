use anyhow::{Result, bail};
use std::cmp::Ordering;
use std::collections::BTreeMap;

use crate::manager::Manager;
use crate::manifest::Resolved;
use crate::manifest::grammar::PackageSpec;

/// A declared pin that does not match the installed version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repin {
    pub spec: PackageSpec,
    pub installed: String,
}

impl Repin {
    /// `Less` means the declared version is older than the installed one.
    pub fn direction(&self) -> Option<Ordering> {
        compare_versions(self.spec.version.as_deref()?, &self.installed)
    }
}

/// Order two version strings, or admit that they cannot be ordered.
///
/// Pre-release ranking (`1.0.0-rc1` below `1.0.0`) is deliberately not modelled:
/// across sixteen managers the spellings do not agree, so anything undecidable
/// returns `None` and is reported without a direction rather than labelled wrongly.
pub fn compare_versions(left: &str, right: &str) -> Option<Ordering> {
    let left = split_version(left);
    let right = split_version(right);

    for index in 0..left.len().max(right.len()) {
        let ordering = match (left.get(index), right.get(index)) {
            (Some(a), Some(b)) => match (a.parse::<u64>(), b.parse::<u64>()) {
                (Ok(a), Ok(b)) => a.cmp(&b),
                _ if a == b => Ordering::Equal,
                _ => return None,
            },
            // A missing segment stands in as zero, so `1.2` equals `1.2.0` while
            // `1.2.1` is above both.
            (Some(a), None) => match a.parse::<u64>() {
                Ok(0) => Ordering::Equal,
                Ok(_) => return Some(Ordering::Greater),
                Err(_) => return None,
            },
            (None, Some(b)) => match b.parse::<u64>() {
                Ok(0) => Ordering::Equal,
                Ok(_) => return Some(Ordering::Less),
                Err(_) => return None,
            },
            (None, None) => break,
        };
        if ordering != Ordering::Equal {
            return Some(ordering);
        }
    }

    Some(Ordering::Equal)
}

fn split_version(version: &str) -> Vec<&str> {
    version
        .split(['.', '-', '+', '_', ':', '~'])
        .filter(|segment| !segment.is_empty())
        .collect()
}

/// The changes that would bring one manager in line with its manifest.
///
/// `status`, `apply --dry-run` and `apply` all render and execute *this* value,
/// so a preview cannot drift away from the action it previews.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reconciliation {
    pub manager: String,
    /// Declared but not installed.
    pub install: Vec<PackageSpec>,
    /// Installed at the wrong version.
    pub repin: Vec<Repin>,
    /// Installed but not declared.
    pub remove: Vec<PackageSpec>,
}

impl Reconciliation {
    pub fn has_work(&self) -> bool {
        !self.install.is_empty() || !self.repin.is_empty() || !self.remove.is_empty()
    }

    /// Packages to install, including repins.
    pub fn to_install(&self) -> Vec<PackageSpec> {
        let mut specs = self.install.clone();
        specs.extend(self.repin.iter().map(|change| change.spec.clone()));
        specs
    }
}

/// Reject a manifest whose pins this manager cannot carry out.
///
/// An error rather than a warning: installing an arbitrary version when an exact
/// one was asked for is the quiet substitution this tool exists to avoid.
pub fn validate(manager: &dyn Manager, resolved: &Resolved) -> Result<()> {
    let mut problems = Vec::new();

    for (name, specs) in &resolved.repeated {
        problems.extend(repeated_problem(manager, name, specs));
    }

    for (name, spec) in &resolved.declared {
        let Some(version) = &spec.version else {
            continue;
        };

        if manager.name_selects_version(name) {
            problems.push(format!(
                "`{name} {version}`: the name `{name}` already selects a version -- drop `{version}`"
            ));
        } else if !manager.supports_pinning() {
            problems.push(format!(
                "`{name} {version}`: {} cannot install a specific version -- declare `{name}` alone",
                manager.id()
            ));
        }
    }

    if !problems.is_empty() {
        bail!("{}", problems.join("\n  "));
    }

    Ok(())
}

/// Why one manifest file may not declare this name more than once, if it may not.
///
/// Two lines carrying the same version are always a mistake -- the second says
/// nothing the first did not. Two different versions are a real declaration only
/// where the manager keeps both *and* an exact version can be asked for;
/// anywhere else the second line would simply replace the first.
fn repeated_problem(manager: &dyn Manager, name: &str, specs: &[PackageSpec]) -> Option<String> {
    let mut versions: Vec<Option<&str>> = specs.iter().map(|s| s.version.as_deref()).collect();
    versions.sort_unstable();
    versions.dedup();

    if versions.len() < specs.len() {
        return Some(format!(
            "`{name}` is declared more than once with the same version -- remove the duplicate"
        ));
    }

    if manager.allows_multiple_versions() && manager.supports_pinning() {
        return None;
    }

    Some(format!(
        "`{name}` is declared at {} different versions, but {} keeps only one version of a package",
        specs.len(),
        manager.id()
    ))
}

/// Compare declared state against installed state.
///
/// Pure: no process is spawned and no file is read, so the safety-critical
/// decision of *what gets uninstalled* is directly testable.
pub fn compute(
    manager: &str,
    declared: &BTreeMap<String, PackageSpec>,
    installed: &BTreeMap<String, PackageSpec>,
    supports_pinning: bool,
) -> Result<Reconciliation> {
    let mut changes = Reconciliation {
        manager: manager.to_string(),
        ..Reconciliation::default()
    };
    let mut unverifiable = Vec::new();

    for (name, wanted) in declared {
        match installed.get(name) {
            None => changes.install.push(wanted.clone()),
            Some(present) => {
                // Pins here were already rejected by `validate`.
                if !supports_pinning {
                    continue;
                }
                let Some(want) = &wanted.version else {
                    continue;
                };
                // Every pinning manager reports a version for everything it lists,
                // so a missing one means the output did not parse. Calling that a
                // satisfied pin would report clean on the very drift mpm catches.
                let Some(have) = &present.version else {
                    unverifiable.push(name.clone());
                    continue;
                };
                if want != have {
                    changes.repin.push(Repin {
                        spec: wanted.clone(),
                        installed: have.clone(),
                    });
                }
            }
        }
    }

    if !unverifiable.is_empty() {
        bail!(
            "`{manager}` listed {} without a version, so the declared pin cannot be \
             verified -- its list output may have changed format",
            unverifiable.join(", ")
        );
    }

    for (name, present) in installed {
        if declared.contains_key(name) {
            continue;
        }
        // The whole spec, not just the name: a manager may need the version to
        // remove it, and the plan is clearer for showing which one goes.
        changes.remove.push(present.clone());
    }

    Ok(changes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Removal now carries whole specs; the tests care about the names.
    fn removed(changes: &Reconciliation) -> Vec<String> {
        changes
            .remove
            .iter()
            .map(|spec| spec.name.clone())
            .collect()
    }

    fn map(specs: &[PackageSpec]) -> BTreeMap<String, PackageSpec> {
        specs
            .iter()
            .map(|spec| (spec.name.clone(), spec.clone()))
            .collect()
    }

    fn changes_of(
        declared: &[PackageSpec],
        installed: &[PackageSpec],
        pinning: bool,
    ) -> Reconciliation {
        compute("test", &map(declared), &map(installed), pinning).expect("reconciles")
    }

    #[test]
    fn matching_state_is_quiet() {
        let changes = changes_of(
            &[PackageSpec::new("vim")],
            &[PackageSpec::new("vim")],
            false,
        );
        assert!(!changes.has_work());
        assert!(!changes.has_work());
    }

    #[test]
    fn declared_but_missing_is_installed() {
        let changes = changes_of(&[PackageSpec::new("vim")], &[], false);
        assert_eq!(changes.install, vec![PackageSpec::new("vim")]);
        assert!(changes.remove.is_empty());
    }

    #[test]
    fn installed_but_undeclared_is_removed() {
        let changes = changes_of(&[], &[PackageSpec::new("nano")], false);
        assert_eq!(removed(&changes), vec!["nano"]);
        assert!(changes.install.is_empty());
    }

    #[test]
    fn a_pin_mismatch_is_a_repin() {
        let changes = changes_of(
            &[PackageSpec::pinned("ripgrep", "14.1.0")],
            &[PackageSpec::pinned("ripgrep", "14.0.0")],
            true,
        );
        assert_eq!(
            changes.repin,
            vec![Repin {
                spec: PackageSpec::pinned("ripgrep", "14.1.0"),
                installed: "14.0.0".into()
            }]
        );
        assert!(changes.install.is_empty());
        assert!(changes.remove.is_empty());
    }

    #[test]
    fn a_satisfied_pin_is_no_work() {
        let changes = changes_of(
            &[PackageSpec::pinned("ripgrep", "14.1.0")],
            &[PackageSpec::pinned("ripgrep", "14.1.0")],
            true,
        );
        assert!(!changes.has_work());
    }

    #[test]
    fn an_unverifiable_pin_is_an_error_not_a_pass() {
        let error = compute(
            "test",
            &map(&[PackageSpec::pinned("ripgrep", "14.1.0")]),
            &map(&[PackageSpec::new("ripgrep")]),
            true,
        )
        .expect_err("must refuse to guess");
        assert!(error.to_string().contains("ripgrep"));
        assert!(error.to_string().contains("cannot be verified"));
    }

    #[test]
    fn an_unpinned_package_needs_no_installed_version() {
        let changes = changes_of(
            &[PackageSpec::new("ripgrep")],
            &[PackageSpec::new("ripgrep")],
            true,
        );
        assert!(!changes.has_work());
    }

    #[test]
    fn repins_are_installed_alongside_new_packages() {
        let changes = changes_of(
            &[
                PackageSpec::new("bat"),
                PackageSpec::pinned("ripgrep", "14.1.0"),
            ],
            &[PackageSpec::pinned("ripgrep", "14.0.0")],
            true,
        );
        assert_eq!(
            changes.to_install(),
            vec![
                PackageSpec::new("bat"),
                PackageSpec::pinned("ripgrep", "14.1.0")
            ]
        );
    }

    #[test]
    fn a_repin_knows_which_way_it_moves() {
        let up = Repin {
            spec: PackageSpec::pinned("ripgrep", "14.1.0"),
            installed: "14.0.0".into(),
        };
        assert_eq!(up.direction(), Some(Ordering::Greater));

        let down = Repin {
            spec: PackageSpec::pinned("ripgrep", "14.0.0"),
            installed: "14.1.0".into(),
        };
        assert_eq!(down.direction(), Some(Ordering::Less));
    }

    #[test]
    fn versions_compare_by_number_not_by_text() {
        // The whole point: "9" sorts above "10" as text, and below it as a number.
        assert_eq!(compare_versions("1.10.0", "1.9.0"), Some(Ordering::Greater));
        assert_eq!(compare_versions("0.15.0", "0.17.0"), Some(Ordering::Less));
        assert_eq!(
            compare_versions("8.0.100", "8.0.100"),
            Some(Ordering::Equal)
        );
    }

    #[test]
    fn a_longer_numeric_version_is_the_higher_one() {
        assert_eq!(compare_versions("1.2.1", "1.2"), Some(Ordering::Greater));
        assert_eq!(compare_versions("1.2", "1.2.1"), Some(Ordering::Less));
        // Equal despite differing text, which is why a repin may have no direction.
        assert_eq!(compare_versions("1.2", "1.2.0"), Some(Ordering::Equal));
    }

    #[test]
    fn an_unrankable_pair_is_admitted_rather_than_guessed() {
        // Pre-release ordering differs per manager, so mpm declines to rank it
        // instead of calling a downgrade an upgrade.
        assert_eq!(compare_versions("1.0.0-rc1", "1.0.0"), None);
        assert_eq!(compare_versions("2.0-alpha", "2.0-beta"), None);
        assert_eq!(compare_versions("stable", "1.0"), None);
    }

    #[test]
    fn output_is_ordered_not_hash_ordered() {
        // Output must be identical run to run, or it cannot be diffed or scripted.
        let declared = [
            PackageSpec::new("zsh"),
            PackageSpec::new("bat"),
            PackageSpec::new("micro"),
        ];
        let first = changes_of(&declared, &[], false);
        let second = changes_of(&declared, &[], false);
        assert_eq!(first.install, second.install);
        assert_eq!(
            first.install,
            vec![
                PackageSpec::new("bat"),
                PackageSpec::new("micro"),
                PackageSpec::new("zsh")
            ]
        );
    }
}

#[cfg(test)]
mod validation_tests {
    use super::*;
    use crate::manager;

    fn declared(specs: &[PackageSpec]) -> Resolved {
        Resolved {
            declared: specs
                .iter()
                .map(|spec| (spec.name.clone(), spec.clone()))
                .collect(),
            repeated: BTreeMap::new(),
        }
    }

    /// One manifest file declaring `name` more than once.
    fn declared_twice(specs: &[PackageSpec]) -> Resolved {
        let name = specs[0].name.clone();
        Resolved {
            declared: [(name.clone(), specs[specs.len() - 1].clone())]
                .into_iter()
                .collect(),
            repeated: [(name, specs.to_vec())].into_iter().collect(),
        }
    }

    #[test]
    fn the_same_package_twice_at_the_same_version_is_always_an_error() {
        // Even where several versions may coexist, two identical lines say
        // nothing the first did not.
        let gem = manager::get("gem").expect("gem");
        let error = validate(
            gem.as_ref(),
            &declared_twice(&[PackageSpec::new("tilt"), PackageSpec::new("tilt")]),
        )
        .expect_err("a plain duplicate is a mistake");
        assert!(error.to_string().contains("same version"), "{error}");
    }

    #[test]
    fn two_versions_of_one_package_are_rejected_where_only_one_can_be_installed() {
        let cargo = manager::get("cargo").expect("cargo");
        let error = validate(
            cargo.as_ref(),
            &declared_twice(&[
                PackageSpec::pinned("ripgrep", "14.0.0"),
                PackageSpec::pinned("ripgrep", "14.1.0"),
            ]),
        )
        .expect_err("cargo keeps one version of a crate");
        assert!(error.to_string().contains("different versions"), "{error}");
        assert!(error.to_string().contains("cargo"), "{error}");
    }

    #[test]
    fn a_repeat_across_layers_is_an_override_not_a_duplicate() {
        // `Layout::resolve` records a repeat only within a single file, so a host
        // layer raising a version carries no `repeated` entry and validates.
        let cargo = manager::get("cargo").expect("cargo");
        validate(
            cargo.as_ref(),
            &declared(&[PackageSpec::pinned("ripgrep", "14.1.0")]),
        )
        .expect("one surviving declaration is fine");
    }

    #[test]
    fn a_pin_on_a_manager_that_cannot_pin_is_an_error() {
        let brew = manager::get("brew").expect("brew");
        let error = validate(
            brew.as_ref(),
            &declared(&[PackageSpec::pinned("ripgrep", "14.1.0")]),
        )
        .expect_err("must reject");
        assert!(
            error
                .to_string()
                .contains("cannot install a specific version")
        );
    }

    #[test]
    fn a_pin_on_a_manager_that_can_pin_is_accepted() {
        let cargo = manager::get("cargo").expect("cargo");
        validate(
            cargo.as_ref(),
            &declared(&[PackageSpec::pinned("ripgrep", "14.1.0")]),
        )
        .expect("accepted");
    }

    #[test]
    fn a_versioned_formula_plus_a_pin_is_a_contradiction() {
        let brew = manager::get("brew").expect("brew");
        let error = validate(
            brew.as_ref(),
            &declared(&[PackageSpec::pinned("node@20", "20.11.0")]),
        )
        .expect_err("must reject");
        assert!(error.to_string().contains("already selects a version"));
    }

    #[test]
    fn a_versioned_formula_alone_is_fine() {
        let brew = manager::get("brew").expect("brew");
        validate(brew.as_ref(), &declared(&[PackageSpec::new("node@20")])).expect("accepted");
    }

    #[test]
    fn unpinned_entries_are_always_fine() {
        for id in manager::ALL {
            let manager = manager::get(id).expect("registered");
            validate(manager.as_ref(), &declared(&[PackageSpec::new("vim")]))
                .unwrap_or_else(|error| panic!("`{id}` rejected an unpinned package: {error}"));
        }
    }

    #[test]
    fn every_problem_is_reported_at_once() {
        let brew = manager::get("brew").expect("brew");
        let error = validate(
            brew.as_ref(),
            &declared(&[
                PackageSpec::pinned("ripgrep", "14.1.0"),
                PackageSpec::pinned("bat", "0.24.0"),
            ]),
        )
        .expect_err("must reject");
        assert!(error.to_string().contains("ripgrep"));
        assert!(error.to_string().contains("bat"));
    }
}
