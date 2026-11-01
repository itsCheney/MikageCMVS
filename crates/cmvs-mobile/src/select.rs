//! Engine profile selection for player hosts.
//!
//! Upstream refuses any game whose start script and script archive hashes it has
//! not recorded. A player has to start updated releases and patched copies too, so
//! when the strict selection fails this picks a profile from the executables in the
//! folder (a recorded build) and otherwise the baseline profile, and the session then
//! runs presumed and unreviewed commands as well. Unsupported commands still stop.
use cmvs_core::profile::{Evidence, FileEvidence, Game, Registry, SelectionError};
use std::path::Path;

/// The profile every other one is compared with (アマツツミ, 2016-02-01).
pub const BASELINE_PROFILE: &str = "cmvs-20160201";

/// How the profile was chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Basis {
    /// The game is recorded upstream (data hashes match).
    Recorded,
    /// An executable in the folder is a recorded build of this profile.
    Executable(String),
    /// Nothing matched; the baseline profile was assumed.
    Assumed,
}

pub struct Selection {
    pub game: Game<'static>,
    pub basis: Basis,
    /// Why the strict selection failed, when it did.
    pub strict_error: Option<String>,
}
impl Selection {
    /// Whether presumed and unreviewed commands should run.
    pub fn lenient(&self) -> bool {
        self.basis != Basis::Recorded
    }
}

pub fn select(root: &Path) -> Result<Selection, SelectionError> {
    let registry = Registry::builtin();
    let error = match registry.identify_game(root, None) {
        Ok(game) => {
            return Ok(Selection {
                game,
                basis: Basis::Recorded,
                strict_error: None,
            });
        }
        Err(e @ (SelectionError::UnknownGame(_) | SelectionError::UnknownBinary { .. })) => e,
        Err(e) => return Err(e),
    };
    let executables = Evidence::executables_in(root)?;
    let (profile, basis) = match profile_for_executables(registry, &executables) {
        Some((profile, file)) => (profile, Basis::Executable(file)),
        None => (BASELINE_PROFILE.to_owned(), Basis::Assumed),
    };
    Ok(Selection {
        game: registry.identify_game(root, Some(&profile))?,
        basis,
        strict_error: Some(error.to_string()),
    })
}

/// The profile of the first executable that is a recorded build, and its file name.
fn profile_for_executables(
    registry: &Registry,
    executables: &[FileEvidence],
) -> Option<(String, String)> {
    executables.iter().find_map(|exe| {
        registry
            .games()
            .iter()
            .find(|game| game.executables.iter().any(|e| e.sha256 == exe.sha256))
            .map(|game| (game.profile.clone(), exe.path.clone()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_profile_exists() {
        assert!(Registry::builtin().profile(BASELINE_PROFILE).is_some());
    }

    #[test]
    fn recorded_executable_names_its_profile() {
        let registry = Registry::builtin();
        let record = registry
            .games()
            .iter()
            .find(|g| g.profile == BASELINE_PROFILE)
            .expect("baseline game record");
        let exe = FileEvidence {
            path: "cmvs32.exe".into(),
            sha256: record.executables[0].sha256,
        };
        let other = FileEvidence {
            path: "cmvs32_CHS.exe".into(),
            sha256: [7; 32],
        };
        assert_eq!(
            profile_for_executables(registry, &[other.clone(), exe]),
            Some((BASELINE_PROFILE.to_owned(), "cmvs32.exe".to_owned()))
        );
        assert_eq!(profile_for_executables(registry, &[other]), None);
    }
}
