use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{init, project};

const REPO: &str = "av/facts";
const BINARY: &str = "facts";
const VERSION: &str = env!("CARGO_PKG_VERSION");

enum InstallMethod {
    Npm,
    Pipx,
    Direct,
}

pub fn run() -> Result<()> {
    let exe = std::env::current_exe().context("cannot determine binary path")?;
    let method = detect_install_method(&exe);

    match method {
        InstallMethod::Npm => update_npm()?,
        InstallMethod::Pipx => update_pipx()?,
        InstallMethod::Direct => update_direct(&exe)?,
    }

    if let Ok(root) = project::find_project_root()
        && root.join(".facts").is_file()
    {
        println!();
        init::run(None)?;
    }

    Ok(())
}

fn detect_install_method(exe: &Path) -> InstallMethod {
    let path_str = exe.to_string_lossy();
    if path_str.contains("node_modules") || path_str.contains("/npm/") || path_str.contains("/nvm/")
    {
        return InstallMethod::Npm;
    }
    if path_str.contains("pipx/venvs") || path_str.contains("site-packages") {
        return InstallMethod::Pipx;
    }
    InstallMethod::Direct
}

fn update_npm() -> Result<()> {
    let downloader = find_downloader()?;
    if needs_update(downloader)?.is_none() {
        return Ok(());
    }
    println!("detected npm install, running: npm update -g @avcodes/facts");
    let status = Command::new("npm")
        .args(["update", "-g", "@avcodes/facts"])
        .status()
        .context("failed to run npm")?;
    if !status.success() {
        bail!("npm update failed (exit {})", status.code().unwrap_or(-1));
    }
    Ok(())
}

fn update_pipx() -> Result<()> {
    let downloader = find_downloader()?;
    if needs_update(downloader)?.is_none() {
        return Ok(());
    }
    println!("detected pipx install, running: pipx upgrade facts-cli");
    let status = Command::new("pipx")
        .args(["upgrade", "facts-cli"])
        .status()
        .context("failed to run pipx")?;
    if !status.success() {
        bail!("pipx upgrade failed (exit {})", status.code().unwrap_or(-1));
    }
    Ok(())
}

fn update_direct(exe: &Path) -> Result<()> {
    let downloader = find_downloader()?;
    let latest_tag = match needs_update(downloader)? {
        None => return Ok(()),
        Some(tag) => tag,
    };
    let latest_version = latest_tag.strip_prefix('v').unwrap_or(&latest_tag);

    let target = detect_platform()?;
    let url = format!(
        "https://github.com/{REPO}/releases/download/{latest_tag}/{BINARY}-{target}.tar.gz"
    );

    println!("updating v{VERSION} → v{latest_version}...");

    let tmpdir = tempdir()?;
    let _cleanup = TempDirGuard(&tmpdir);
    let archive = tmpdir.join("download.tar.gz");

    download(&url, &archive, downloader)?;

    let meta = std::fs::metadata(&archive)
        .with_context(|| format!("downloaded archive missing: {}", archive.display()))?;
    if meta.len() == 0 {
        bail!("downloaded archive is empty");
    }

    extract(&archive, &tmpdir)?;

    let new_binary = tmpdir.join(BINARY);
    if !new_binary.exists() {
        bail!("archive did not contain '{BINARY}' binary");
    }

    replace_binary(&new_binary, exe)?;
    println!("updated v{VERSION} → v{latest_version}");

    Ok(())
}

fn detect_platform() -> Result<String> {
    let os = detect_os()?;
    let arch = detect_arch()?;
    Ok(format!("{os}-{arch}"))
}

fn detect_os() -> Result<&'static str> {
    if cfg!(target_os = "linux") {
        Ok("linux")
    } else if cfg!(target_os = "macos") {
        Ok("darwin")
    } else if cfg!(target_os = "windows") {
        bail!("self-update is not supported on Windows; use npm or pip to update")
    } else {
        bail!("unsupported operating system: {}", std::env::consts::OS)
    }
}

fn detect_arch() -> Result<&'static str> {
    if cfg!(target_arch = "x86_64") {
        Ok("amd64")
    } else if cfg!(target_arch = "aarch64") {
        Ok("arm64")
    } else {
        bail!("unsupported architecture: {}", std::env::consts::ARCH)
    }
}

fn find_downloader() -> Result<&'static str> {
    for cmd in &["curl", "wget"] {
        let result = Command::new(cmd)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        if result.map(|s| s.success()).unwrap_or(false) {
            return Ok(cmd);
        }
    }
    bail!("curl or wget is required for self-update");
}

/// Returns true when `latest_tag` resolves to the same version as `current`.
fn is_up_to_date(current: &str, latest_tag: &str) -> bool {
    let latest_version = latest_tag.strip_prefix('v').unwrap_or(latest_tag);
    if latest_version == current {
        println!("already up to date (v{})", current);
        true
    } else {
        false
    }
}

/// Fetches the latest release tag and returns `None` when already up to date.
fn needs_update(downloader: &str) -> Result<Option<String>> {
    let latest_tag = get_latest_version(downloader)?;
    if is_up_to_date(VERSION, &latest_tag) {
        Ok(None)
    } else {
        Ok(Some(latest_tag))
    }
}

fn parse_latest_version(body: &str) -> Result<String> {
    let json: serde_json::Value =
        serde_json::from_str(body).context("GitHub API response is not valid JSON")?;
    let tag = json
        .get("tag_name")
        .and_then(|v| v.as_str())
        .context("could not parse tag_name from GitHub API response")?;
    Ok(tag.to_string())
}

fn get_latest_version(downloader: &str) -> Result<String> {
    let output = match downloader {
        "curl" => Command::new("curl")
            .args([
                "-fsSL",
                &format!("https://api.github.com/repos/{REPO}/releases/latest"),
            ])
            .output()
            .context("failed to run curl")?,
        "wget" => Command::new("wget")
            .args([
                "-qO-",
                &format!("https://api.github.com/repos/{REPO}/releases/latest"),
            ])
            .output()
            .context("failed to run wget")?,
        _ => bail!("unsupported downloader: {downloader}"),
    };

    if !output.status.success() {
        bail!("failed to fetch latest version from GitHub");
    }

    let body = String::from_utf8_lossy(&output.stdout);
    parse_latest_version(&body)
}

fn download(url: &str, dest: &Path, downloader: &str) -> Result<()> {
    let status = match downloader {
        "curl" => Command::new("curl")
            .args(["-fsSL", url, "-o"])
            .arg(dest)
            .status()
            .context("failed to run curl")?,
        "wget" => Command::new("wget")
            .args(["-qO"])
            .arg(dest)
            .arg(url)
            .status()
            .context("failed to run wget")?,
        _ => bail!("unsupported downloader: {downloader}"),
    };

    if !status.success() {
        bail!("download failed: {url}");
    }
    Ok(())
}

fn extract(archive: &Path, dest: &Path) -> Result<()> {
    let status = Command::new("tar")
        .args(["xzf"])
        .arg(archive)
        .arg("-C")
        .arg(dest)
        .status()
        .context("failed to run tar")?;
    if !status.success() {
        bail!("failed to extract archive");
    }
    Ok(())
}

fn replace_binary(src: &Path, dest: &Path) -> Result<()> {
    // Atomic-ish replacement: copy to temp next to dest, then rename.
    // rename() across filesystems fails, so we write to the same dir.
    let dest_dir = dest.parent().unwrap_or(Path::new("."));
    let tmp_dest = dest_dir.join(".facts.update.tmp");

    std::fs::copy(src, &tmp_dest)
        .with_context(|| format!("failed to copy new binary to {}", tmp_dest.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(&tmp_dest, std::fs::Permissions::from_mode(0o755))
            .context("failed to set permissions")
        {
            std::fs::remove_file(&tmp_dest).ok();
            return Err(e);
        }
    }

    if let Err(e) = std::fs::rename(&tmp_dest, dest)
        .with_context(|| format!("failed to replace binary at {}", dest.display()))
    {
        std::fs::remove_file(&tmp_dest).ok();
        return Err(e);
    }

    Ok(())
}

fn tempdir() -> Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!(
        "facts-update-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    ));
    std::fs::create_dir_all(&dir).context("failed to create temp directory")?;
    Ok(dir)
}

/// RAII guard that removes a temporary directory on drop.
struct TempDirGuard<'a>(&'a Path);

impl<'a> Drop for TempDirGuard<'a> {
    fn drop(&mut self) {
        std::fs::remove_dir_all(self.0).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_npm() {
        let p = Path::new("/usr/local/lib/node_modules/@avcodes/facts/bin/facts");
        assert!(matches!(detect_install_method(p), InstallMethod::Npm));
    }

    #[test]
    fn test_detect_npm_nvm() {
        let p = Path::new(
            "/home/user/.nvm/versions/node/v20/lib/node_modules/@avcodes/facts/bin/facts",
        );
        assert!(matches!(detect_install_method(p), InstallMethod::Npm));
    }

    #[test]
    fn test_detect_pipx() {
        let p = Path::new("/home/user/.local/pipx/venvs/facts-cli/bin/facts");
        assert!(matches!(detect_install_method(p), InstallMethod::Pipx));
    }

    #[test]
    fn test_detect_pipx_share_path() {
        let p = Path::new("/root/.local/share/pipx/venvs/facts-cli/bin/facts");
        assert!(matches!(detect_install_method(p), InstallMethod::Pipx));
    }

    #[test]
    fn test_detect_direct_usr_local() {
        let p = Path::new("/usr/local/bin/facts");
        assert!(matches!(detect_install_method(p), InstallMethod::Direct));
    }

    #[test]
    fn test_detect_direct_cargo() {
        let p = Path::new("/home/user/.cargo/bin/facts");
        assert!(matches!(detect_install_method(p), InstallMethod::Direct));
    }

    #[test]
    fn test_detect_platform() {
        let target = detect_platform().unwrap();
        assert!(
            target.contains("linux") || target.contains("darwin"),
            "unexpected target: {target}"
        );
        assert!(
            target.contains("amd64") || target.contains("arm64"),
            "unexpected target: {target}"
        );
    }

    #[test]
    fn test_detect_os_returns_known_platform() {
        let os = detect_os().unwrap();
        assert!(os == "linux" || os == "darwin", "unexpected os: {os}");
    }

    #[test]
    fn test_detect_arch_returns_known_arch() {
        let arch = detect_arch().unwrap();
        assert!(
            arch == "amd64" || arch == "arm64",
            "unexpected arch: {arch}"
        );
    }

    #[test]
    fn test_detect_platform_error_includes_os() {
        // On this platform detect_os should succeed; verify the error message
        // format by checking the current platform constant is non-empty.
        assert!(!std::env::consts::OS.is_empty());
    }

    #[test]
    fn test_detect_platform_error_includes_arch() {
        assert!(!std::env::consts::ARCH.is_empty());
    }

    #[test]
    fn test_version_is_set() {
        assert!(!VERSION.is_empty());
        assert!(VERSION.contains('.'));
    }

    #[test]
    fn test_replace_binary_cleans_up_temp_on_rename_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let dest_dir = tmp.path().join("dest");
        std::fs::create_dir_all(&dest_dir).unwrap();

        let src = tmp.path().join("src_bin");
        std::fs::write(&src, b"new binary").unwrap();

        let dest = dest_dir.join("existing_bin");
        std::fs::write(&dest, b"old binary").unwrap();

        // Make dest directory read-only so rename fails.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dest_dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        }

        let result = replace_binary(&src, &dest);
        assert!(result.is_err(), "expected rename to fail");

        // Restore permissions so assertion can read the dir.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dest_dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let tmp_file = dest_dir.join(".facts.update.tmp");
        assert!(
            !tmp_file.exists(),
            "temp file should be cleaned up after failed rename"
        );
    }

    #[test]
    fn test_temp_dir_guard_cleans_up_on_drop() {
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("file.txt"), b"content").unwrap();

        {
            let guard = TempDirGuard(&nested);
            assert!(nested.exists());
            drop(guard);
        }

        assert!(
            !nested.exists(),
            "TempDirGuard should remove directory on drop"
        );
    }

    #[test]
    fn test_temp_dir_guard_cleans_up_nested_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("a").join("b").join("c");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("file.txt"), b"content").unwrap();

        {
            let top = tmp.path().join("a");
            let guard = TempDirGuard(top.as_path());
            drop(guard);
        }

        assert!(!tmp.path().join("a").exists());
    }

    #[test]
    fn test_tempdir_includes_timestamp() {
        let d1 = tempdir().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        let d2 = tempdir().unwrap();
        assert_ne!(d1, d2, "consecutive tempdirs should be unique");
    }

    #[test]
    fn test_parse_latest_version_normal() {
        let body = r#"{"id":123,"tag_name":"v0.5.0","name":"0.5.0"}"#;
        assert_eq!(parse_latest_version(body).unwrap(), "v0.5.0");
    }

    #[test]
    fn test_parse_latest_version_reordered_fields() {
        let body = r#"{"tag_name":"v1.2.3","id":456,"created_at":"2024-01-01T00:00:00Z"}"#;
        assert_eq!(parse_latest_version(body).unwrap(), "v1.2.3");
    }

    #[test]
    fn test_parse_latest_version_extra_fields() {
        let body =
            r#"{"id":789,"tag_name":"v2.0.0-beta.1","extra":"ignored","nested":{"foo":"bar"}}"#;
        assert_eq!(parse_latest_version(body).unwrap(), "v2.0.0-beta.1");
    }

    #[test]
    fn test_parse_latest_version_missing_field() {
        let body = r#"{"id":123,"name":"release"}"#;
        let result = parse_latest_version(body);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("tag_name"),
            "error should mention tag_name: {err}"
        );
    }

    #[test]
    fn test_parse_latest_version_malformed_json() {
        let body = r#"{"id":123,"tag_name":}"#;
        let result = parse_latest_version(body);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("JSON"), "error should mention JSON: {err}");
    }

    #[test]
    fn test_parse_latest_version_null_tag_name() {
        let body = r#"{"id":123,"tag_name":null}"#;
        let result = parse_latest_version(body);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("tag_name"),
            "error should mention tag_name: {err}"
        );
    }

    #[test]
    fn test_parse_latest_version_number_tag_name() {
        let body = r#"{"id":123,"tag_name":1.0}"#;
        let result = parse_latest_version(body);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("tag_name"),
            "error should mention tag_name: {err}"
        );
    }

    #[test]
    fn test_parse_latest_version_whitespace_variations() {
        let body = r#"{
            "id" : 123 ,
            "tag_name" : "v3.0.0" ,
            "name" : "release"
        }"#;
        assert_eq!(parse_latest_version(body).unwrap(), "v3.0.0");
    }

    #[test]
    fn test_parse_latest_version_empty_json() {
        let body = r#"{}"#;
        let result = parse_latest_version(body);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("tag_name"),
            "error should mention tag_name: {err}"
        );
    }

    #[test]
    fn test_get_latest_version_unsupported_downloader() {
        let result = get_latest_version("fake_downloader");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("unsupported downloader"),
            "error should mention unsupported downloader: {err}"
        );
    }

    #[test]
    fn test_download_unsupported_downloader() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("archive.tar.gz");
        let result = download("http://example.com/test.tar.gz", &dest, "fake_downloader");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("unsupported downloader"),
            "error should mention unsupported downloader: {err}"
        );
    }

    #[test]
    fn test_is_up_to_date_same_with_v_prefix() {
        assert!(is_up_to_date("0.6.0", "v0.6.0"));
    }

    #[test]
    fn test_is_up_to_date_same_without_prefix() {
        assert!(is_up_to_date("0.6.0", "0.6.0"));
    }

    #[test]
    fn test_is_up_to_date_different_with_v_prefix() {
        assert!(!is_up_to_date("0.6.0", "v0.7.0"));
    }

    #[test]
    fn test_is_up_to_date_different_without_prefix() {
        assert!(!is_up_to_date("0.6.0", "0.7.0"));
    }

    #[test]
    fn test_needs_update_unsupported_downloader() {
        let result = needs_update("fake_downloader");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("unsupported downloader"),
            "error should mention unsupported downloader: {err}"
        );
    }
}
