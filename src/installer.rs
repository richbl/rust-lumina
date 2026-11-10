//! User-level installer for the `lumina` CLI and the optional `lumina-gui` front end
//!
//! Everything goes under the user's home (no root needed):
//!
//! A. binaries in `~/.local/bin`
//! B the `.desktop` file in `~/.local/share/applications`
//! C. the icon in `~/.local/share/icons/hicolor/scalable/apps`
//!
//! The `.desktop` file and icon are embedded in the binary, so only the executables need to be
//! present at install time

use anyhow::{Context, Result, bail};
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, ErrorKind, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const CLI_NAME: &str = "lumina";
const GUI_NAME: &str = "lumina-gui";
const DESKTOP_NAME: &str = "io.github.richbl.Lumina.desktop";
const ICON_NAME: &str = "io.github.richbl.Lumina.svg";

/// The `.desktop` template; its `Exec=` line is rewritten at install time
const DESKTOP_TEMPLATE: &str = include_str!("../data/io.github.richbl.Lumina.desktop");

/// The application icon
const ICON: &[u8] = include_bytes!("../data/lumina.svg");

/// Permissions for installed executables
const EXEC_MODE: u32 = 0o755;

/// Permissions for installed data files
const DATA_MODE: u32 = 0o644;

/// Every location the installer reads or writes
#[derive(Debug, Clone, PartialEq, Eq)]
struct Paths {
    bin_dir: PathBuf,
    apps_dir: PathBuf,

    /// The `.../icons/hicolor` directory (what `gtk-update-icon-cache` uses)
    theme_dir: PathBuf,
    icon_dir: PathBuf,
    cli: PathBuf,
    gui: PathBuf,
    desktop: PathBuf,
    icon: PathBuf,
}

impl Paths {
    /// Lay out all paths under `bin_dir` and the XDG data directory `data_dir`
    fn new(bin_dir: PathBuf, data_dir: &Path) -> Self {
        let apps_dir = data_dir.join("applications");
        let theme_dir = data_dir.join("icons").join("hicolor");
        let icon_dir = theme_dir.join("scalable").join("apps");

        Self {
            cli: bin_dir.join(CLI_NAME),
            gui: bin_dir.join(GUI_NAME),
            desktop: apps_dir.join(DESKTOP_NAME),
            icon: icon_dir.join(ICON_NAME),
            bin_dir,
            apps_dir,
            theme_dir,
            icon_dir,
        }
    }

    /// Resolve paths from the environment (`HOME`, `XDG_DATA_HOME`, `XDG_BIN_HOME`)
    fn from_env() -> Result<Self> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|h| h.is_absolute())
            .context("HOME is not set to an absolute path")?;

        let data = xdg_dir(std::env::var_os("XDG_DATA_HOME"), || {
            home.join(".local").join("share")
        });

        let bin = xdg_dir(std::env::var_os("XDG_BIN_HOME"), || {
            home.join(".local").join("bin")
        });

        Ok(Self::new(bin, &data))
    }
}

/// An XDG directory from an environment value, ignoring empty or relative values per the spec
fn xdg_dir(value: Option<OsString>, fallback: impl FnOnce() -> PathBuf) -> PathBuf {
    value
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(fallback)
}

/// The `lumina-gui` binary sitting next to `lumina` (CLI mode)
fn gui_sibling(cli: &Path) -> Option<PathBuf> {
    Some(cli.with_file_name(GUI_NAME)).filter(|p| p.is_file())
}

/// Quote a path for use as the program in an `Exec=` line
fn exec_quote(path: &Path) -> Result<String> {
    const RESERVED: &[char] = &[
        ' ', '\t', '"', '\'', '\\', '>', '<', '~', '|', '&', ';', '$', '*', '?', '#', '(', ')', '`',
    ];

    let s = path
        .to_str()
        .filter(|s| !s.contains(['\n', '\r']))
        .context("install path must be valid UTF-8 without line breaks")?;

    let quote = s.contains(RESERVED);
    let mut out = String::with_capacity(s.len() + 2);

    if quote {
        out.push('"');
    }

    for c in s.chars() {
        match c {
            '%' => out.push_str("%%"),
            '"' | '`' | '$' => {
                out.push_str("\\\\");
                out.push(c);
            }
            '\\' => out.push_str("\\\\\\\\"),
            _ => out.push(c),
        }
    }

    if quote {
        out.push('"');
    }

    Ok(out)
}

/// The `.desktop` file text with `Exec=` pointing at the installed GUI binary
fn render_desktop(template: &str, gui: &Path) -> Result<String> {
    let exec = format!("Exec={}", exec_quote(gui)?);
    let mut found = false;
    let mut out = String::with_capacity(template.len() + exec.len());

    for line in template.lines() {
        if line.starts_with("Exec=") {
            out.push_str(&exec);
            found = true;
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }

    if !found {
        bail!("desktop template has no Exec= line");
    }

    Ok(out)
}

/// Create `dst` atomically: write a sibling temp file, set its mode, then rename it into place
///
/// Renaming (rather than overwriting) means a running binary can be replaced without contention
/// errors (and an interrupted install never leaves a truncated file behind)
fn write_atomic(
    dst: &Path,
    mode: u32,
    fill: impl FnOnce(&mut File) -> io::Result<()>,
) -> Result<()> {
    let name = dst.file_name().context("destination has no file name")?;
    let mut tmp_name = OsString::from(".");

    tmp_name.push(name);
    tmp_name.push(".tmp");

    let tmp = dst.with_file_name(tmp_name);

    let write = || -> io::Result<()> {
        let mut f = File::create(&tmp)?;
        fill(&mut f)?;
        f.set_permissions(fs::Permissions::from_mode(mode))?;
        f.sync_all()?;
        fs::rename(&tmp, dst)
    };

    if let Err(e) = write() {
        let _ = fs::remove_file(&tmp);
        return Err(e).with_context(|| format!("writing {}", dst.display()));
    }

    Ok(())
}

/// Copy an executable into its rightful place
fn copy_binary(src: &Path, dst: &Path) -> Result<()> {
    let mut input = File::open(src).with_context(|| format!("opening {}", src.display()))?;

    write_atomic(dst, EXEC_MODE, |f| io::copy(&mut input, f).map(|_| ()))
}

/// Create a directory (and parents)
fn ensure_dir(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))
}

/// Install the CLI, and, when `gui_src` is given, the GUI binary, `.desktop` file and icon
fn install_to(paths: &Paths, cli_src: &Path, gui_src: Option<&Path>) -> Result<()> {
    // Validate so a bad template or path fails before anything is written
    let desktop = gui_src
        .map(|_| render_desktop(DESKTOP_TEMPLATE, &paths.gui))
        .transpose()?;

    ensure_dir(&paths.bin_dir)?;
    copy_binary(cli_src, &paths.cli)?;

    if let (Some(gui_src), Some(desktop)) = (gui_src, desktop) {
        ensure_dir(&paths.apps_dir)?;
        ensure_dir(&paths.icon_dir)?;
        copy_binary(gui_src, &paths.gui)?;
        write_atomic(&paths.desktop, DATA_MODE, |f| {
            f.write_all(desktop.as_bytes())
        })?;
        write_atomic(&paths.icon, DATA_MODE, |f| f.write_all(ICON))?;
    }

    Ok(())
}

/// Remove every installed file, reporting for each whether it was present. Directories are left
/// alone since other applications share them
fn uninstall_from(paths: &Paths) -> Result<Vec<(PathBuf, bool)>> {
    [&paths.cli, &paths.gui, &paths.desktop, &paths.icon]
        .into_iter()
        .map(|p| match fs::remove_file(p) {
            Ok(()) => Ok((p.clone(), true)),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok((p.clone(), false)),
            Err(e) => Err(e).with_context(|| format!("removing {}", p.display())),
        })
        .collect()
}

/// Run a cache-refresh tool on `dir`
fn refresh(program: &str, flags: &[&str], dir: &Path) {
    if !dir.is_dir() {
        return;
    }

    let status = Command::new(program)
        .args(flags)
        .arg(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    match status {
        Ok(s) if s.success() => {}
        Ok(s) => eprintln!("note: `{program}` exited with {s}"),
        Err(e) if e.kind() == ErrorKind::NotFound => {
            eprintln!("note: `{program}` not found; skipping");
        }
        Err(e) => eprintln!("note: could not run `{program}`: {e}"),
    }
}

/// Ask the desktop environment to notice the changes right away
fn refresh_caches(paths: &Paths) {
    refresh("update-desktop-database", &[], &paths.apps_dir);
    refresh(
        "gtk-update-icon-cache",
        &["-f", "-t", "-q"],
        &paths.theme_dir,
    );
}

/// `true` if `dir` is one of the entries in `PATH`
fn on_path(dir: &Path) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|entry| entry == dir))
}

/// Print the pre-install/uninstall summary
fn print_plan(action: &str, paths: &Paths, with_gui: bool) {
    println!();
    println!("{action} lumina {} files...", env!("CARGO_PKG_VERSION"));
    println!();
    println!("CLI binary:   {}", paths.cli.display());
    if with_gui {
        println!("GUI binary:   {}", paths.gui.display());
        println!("Desktop file: {}", paths.desktop.display());
        println!("Icon:         {}", paths.icon.display());
    }
    println!();
}

/// Install lumina for the current user. The GUI pieces are installed only if `lumina-gui` sits
/// next to the running `lumina` executable (i.e. the build used `--features gui`)
///
/// # Errors
/// Returns an error if paths cannot be resolved or a file cannot be written
pub fn install() -> Result<()> {
    let paths = Paths::from_env()?;
    let cli_src = std::env::current_exe()
        .and_then(fs::canonicalize)
        .context("cannot locate the running executable")?;
    let gui_src = gui_sibling(&cli_src);

    print_plan("Installing", &paths, gui_src.is_some());
    install_to(&paths, &cli_src, gui_src.as_deref())?;
    refresh_caches(&paths);

    if gui_src.is_none() {
        eprintln!(
            "note: {GUI_NAME} not found next to {CLI_NAME}; installed the CLI only \
             (build with `--features gui` to include the GUI)"
        );
    }
    if !on_path(&paths.bin_dir) {
        eprintln!(
            "note: {} is not in your PATH; add it to run `{CLI_NAME}` from a shell",
            paths.bin_dir.display()
        );
    }

    println!("Installation completed successfully.");
    println!();

    Ok(())
}

/// Remove the files placed by `install`. Missing files are not an error, and the config file is
/// left untouched
///
/// # Errors
/// Returns an error if paths cannot be resolved or a file cannot be removed
pub fn uninstall() -> Result<()> {
    let paths = Paths::from_env()?;

    print_plan("Uninstalling", &paths, true);
    let results = uninstall_from(&paths)?;
    refresh_caches(&paths);

    for (path, removed) in &results {
        let label = if *removed {
            "Removed:   "
        } else {
            "Not found: "
        };
        println!("{label} {}", path.display());
    }
    println!();
    println!("Uninstallation completed successfully (your configuration file was not touched).");
    println!();

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, Paths, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let build = tmp.path().join("build");
        fs::create_dir(&build).unwrap();
        let cli = build.join("lumina");
        let gui = build.join("lumina-gui");
        fs::write(&cli, b"cli-bytes").unwrap();
        fs::write(&gui, b"gui-bytes").unwrap();
        let paths = Paths::new(
            tmp.path().join("home/.local/bin"),
            &tmp.path().join("home/.local/share"),
        );

        (tmp, paths, cli, gui)
    }

    fn mode(p: &Path) -> u32 {
        fs::metadata(p).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn paths_follow_the_xdg_layout() {
        let p = Paths::new(PathBuf::from("/h/.local/bin"), Path::new("/h/.local/share"));
        assert_eq!(p.cli, Path::new("/h/.local/bin/lumina"));
        assert_eq!(p.gui, Path::new("/h/.local/bin/lumina-gui"));
        assert_eq!(
            p.desktop,
            Path::new("/h/.local/share/applications/io.github.richbl.Lumina.desktop")
        );
        assert_eq!(
            p.icon,
            Path::new("/h/.local/share/icons/hicolor/scalable/apps/io.github.richbl.Lumina.svg")
        );
        assert_eq!(p.theme_dir, Path::new("/h/.local/share/icons/hicolor"));
    }

    #[test]
    fn xdg_dir_ignores_missing_empty_and_relative_values() {
        let fb = || PathBuf::from("/fallback");
        assert_eq!(xdg_dir(None, fb), Path::new("/fallback"));
        assert_eq!(xdg_dir(Some(OsString::new()), fb), Path::new("/fallback"));
        assert_eq!(xdg_dir(Some("rel/dir".into()), fb), Path::new("/fallback"));
        assert_eq!(xdg_dir(Some("/abs/dir".into()), fb), Path::new("/abs/dir"));
    }

    #[test]
    fn exec_quoting() {
        let q = |s: &str| exec_quote(Path::new(s)).unwrap();
        assert_eq!(q("/h/.local/bin/lumina-gui"), "/h/.local/bin/lumina-gui");
        assert_eq!(q("/h/a b/lumina-gui"), "\"/h/a b/lumina-gui\"");
        assert_eq!(q("/h/$x/g"), r#""/h/\\$x/g""#);
        assert_eq!(q("/h/100%/g"), "/h/100%%/g");
        assert!(exec_quote(Path::new("/h/a\nb")).is_err());
    }

    #[test]
    fn desktop_exec_line_is_rewritten() {
        let out = render_desktop(DESKTOP_TEMPLATE, Path::new("/x/bin/lumina-gui")).unwrap();
        assert!(out.contains("\nExec=/x/bin/lumina-gui\n"));
        assert!(!out.contains("Exec=lumina\n"));
        assert!(out.contains("Icon=io.github.richbl.Lumina"));
        assert!(render_desktop("[Desktop Entry]\nName=x\n", Path::new("/x")).is_err());
    }

    #[test]
    fn embedded_icon_is_an_svg() {
        assert!(String::from_utf8_lossy(ICON).contains("<svg"));
    }

    #[test]
    fn gui_sibling_requires_the_file() {
        let (_tmp, _paths, cli, gui) = fixture();
        assert_eq!(gui_sibling(&cli), Some(gui.clone()));
        fs::remove_file(gui).unwrap();
        assert_eq!(gui_sibling(&cli), None);
    }

    fn assert_installed_contents(paths: &Paths) {
        assert_eq!(fs::read(&paths.cli).unwrap(), b"cli-bytes");
        assert_eq!(fs::read(&paths.gui).unwrap(), b"gui-bytes");
        assert_eq!(fs::read(&paths.icon).unwrap(), ICON);

        let desktop = fs::read_to_string(&paths.desktop).unwrap();
        let exec = exec_quote(&paths.gui).unwrap();
        assert!(desktop.contains(&format!("Exec={exec}\n")));
    }

    fn assert_installed_modes(paths: &Paths) {
        assert_eq!((mode(&paths.cli), mode(&paths.gui)), (0o755, 0o755));
        assert_eq!((mode(&paths.desktop), mode(&paths.icon)), (0o644, 0o644));
    }

    fn assert_no_tmp_files_remain(paths: &Paths) {
        assert!(!paths.bin_dir.join(".lumina.tmp").exists());
        assert!(!paths.bin_dir.join(".lumina-gui.tmp").exists());
    }

    #[test]
    fn install_places_every_file() {
        let (_tmp, paths, cli, gui) = fixture();
        install_to(&paths, &cli, Some(&gui)).unwrap();

        assert_installed_contents(&paths);
        assert_installed_modes(&paths);
        assert_no_tmp_files_remain(&paths);
    }

    #[test]
    fn reinstall_overwrites() {
        let (_tmp, paths, cli, gui) = fixture();
        install_to(&paths, &cli, Some(&gui)).unwrap();
        fs::write(&cli, b"new-cli").unwrap();
        install_to(&paths, &cli, Some(&gui)).unwrap();
        assert_eq!(fs::read(&paths.cli).unwrap(), b"new-cli");
    }

    #[test]
    fn cli_only_install_skips_gui_assets() {
        let (_tmp, paths, cli, _gui) = fixture();
        install_to(&paths, &cli, None).unwrap();

        assert!(paths.cli.is_file());
        assert!(!paths.gui.exists() && !paths.desktop.exists() && !paths.icon.exists());
    }

    #[test]
    fn uninstall_removes_files_and_is_idempotent() {
        let (_tmp, paths, cli, gui) = fixture();
        install_to(&paths, &cli, Some(&gui)).unwrap();

        let first = uninstall_from(&paths).unwrap();
        assert!(first.iter().all(|(_, removed)| *removed));
        assert!(!paths.cli.exists() && !paths.desktop.exists() && !paths.icon.exists());
        assert!(paths.bin_dir.is_dir(), "directories are left in place");

        let second = uninstall_from(&paths).unwrap();
        assert!(second.iter().all(|(_, removed)| !*removed));
    }
}
