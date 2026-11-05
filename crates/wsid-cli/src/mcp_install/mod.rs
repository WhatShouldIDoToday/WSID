//! `wsid mcp install`, `uninstall`, and `status`: put WSID's MCP server into AI client configs.
//!
//! Nothing here starts the server or opens the database.

mod adapters;
mod binary;
mod edit;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use serde_json::Value;

use self::adapters::{Client, Env, Method, SERVER_ARGS, clients, find, launch_line, path_is_present};
use self::binary::Resolved;
use crate::args::{McpAction, McpInstallArgs, McpUninstallArgs};
use crate::cli_runner::OutputMode;

/// What the installer needs from the machine besides files, so tests can fake it.
trait Host {
    fn which(&self, program: &str) -> Option<PathBuf>;
    fn run(&self, program: &str, args: &[String]) -> Result<(), String>;
}

struct SystemHost;

impl Host for SystemHost {
    fn which(&self, program: &str) -> Option<PathBuf> {
        which::which(program).ok()
    }

    fn run(&self, program: &str, args: &[String]) -> Result<(), String> {
        let path = which::which(program).map_err(|_| format!("`{program}` was not found on PATH"))?;
        let output = Command::new(path)
            .args(args)
            .output()
            .map_err(|err| format!("could not run `{program}`: {err}"))?;
        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("`{program}` failed: {}", stderr.trim()))
        }
    }
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct Report {
    agent: String,
    label: String,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

impl Report {
    fn new(client: &Client, status: &'static str) -> Self {
        Self { agent: client.id.to_string(), label: client.label.to_string(), status, path: None, detail: None }
    }

    fn at(mut self, path: &Path) -> Self {
        self.path = Some(path.display().to_string());
        self
    }

    fn because(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    fn is_failure(&self) -> bool {
        matches!(self.status, "conflict" | "error")
    }
}

/// Entry point for the `wsid mcp` subcommands.
pub fn run<W: Write>(action: &McpAction, mode: OutputMode, out: &mut W) -> Result<(), String> {
    let env = Env::from_system()?;
    let key = wsid_core::APP_DIR_NAME;
    let host = SystemHost;
    let reports = match action {
        McpAction::Install(args) => {
            let inputs = binary::Inputs::from_env(args.binary.as_deref());
            install(&env, &host, key, &binary::resolve(&inputs)?, args)?
        }
        McpAction::Uninstall(args) => uninstall(&env, &host, key, args)?,
        McpAction::Status => {
            let resolved = binary::resolve(&binary::Inputs::from_env(None)).ok();
            status(&env, &host, key, resolved.as_ref())
        }
    };
    render(&reports, mode, out).map_err(|err| err.to_string())?;
    if reports.iter().any(Report::is_failure) {
        return Err("some clients were not changed; see above".to_string());
    }
    Ok(())
}

fn render<W: Write>(reports: &[Report], mode: OutputMode, out: &mut W) -> std::io::Result<()> {
    if mode == OutputMode::Json {
        let text = serde_json::to_string_pretty(reports).unwrap_or_else(|_| "[]".to_string());
        return writeln!(out, "{text}");
    }
    if reports.is_empty() {
        return writeln!(out, "No clients matched.");
    }
    for report in reports {
        write!(out, "{} ({}): {}", report.label, report.agent, report.status)?;
        if let Some(path) = &report.path {
            write!(out, " [{path}]")?;
        }
        writeln!(out)?;
        if let Some(detail) = &report.detail {
            for line in detail.lines() {
                writeln!(out, "  {line}")?;
            }
        }
    }
    Ok(())
}

// ------------------------------------------------------------------ selection

fn select<'a>(all: &'a [Client], agents: &[String], all_detected: bool, fallback_all: bool) -> Result<Vec<&'a Client>, String> {
    let mut chosen: Vec<&Client> = Vec::new();
    for name in agents {
        let client = find(all, name).ok_or_else(|| {
            let ids: Vec<&str> = all.iter().map(|c| c.id).collect();
            format!("unknown client '{name}'. Choose from: {}", ids.join(", "))
        })?;
        if !chosen.iter().any(|c| c.id == client.id) {
            chosen.push(client);
        }
    }
    if all_detected {
        for client in all {
            if !chosen.iter().any(|c| c.id == client.id) {
                chosen.push(client);
            }
        }
    }
    if chosen.is_empty() {
        if fallback_all {
            return Ok(all.iter().collect());
        }
        return Err("choose a client with --agent <id>, or use --all-detected. Run `wsid mcp status` to list client IDs".to_string());
    }
    Ok(chosen)
}

fn is_detected(client: &Client, env: &Env, host: &dyn Host) -> bool {
    let paths = (client.paths)(env);
    match &client.method {
        Method::File => paths.iter().any(|p| path_is_present(p)),
        // A file directly in the home folder says nothing, so these need the file itself or the program.
        Method::Command { program, .. } => paths.iter().any(|p| p.exists()) || host.which(program).is_some(),
    }
}

// ------------------------------------------------------------------- existing

enum State {
    Missing,
    Same,
    Different(Vec<String>),
}

fn wanted_line(bin: &str) -> Vec<String> {
    std::iter::once(bin.to_string())
        .chain(SERVER_ARGS.iter().map(|s| (*s).to_string()))
        .collect()
}

fn classify(existing: Option<&Value>, bin: &str) -> State {
    match existing {
        None => State::Missing,
        Some(entry) => {
            let line = launch_line(entry);
            if line == wanted_line(bin) { State::Same } else { State::Different(line) }
        }
    }
}

fn read_entry(client: &Client, path: &Path, key: &str) -> Result<Option<Value>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => edit::get(client.format, &text, client.section, key),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(format!("could not read {}: {err}", path.display())),
    }
}

/// The first entry named `key` across the client's candidate files, and the file it was found in.
fn find_entry(client: &Client, env: &Env, key: &str) -> Result<Option<(PathBuf, Value)>, String> {
    for path in (client.paths)(env) {
        if let Some(entry) = read_entry(client, &path, key)? {
            return Ok(Some((path, entry)));
        }
    }
    Ok(None)
}

// -------------------------------------------------------------------- install

fn install(env: &Env, host: &dyn Host, key: &str, resolved: &Resolved, args: &McpInstallArgs) -> Result<Vec<Report>, String> {
    if *resolved == Resolved::Flatpak {
        return Err("this copy of WSID runs inside a Flatpak sandbox, so AI clients on your system cannot launch it. Install the `wsid` command line tool on the host, then run this there".to_string());
    }
    let all = clients();
    let selected = select(&all, &args.agents, args.all_detected, args.print)?;
    let mut reports = Vec::new();

    for client in selected {
        let detected = is_detected(client, env, host);
        if args.all_detected && args.agents.is_empty() && !detected {
            continue;
        }
        let (bin, manual) = match resolved {
            Resolved::Path(path) => (path.as_str(), args.print),
            Resolved::Snap(path) => (path.as_str(), true),
            Resolved::Flatpak => unreachable!("handled above"),
        };
        if manual {
            let mut report = Report::new(client, "manual").because(snippet(client, bin, key)?);
            if matches!(resolved, Resolved::Snap(_)) && !args.print {
                report = report.because(format!(
                    "WSID is running as a Snap, which cannot edit client config files. Add this yourself:\n{}",
                    snippet(client, bin, key)?
                ));
            }
            reports.push(report);
            continue;
        }
        match client.method {
            Method::File => install_file(client, env, key, bin, args, &mut reports),
            Method::Command { .. } => install_command(client, env, host, key, bin, args, &detected, &mut reports),
        }
    }
    Ok(reports)
}

/// The config a client would hold, for `--print` and Snap.
fn snippet(client: &Client, bin: &str, key: &str) -> Result<String, String> {
    let entry = (client.entry)(bin, SERVER_ARGS);
    edit::set(client.format, "", client.section, key, &entry)
}

fn install_file(client: &Client, env: &Env, key: &str, bin: &str, args: &McpInstallArgs, reports: &mut Vec<Report>) {
    let candidates = (client.paths)(env);
    let mut targets: Vec<PathBuf> = candidates
        .iter()
        .filter(|p| path_is_present(p))
        .cloned()
        .collect();
    if targets.is_empty() {
        if args.force {
            targets = candidates.into_iter().take(1).collect();
        } else {
            reports.push(
                Report::new(client, "not detected")
                    .because("Its config folder was not found. Use --force to create the config file anyway."),
            );
            return;
        }
    }
    for path in targets {
        reports.push(install_one_file(client, &path, key, bin, args));
    }
}

fn install_one_file(client: &Client, path: &Path, key: &str, bin: &str, args: &McpInstallArgs) -> Report {
    let report = |status| Report::new(client, status).at(path);
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => return report("error").because(format!("could not read the file: {err}")),
    };
    let existing = match edit::get(client.format, &text, client.section, key) {
        Ok(existing) => existing,
        Err(err) => return report("error").because(format!("{err}. The file was left unchanged.")),
    };
    match classify(existing.as_ref(), bin) {
        State::Same => return report("already installed"),
        State::Different(line) if !args.force => {
            return report("conflict").because(format!("`{key}` already points to `{}`. Use --force to replace it.", line.join(" ")));
        }
        _ => {}
    }
    if args.dry_run {
        return report("would install");
    }
    let entry = (client.entry)(bin, SERVER_ARGS);
    let updated = match edit::set(client.format, &text, client.section, key, &entry) {
        Ok(updated) => updated,
        Err(err) => return report("error").because(format!("{err}. The file was left unchanged.")),
    };
    match edit::write_atomic(path, &updated) {
        Ok(()) => report("installed"),
        Err(err) => report("error").because(err),
    }
}

#[allow(clippy::too_many_arguments)]
fn install_command(
    client: &Client,
    env: &Env,
    host: &dyn Host,
    key: &str,
    bin: &str,
    args: &McpInstallArgs,
    detected: &bool,
    reports: &mut Vec<Report>,
) {
    let Method::Command { program, add, remove } = &client.method else { return };
    let report = |status| Report::new(client, status);
    if host.which(program).is_none() {
        let note = if *detected { format!("`{program}` was not found on PATH, so its command cannot run.") } else { String::new() };
        reports.push(report("not detected").because(note));
        return;
    }
    let existing = match find_entry(client, env, key) {
        Ok(found) => found,
        Err(err) => {
            reports.push(report("error").because(err));
            return;
        }
    };
    let must_remove_first = match classify(existing.as_ref().map(|(_, e)| e), bin) {
        State::Same => {
            reports.push(report("already installed"));
            return;
        }
        State::Different(line) if !args.force => {
            reports.push(report("conflict").because(format!("`{key}` already points to `{}`. Use --force to replace it.", line.join(" "))));
            return;
        }
        State::Different(_) => true,
        State::Missing => false,
    };
    if args.dry_run {
        reports.push(report("would install").because(format!("{program} {}", add(bin, key).join(" "))));
        return;
    }
    let remove_args = remove(key);
    if must_remove_first
        && !remove_args.is_empty()
        && let Err(err) = host.run(program, &remove_args)
    {
        reports.push(report("error").because(err));
        return;
    }
    match host.run(program, &add(bin, key)) {
        Ok(()) => reports.push(report("installed")),
        Err(err) => reports.push(report("error").because(err)),
    }
}

// ------------------------------------------------------------------ uninstall

fn uninstall(env: &Env, host: &dyn Host, key: &str, args: &McpUninstallArgs) -> Result<Vec<Report>, String> {
    let all = clients();
    let selected = select(&all, &args.agents, args.all_detected, false)?;
    let explicit = !args.agents.is_empty();
    let mut reports = Vec::new();

    for client in selected {
        let present: Vec<PathBuf> = (client.paths)(env)
            .into_iter()
            .filter(|p| matches!(read_entry(client, p, key), Ok(Some(_))))
            .collect();
        if present.is_empty() {
            if explicit {
                reports.push(Report::new(client, "not installed"));
            }
            continue;
        }
        if let Method::Command { program, remove, .. } = &client.method {
            let remove_args = remove(key);
            if !remove_args.is_empty() && host.which(program).is_some() {
                if args.dry_run {
                    reports.push(Report::new(client, "would remove").because(format!("{program} {}", remove_args.join(" "))));
                } else {
                    match host.run(program, &remove_args) {
                        Ok(()) => reports.push(Report::new(client, "removed")),
                        Err(err) => reports.push(Report::new(client, "error").because(err)),
                    }
                }
                continue;
            }
        }
        for path in present {
            reports.push(uninstall_one_file(client, &path, key, args.dry_run));
        }
    }
    Ok(reports)
}

fn uninstall_one_file(client: &Client, path: &Path, key: &str, dry_run: bool) -> Report {
    let report = |status| Report::new(client, status).at(path);
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) => return report("error").because(format!("could not read the file: {err}")),
    };
    match edit::remove(client.format, &text, client.section, key) {
        Ok((_, false)) => report("not installed"),
        Ok(_) if dry_run => report("would remove"),
        Ok((updated, true)) => match edit::write_atomic(path, &updated) {
            Ok(()) => report("removed"),
            Err(err) => report("error").because(err),
        },
        Err(err) => report("error").because(format!("{err}. The file was left unchanged.")),
    }
}

// --------------------------------------------------------------------- status

fn status(env: &Env, host: &dyn Host, key: &str, resolved: Option<&Resolved>) -> Vec<Report> {
    let bin = match resolved {
        Some(Resolved::Path(p) | Resolved::Snap(p)) => Some(p.as_str()),
        _ => None,
    };
    clients()
        .iter()
        .map(|client| {
            let report = |status| Report::new(client, status);
            match find_entry(client, env, key) {
                Err(err) => report("error").because(err),
                Ok(Some((path, entry))) => {
                    let line = launch_line(&entry);
                    match bin {
                        Some(bin) if line != wanted_line(bin) => report("configured, different binary")
                            .at(&path)
                            .because(format!("launches `{}`", line.join(" "))),
                        _ => report("configured").at(&path),
                    }
                }
                Ok(None) if is_detected(client, env, host) => report("detected, not configured"),
                Ok(None) => report("not detected"),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::fs;

    use serde_json::json;

    use super::*;

    #[derive(Default)]
    struct FakeHost {
        programs: Vec<&'static str>,
        calls: RefCell<Vec<(String, Vec<String>)>>,
    }

    impl Host for FakeHost {
        fn which(&self, program: &str) -> Option<PathBuf> {
            self.programs
                .contains(&program)
                .then(|| PathBuf::from(format!("/usr/bin/{program}")))
        }

        fn run(&self, program: &str, args: &[String]) -> Result<(), String> {
            self.calls
                .borrow_mut()
                .push((program.to_string(), args.to_vec()));
            Ok(())
        }
    }

    fn env() -> (tempfile::TempDir, Env) {
        let dir = tempfile::tempdir().unwrap();
        let env = Env { home: dir.path().join("home"), config: dir.path().join("config") };
        fs::create_dir_all(&env.home).unwrap();
        fs::create_dir_all(&env.config).unwrap();
        (dir, env)
    }

    fn install_args(agents: &[&str]) -> McpInstallArgs {
        McpInstallArgs {
            agents: agents.iter().map(|s| (*s).to_string()).collect(),
            all_detected: false,
            dry_run: false,
            print: false,
            binary: None,
            force: false,
        }
    }

    fn uninstall_args(agents: &[&str]) -> McpUninstallArgs {
        McpUninstallArgs { agents: agents.iter().map(|s| (*s).to_string()).collect(), all_detected: false, dry_run: false }
    }

    fn bin(path: &str) -> Resolved {
        Resolved::Path(path.to_string())
    }

    const FILE_CLIENTS: &[&str] = &["claude-desktop", "cursor", "devin", "zed", "opencode", "cline", "codex", "gemini"];

    fn prepare(client: &Client, env: &Env) -> PathBuf {
        let path = (client.paths)(env).remove(0);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        path
    }

    fn only(reports: Vec<Report>) -> Report {
        assert_eq!(reports.len(), 1, "{reports:?}");
        reports.into_iter().next().unwrap()
    }

    #[test]
    fn every_file_client_installs_into_a_new_file_and_reruns_as_a_no_op() {
        let all = clients();
        for id in FILE_CLIENTS {
            let (_dir, env) = env();
            let client = find(&all, id).unwrap();
            let path = prepare(client, &env);
            let host = FakeHost::default();

            let first = only(install(&env, &host, "wsid", &bin("/bin/wsid"), &install_args(&[id])).unwrap());
            assert_eq!(first.status, "installed", "{id}");
            let text = fs::read_to_string(&path).unwrap();
            let entry = edit::get(client.format, &text, client.section, "wsid")
                .unwrap()
                .unwrap();
            assert_eq!(launch_line(&entry), ["/bin/wsid", "mcp"], "{id}");

            let before = fs::read_to_string(&path).unwrap();
            let second = only(install(&env, &host, "wsid", &bin("/bin/wsid"), &install_args(&[id])).unwrap());
            assert_eq!(second.status, "already installed", "{id}");
            assert_eq!(fs::read_to_string(&path).unwrap(), before, "{id}");
        }
    }

    #[test]
    fn existing_unrelated_servers_and_settings_survive_install_and_uninstall() {
        let all = clients();
        for id in FILE_CLIENTS {
            let (_dir, env) = env();
            let client = find(&all, id).unwrap();
            let path = prepare(client, &env);
            let seed = edit::set(client.format, "", client.section, "other", &(client.entry)("/bin/other", &["serve"])).unwrap();
            fs::write(&path, &seed).unwrap();
            let host = FakeHost::default();

            install(&env, &host, "wsid", &bin("/bin/wsid"), &install_args(&[id])).unwrap();
            let text = fs::read_to_string(&path).unwrap();
            assert!(
                edit::get(client.format, &text, client.section, "other")
                    .unwrap()
                    .is_some(),
                "{id}"
            );
            assert_eq!(fs::read_to_string(edit::backup_path(&path)).unwrap(), seed, "{id}");

            let removed = only(uninstall(&env, &host, "wsid", &uninstall_args(&[id])).unwrap());
            assert_eq!(removed.status, "removed", "{id}");
            let text = fs::read_to_string(&path).unwrap();
            assert!(
                edit::get(client.format, &text, client.section, "wsid")
                    .unwrap()
                    .is_none(),
                "{id}"
            );
            assert!(
                edit::get(client.format, &text, client.section, "other")
                    .unwrap()
                    .is_some(),
                "{id}"
            );
        }
    }

    #[test]
    fn a_conflicting_entry_is_kept_unless_forced() {
        let all = clients();
        for id in FILE_CLIENTS {
            let (_dir, env) = env();
            let client = find(&all, id).unwrap();
            let path = prepare(client, &env);
            let host = FakeHost::default();
            install(&env, &host, "wsid", &bin("/old/wsid"), &install_args(&[id])).unwrap();
            let before = fs::read_to_string(&path).unwrap();

            let refused = only(install(&env, &host, "wsid", &bin("/new/wsid"), &install_args(&[id])).unwrap());
            assert_eq!(refused.status, "conflict", "{id}");
            assert_eq!(fs::read_to_string(&path).unwrap(), before, "{id}");

            let mut args = install_args(&[id]);
            args.force = true;
            let forced = only(install(&env, &host, "wsid", &bin("/new/wsid"), &args).unwrap());
            assert_eq!(forced.status, "installed", "{id}");
            let text = fs::read_to_string(&path).unwrap();
            let entry = edit::get(client.format, &text, client.section, "wsid")
                .unwrap()
                .unwrap();
            assert_eq!(launch_line(&entry), ["/new/wsid", "mcp"], "{id}");
        }
    }

    #[test]
    fn jsonc_clients_keep_comments() {
        let all = clients();
        for (id, rel) in [("zed", ".config/zed/settings.json"), ("opencode", ".config/opencode/opencode.jsonc")] {
            let (_dir, env) = env();
            if cfg!(windows) && id == "zed" {
                continue;
            }
            let path = env.home.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, "{\n  // keep this note\n  \"theme\": \"One Dark\", // and this one\n}\n").unwrap();
            assert!(find(&all, id).is_some());
            install(&env, &FakeHost::default(), "wsid", &bin("/bin/wsid"), &install_args(&[id])).unwrap();
            let text = fs::read_to_string(&path).unwrap();
            assert!(text.contains("// keep this note") && text.contains("// and this one"), "{id}: {text}");
            assert!(text.contains("\"theme\": \"One Dark\""), "{id}");
        }
    }

    #[test]
    fn undetected_clients_are_skipped_unless_forced() {
        let (_dir, env) = env();
        let host = FakeHost::default();
        let skipped = only(install(&env, &host, "wsid", &bin("/bin/wsid"), &install_args(&["gemini"])).unwrap());
        assert_eq!(skipped.status, "not detected");
        assert!(!env.home.join(".gemini").exists());

        let mut args = install_args(&["gemini"]);
        args.force = true;
        let created = only(install(&env, &host, "wsid", &bin("/bin/wsid"), &args).unwrap());
        assert_eq!(created.status, "installed");
        assert!(env.home.join(".gemini/settings.json").exists());
    }

    #[test]
    fn an_unparsable_file_is_left_untouched() {
        let (_dir, env) = env();
        let path = env.home.join(".cursor/mcp.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{ this is not json").unwrap();
        let report = only(install(&env, &FakeHost::default(), "wsid", &bin("/bin/wsid"), &install_args(&["cursor"])).unwrap());
        assert_eq!(report.status, "error");
        assert_eq!(fs::read_to_string(&path).unwrap(), "{ this is not json");
        assert!(!edit::backup_path(&path).exists());
    }

    #[test]
    fn dry_run_and_print_write_nothing() {
        let (_dir, env) = env();
        let path = env.home.join(".cursor/mcp.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut args = install_args(&["cursor"]);
        args.dry_run = true;
        assert_eq!(only(install(&env, &FakeHost::default(), "wsid", &bin("/bin/wsid"), &args).unwrap()).status, "would install");
        assert!(!path.exists());

        let mut args = install_args(&["codex"]);
        args.print = true;
        let printed = only(install(&env, &FakeHost::default(), "wsid", &bin("/bin/wsid"), &args).unwrap());
        assert_eq!(printed.status, "manual");
        assert!(printed.detail.unwrap().contains("[mcp_servers.wsid]"));
        assert!(!env.home.join(".codex").exists());
    }

    #[test]
    fn the_beta_key_installs_beside_stable() {
        let (_dir, env) = env();
        let client = find(&clients(), "cursor").unwrap().id;
        let path = env.home.join(".cursor/mcp.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let host = FakeHost::default();
        install(&env, &host, "wsid", &bin("/bin/wsid"), &install_args(&[client])).unwrap();
        install(&env, &host, "wsid-beta", &bin("/bin/wsid-beta"), &install_args(&[client])).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        let section = ["mcpServers"];
        assert!(
            edit::get(Format::Json, &text, &section, "wsid")
                .unwrap()
                .is_some()
        );
        assert!(
            edit::get(Format::Json, &text, &section, "wsid-beta")
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn snap_prints_manual_config_and_flatpak_is_refused() {
        let (_dir, env) = env();
        let host = FakeHost::default();
        let snap = only(install(&env, &host, "wsid", &Resolved::Snap("/snap/bin/wsid".to_string()), &install_args(&["cursor"])).unwrap());
        assert_eq!(snap.status, "manual");
        assert!(snap.detail.unwrap().contains("/snap/bin/wsid"));
        assert!(!env.home.join(".cursor").exists());
        assert!(install(&env, &host, "wsid", &Resolved::Flatpak, &install_args(&["cursor"])).is_err());
    }

    #[test]
    fn selection_rejects_unknown_and_missing_clients() {
        let (_dir, env) = env();
        let host = FakeHost::default();
        let unknown = install(&env, &host, "wsid", &bin("/b"), &install_args(&["nope"])).unwrap_err();
        assert!(unknown.contains("claude-desktop"));
        assert!(install(&env, &host, "wsid", &bin("/b"), &install_args(&[])).is_err());
    }

    #[test]
    fn all_detected_configures_only_clients_that_are_present() {
        let (_dir, env) = env();
        fs::create_dir_all(env.home.join(".cursor")).unwrap();
        fs::create_dir_all(env.home.join(".gemini")).unwrap();
        let mut args = install_args(&[]);
        args.all_detected = true;
        let reports = install(&env, &FakeHost::default(), "wsid", &bin("/bin/wsid"), &args).unwrap();
        let mut ids: Vec<_> = reports.iter().map(|r| r.agent.as_str()).collect();
        ids.sort_unstable();
        assert_eq!(ids, ["cursor", "gemini"]);
        assert!(reports.iter().all(|r| r.status == "installed"));
    }

    #[test]
    fn claude_code_installs_through_its_cli_and_reads_its_file_for_status() {
        let (_dir, env) = env();
        let host = FakeHost { programs: vec!["claude"], ..FakeHost::default() };
        let report = only(install(&env, &host, "wsid", &bin("/bin/wsid"), &install_args(&["claude-code"])).unwrap());
        assert_eq!(report.status, "installed");
        let calls = host.calls.borrow();
        assert_eq!(calls[0].0, "claude");
        assert_eq!(calls[0].1.join(" "), "mcp add --transport stdio --scope user wsid -- /bin/wsid mcp");
        assert!(!env.home.join(".claude.json").exists(), "the file must not be edited directly");
        drop(calls);

        fs::write(env.home.join(".claude.json"), r#"{"mcpServers":{"wsid":{"type":"stdio","command":"/bin/wsid","args":["mcp"]}}}"#)
            .unwrap();
        assert_eq!(
            only(install(&env, &host, "wsid", &bin("/bin/wsid"), &install_args(&["claude-code"])).unwrap()).status,
            "already installed"
        );
        assert_eq!(host.calls.borrow().len(), 1);

        let conflict = only(install(&env, &host, "wsid", &bin("/other/wsid"), &install_args(&["claude-code"])).unwrap());
        assert_eq!(conflict.status, "conflict");
        let mut forced = install_args(&["claude-code"]);
        forced.force = true;
        assert_eq!(only(install(&env, &host, "wsid", &bin("/other/wsid"), &forced).unwrap()).status, "installed");
        let calls = host.calls.borrow();
        assert_eq!(calls[1].1.join(" "), "mcp remove --scope user wsid");
        assert!(calls[2].1.join(" ").ends_with("-- /other/wsid mcp"));
    }

    #[test]
    fn vscode_installs_with_add_mcp_json_and_uninstalls_by_editing_its_file() {
        let (_dir, env) = env();
        let host = FakeHost { programs: vec!["code"], ..FakeHost::default() };
        assert_eq!(only(install(&env, &host, "wsid", &bin("/bin/wsid"), &install_args(&["vscode"])).unwrap()).status, "installed");
        let calls = host.calls.borrow();
        assert_eq!(calls[0].1[0], "--add-mcp");
        let payload: Value = serde_json::from_str(&calls[0].1[1]).unwrap();
        assert_eq!(payload, json!({"name": "wsid", "type": "stdio", "command": "/bin/wsid", "args": ["mcp"]}));
        drop(calls);

        let path = env.config.join("Code/User/mcp.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, r#"{"servers":{"wsid":{"type":"stdio","command":"/bin/wsid","args":["mcp"]}}}"#).unwrap();
        assert_eq!(only(uninstall(&env, &host, "wsid", &uninstall_args(&["vscode"])).unwrap()).status, "removed");
        assert!(
            edit::get(Format::Json, &fs::read_to_string(&path).unwrap(), &["servers"], "wsid")
                .unwrap()
                .is_none()
        );
    }

    use super::edit::Format;

    #[test]
    fn status_reports_configured_different_and_detected_clients() {
        let (_dir, env) = env();
        fs::create_dir_all(env.home.join(".cursor")).unwrap();
        fs::create_dir_all(env.home.join(".gemini")).unwrap();
        let host = FakeHost::default();
        install(&env, &host, "wsid", &bin("/bin/wsid"), &install_args(&["gemini"])).unwrap();
        let reports = status(&env, &host, "wsid", Some(&bin("/other/wsid")));
        let find = |id: &str| reports.iter().find(|r| r.agent == id).unwrap();
        assert_eq!(find("gemini").status, "configured, different binary");
        assert_eq!(find("cursor").status, "detected, not configured");
        assert_eq!(find("zed").status, "not detected");
        let same = status(&env, &host, "wsid", Some(&bin("/bin/wsid")));
        assert_eq!(same.iter().find(|r| r.agent == "gemini").unwrap().status, "configured");
        assert_eq!(reports.len(), 10);
    }

    #[test]
    fn uninstall_all_detected_skips_clients_without_wsid() {
        let (_dir, env) = env();
        let host = FakeHost::default();
        fs::create_dir_all(env.home.join(".cursor")).unwrap();
        install(&env, &host, "wsid", &bin("/bin/wsid"), &install_args(&["cursor"])).unwrap();
        let mut args = uninstall_args(&[]);
        args.all_detected = true;
        let reports = uninstall(&env, &host, "wsid", &args).unwrap();
        assert_eq!(only(reports).agent, "cursor");
        assert_eq!(only(uninstall(&env, &host, "wsid", &uninstall_args(&["cursor"])).unwrap()).status, "not installed");
    }

    #[test]
    fn render_emits_json_or_linear_text() {
        let client = &clients()[0];
        let reports = vec![Report::new(client, "installed").at(Path::new("/x/y.json"))];
        let mut json_out = Vec::new();
        render(&reports, OutputMode::Json, &mut json_out).unwrap();
        let parsed: Value = serde_json::from_slice(&json_out).unwrap();
        assert_eq!(parsed[0]["agent"], "claude-desktop");
        let mut text = Vec::new();
        render(&reports, OutputMode::Accessible, &mut text).unwrap();
        assert_eq!(String::from_utf8(text).unwrap(), "Claude Desktop (claude-desktop): installed [/x/y.json]\n");
    }
}
