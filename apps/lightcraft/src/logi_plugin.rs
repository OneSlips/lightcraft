//! The LightCraft plugin for Logi Options+ (MX Creative Console): finding it, installing it
//! through Logi's own installer and removing it again (Settings ▸ Controls).
//!
//! The plugin is built from its own repository (C#, the Logi Actions SDK) into
//! `LightCraft.lplug4`. A LightCraft release carries that file next to its executable
//! (`LightCraft.lplug4` or `plugins/LightCraft.lplug4`); `LIGHTCRAFT_LOGI_PLUGIN` points at another
//! copy. Installing hands it to `LogiPluginServiceTool install-plugin` — what double-clicking a
//! `.lplug4` does — so Logi Options+ asks the user to confirm, installs the plugin and its default
//! profiles and reloads its plugin service by itself. Nothing else of Options+ is touched.
//!
//! Windows only for now: elsewhere there is no installer and Settings ▸ Controls says how to
//! install the plugin by hand.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use lightcraft_ui_egui::surface_plugin::{PluginInstaller, PluginStatus};

const PACKAGE: &str = "LightCraft.lplug4";

/// The installer for this platform, if it has one.
pub fn installer() -> Option<Arc<dyn PluginInstaller>> {
    if cfg!(windows) { Some(Arc::new(Logi)) } else { None }
}

struct Logi;

fn env_dir(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// Logi's plugin installer (`LogiPluginServiceTool.exe`).
fn tool() -> Option<PathBuf> {
    let p = env_dir("ProgramFiles")?.join("Logi").join("LogiPluginService").join("LogiPluginServiceTool.exe");
    p.is_file().then_some(p)
}

/// `%LOCALAPPDATA%\Logi\LogiPluginService`.
fn service_dir() -> Option<PathBuf> {
    Some(env_dir("LOCALAPPDATA")?.join("Logi").join("LogiPluginService"))
}

/// The plugin package that comes with this LightCraft.
fn package() -> Option<PathBuf> {
    if let Some(p) = env_dir("LIGHTCRAFT_LOGI_PLUGIN").filter(|p| p.is_file()) {
        return Some(p);
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [dir.join(PACKAGE), dir.join("plugins").join(PACKAGE)].into_iter().find(|p| p.is_file())
}

/// `version:` from a `LoupedeckPackage.yaml`.
fn yaml_version(text: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let v = l.trim().strip_prefix("version:")?.trim().trim_matches(['\'', '"']);
        (!v.is_empty() && v.len() < 40).then(|| v.to_string())
    })
}

fn package_version(path: &Path) -> Option<String> {
    let len = std::fs::metadata(path).ok()?.len();
    if len > lightcraft_surface::zip::MAX_ARCHIVE as u64 {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let yaml = lightcraft_surface::zip::read_member(&bytes, "metadata/LoupedeckPackage.yaml").ok()?;
    yaml_version(&String::from_utf8_lossy(&yaml))
}

fn run_tool(verb: &str, pkg: &Path) -> Result<(), String> {
    let tool = tool().ok_or("Logi Options+ isn't installed on this computer")?;
    let mut c = std::process::Command::new(tool);
    c.arg(verb).arg(pkg);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    let status = c.status().map_err(|e| format!("can't start Logi's installer: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("Logi's installer stopped with {status}")) }
}

impl PluginInstaller for Logi {
    fn name(&self) -> String {
        "LightCraft plugin for Logi Options+".into()
    }

    fn status(&self) -> PluginStatus {
        let installed = service_dir()
            .map(|d| d.join("Plugins").join("LightCraft").join("metadata").join("LoupedeckPackage.yaml"))
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|t| yaml_version(&t).unwrap_or_else(|| "?".into()));
        let pkg = package();
        PluginStatus {
            host: tool().map(|_| "Logi Options+".to_string()),
            installed,
            package: pkg.as_deref().and_then(package_version),
            package_path: pkg.map(|p| p.display().to_string()),
        }
    }

    fn install(&self, port: u16) -> Result<String, String> {
        let pkg = package().ok_or("this LightCraft has no plugin package with it")?;
        // where the plugin finds LightCraft (it reads this when it starts)
        if let Some(dir) = service_dir().map(|d| d.join("PluginData").join("LightCraft")) {
            std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            std::fs::write(dir.join("config.json"), format!("{{\"port\": {port}}}\n")).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        run_tool("install-plugin", &pkg)?;
        Ok("Logi Options+ asks to install the plugin: click Continue there. Then approve the pairing request here.".into())
    }

    fn uninstall(&self) -> Result<String, String> {
        let pkg = package().ok_or("this LightCraft has no plugin package with it; remove the plugin in Logi Options+")?;
        run_tool("uninstall-plugin", &pkg)?;
        Ok("Logi Options+ asks to remove the plugin: confirm it there.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_versions_read_from_the_yaml() {
        assert_eq!(yaml_version("type: plugin4\nname: LightCraft\nversion: 0.2.0\n"), Some("0.2.0".into()));
        assert_eq!(yaml_version("version: '1.0'"), Some("1.0".into()));
        assert_eq!(yaml_version("name: x"), None);
        assert_eq!(yaml_version("version:"), None);
        // a file that isn't a package has no version (and doesn't crash)
        let dir = std::env::temp_dir().join(format!("lc-logi-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("bad.lplug4");
        std::fs::write(&f, b"not a zip").unwrap();
        assert_eq!(package_version(&f), None);
        std::fs::remove_dir_all(&dir).ok();
    }
}
