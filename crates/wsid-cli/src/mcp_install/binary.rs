//! Work out which path to register in AI client configs.

use std::path::{Path, PathBuf};

/// What the binary path means for the clients that will launch it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    /// A path any host client can launch.
    Path(String),
    /// Running as a Snap. Strict confinement blocks the clients' config files, so only print the config.
    Snap(String),
    /// Running inside a Flatpak sandbox. Host clients cannot launch this path.
    Flatpak,
}

/// Everything resolution reads from the environment, so tests can supply it.
#[derive(Debug, Default)]
pub struct Inputs {
    pub explicit: Option<PathBuf>,
    pub current_exe: Option<PathBuf>,
    pub snap_name: Option<String>,
    pub flatpak_id: Option<String>,
    /// The path `which` finds for the running binary's own name, if any.
    pub which: Option<PathBuf>,
}

impl Inputs {
    pub fn from_env(explicit: Option<&Path>) -> Self {
        let current_exe = std::env::current_exe().ok();
        let which = current_exe
            .as_ref()
            .and_then(|exe| exe.file_name())
            .and_then(|name| which::which(name).ok());
        Self {
            explicit: explicit.map(Path::to_path_buf),
            current_exe,
            snap_name: std::env::var("SNAP")
                .ok()
                .filter(|v| !v.is_empty())
                .and_then(|_| std::env::var("SNAP_NAME").ok()),
            flatpak_id: std::env::var("FLATPAK_ID").ok().filter(|v| !v.is_empty()),
            which,
        }
    }
}

pub fn resolve(inputs: &Inputs) -> Result<Resolved, String> {
    if let Some(path) = &inputs.explicit {
        let absolute = std::path::absolute(path).map_err(|err| format!("invalid --binary path: {err}"))?;
        return Ok(Resolved::Path(absolute.to_string_lossy().into_owned()));
    }

    let exe = inputs
        .current_exe
        .as_ref()
        .ok_or_else(|| "could not find the path of this program; pass --binary".to_string())?;
    let exe_str = exe.to_string_lossy();

    if inputs.flatpak_id.is_some() || exe_str.starts_with("/app/") {
        return Ok(Resolved::Flatpak);
    }

    if let Some(name) = &inputs.snap_name {
        return Ok(Resolved::Snap(format!("/snap/bin/{name}")));
    }

    // Homebrew keeps versioned copies under Cellar; the prefix's bin link survives upgrades.
    if let Some((prefix, _)) = exe_str.split_once("/Cellar/")
        && let Some(name) = exe.file_name()
    {
        return Ok(Resolved::Path(format!("{prefix}/bin/{}", name.to_string_lossy())));
    }

    // A stable link on PATH (winget, apt, cargo) beats the resolved real path. Do not canonicalize it.
    if let Some(found) = &inputs.which
        && same_file(found, exe)
    {
        return Ok(Resolved::Path(found.to_string_lossy().into_owned()));
    }

    Ok(Resolved::Path(exe_str.into_owned()))
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exe(path: &str) -> Inputs {
        Inputs { current_exe: Some(PathBuf::from(path)), ..Inputs::default() }
    }

    #[test]
    fn explicit_binary_wins_and_is_made_absolute() {
        let dir = tempfile::tempdir().unwrap();
        let explicit = dir.path().join("wsid");
        let inputs = Inputs { explicit: Some(explicit.clone()), ..exe("/somewhere/else/wsid") };
        assert_eq!(resolve(&inputs).unwrap(), Resolved::Path(explicit.to_string_lossy().into_owned()));
        // A relative path is made absolute.
        let relative = Inputs { explicit: Some(PathBuf::from("bin/wsid")), ..exe("/somewhere/else/wsid") };
        let Resolved::Path(path) = resolve(&relative).unwrap() else { panic!("not a path") };
        assert!(Path::new(&path).is_absolute());
    }

    #[test]
    fn homebrew_cellar_path_maps_to_the_prefix_bin_link() {
        let inputs = exe("/opt/homebrew/Cellar/wsid/0.2.0/bin/wsid");
        assert_eq!(resolve(&inputs).unwrap(), Resolved::Path("/opt/homebrew/bin/wsid".to_string()));
        let beta = exe("/usr/local/Cellar/wsid-beta/0.2.1-beta.3/bin/wsid-beta");
        assert_eq!(resolve(&beta).unwrap(), Resolved::Path("/usr/local/bin/wsid-beta".to_string()));
    }

    // Windows cannot make a link here without elevated rights, and hard links keep their own path,
    // so the link case runs on Unix only.
    #[cfg(unix)]
    #[test]
    fn a_path_link_to_the_same_file_is_kept_uncanonicalized() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real").join("wsid");
        std::fs::create_dir_all(real.parent().unwrap()).unwrap();
        std::fs::write(&real, "x").unwrap();
        let link_dir = dir.path().join("links");
        std::fs::create_dir_all(&link_dir).unwrap();
        let link = link_dir.join("wsid");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let inputs = Inputs { current_exe: Some(real), which: Some(link.clone()), ..Inputs::default() };
        assert_eq!(resolve(&inputs).unwrap(), Resolved::Path(link.to_string_lossy().into_owned()));
    }

    #[test]
    fn a_path_entry_for_the_running_file_is_used() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("wsid");
        std::fs::write(&exe, "x").unwrap();
        let inputs = Inputs { current_exe: Some(exe.clone()), which: Some(exe.clone()), ..Inputs::default() };
        assert_eq!(resolve(&inputs).unwrap(), Resolved::Path(exe.to_string_lossy().into_owned()));
    }

    #[test]
    fn an_unrelated_which_result_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let mine = dir.path().join("mine");
        let other = dir.path().join("other");
        std::fs::write(&mine, "1").unwrap();
        std::fs::write(&other, "2").unwrap();
        let inputs = Inputs { current_exe: Some(mine.clone()), which: Some(other), ..Inputs::default() };
        assert_eq!(resolve(&inputs).unwrap(), Resolved::Path(mine.to_string_lossy().into_owned()));
    }

    #[test]
    fn snap_uses_the_snap_bin_path_and_flatpak_is_refused() {
        let snap = Inputs { snap_name: Some("wsid".to_string()), ..exe("/snap/wsid/12/bin/wsid") };
        assert_eq!(resolve(&snap).unwrap(), Resolved::Snap("/snap/bin/wsid".to_string()));
        let flatpak = Inputs { flatpak_id: Some("io.github.whatshouldidotoday.wsid".to_string()), ..exe("/app/bin/wsid") };
        assert_eq!(resolve(&flatpak).unwrap(), Resolved::Flatpak);
        assert_eq!(resolve(&exe("/app/bin/wsid")).unwrap(), Resolved::Flatpak);
    }

    #[test]
    fn missing_current_exe_asks_for_binary() {
        assert!(resolve(&Inputs::default()).is_err());
    }
}
