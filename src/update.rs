//! 应用更新：检测 GitHub Release 最新版本，支持一键下载安装。

use serde::Deserialize;

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

const RELEASE_API: &str =
    "https://api.github.com/repos/RayYiHang/pixel-synthesis-calculator/releases/latest";

/// 最新 Release 信息（含当前平台资产）。
pub struct LatestRelease {
    /// 完整标签，如 `v1.2.0`。
    pub tag: String,
    /// 当前平台资产的下载地址。
    pub asset_url: String,
}

#[derive(Deserialize)]
struct ReleaseJson {
    tag_name: String,
    assets: Vec<AssetJson>,
}

#[derive(Deserialize)]
struct AssetJson {
    #[allow(dead_code)]
    name: String,
    #[allow(dead_code)]
    size: u64,
    browser_download_url: String,
}

/// 拉取最新 Release 并挑选当前平台的资产。
pub fn fetch_latest() -> Result<LatestRelease, String> {
    let release: ReleaseJson = ureq::get(RELEASE_API)
        .set("User-Agent", "pixel-synthesis-calculator")
        .set("Accept", "application/vnd.github+json")
        .timeout(std::time::Duration::from_secs(15))
        .call()
        .map_err(|e| format!("网络请求失败：{e}"))?
        .into_json()
        .map_err(|e| format!("解析响应失败：{e}"))?;

    let asset = pick_asset(&release.assets)
        .ok_or_else(|| "最新 Release 中没有当前平台的安装包".to_string())?;
    Ok(LatestRelease {
        tag: release.tag_name,
        asset_url: asset.browser_download_url.clone(),
    })
}

fn pick_asset(assets: &[AssetJson]) -> Option<&AssetJson> {
    match std::env::consts::OS {
        "windows" => assets
            .iter()
            .find(|a| a.name == "PixelSynthesisCalculator-windows-x64.zip"),
        "macos" => assets
            .iter()
            .find(|a| a.name == "PixelSynthesisCalculator-macos-universal.dmg"),
        "linux" => assets.iter().find(|a| {
            a.name.starts_with("pixel-synthesis-calculator_") && a.name.ends_with("_amd64.deb")
        }),
        _ => None,
    }
}

/// 比较语义化版本（仅数字段）：`latest` 是否比 `current` 新。
pub fn is_newer(latest: &str, current: &str) -> bool {
    fn parse(v: &str) -> Vec<u64> {
        v.trim_start_matches('v')
            .split('.')
            .map(|p| p.trim().parse().unwrap_or(0))
            .collect()
    }
    let (l, c) = (parse(latest), parse(current));
    for i in 0..l.len().max(c.len()) {
        let li = l.get(i).copied().unwrap_or(0);
        let ci = c.get(i).copied().unwrap_or(0);
        if li != ci {
            return li > ci;
        }
    }
    false
}

/// 下载资产到目标路径，返回写入字节数。
pub fn download_to(url: &str, dest: &std::path::Path) -> Result<u64, String> {
    use std::io::Write as _;
    let resp = ureq::get(url)
        .set("User-Agent", "pixel-synthesis-calculator")
        .timeout(std::time::Duration::from_secs(600))
        .call()
        .map_err(|e| format!("下载失败：{e}"))?;
    let mut reader = resp.into_reader();
    let mut file = std::fs::File::create(dest).map_err(|e| format!("创建文件失败：{e}"))?;
    let mut total = 0u64;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("下载中断：{e}"))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])
            .map_err(|e| format!("写入失败：{e}"))?;
        total += n as u64;
    }
    Ok(total)
}

/// 一键安装的结果。部分变体仅在特定平台构建中使用。
#[allow(dead_code)]
pub enum InstallOutcome {
    /// 新版本可执行文件已就位，应启动它并退出当前实例。
    Restart(std::path::PathBuf),
    /// 已完成（或已交由用户手动完成），展示提示信息。
    Message(String),
}

/// 平台分发：下载完成后按平台自动安装。
pub fn install_auto(asset: &std::path::Path) -> Result<InstallOutcome, String> {
    #[cfg(target_os = "windows")]
    {
        install_windows(asset).map(InstallOutcome::Restart)
    }
    #[cfg(target_os = "macos")]
    {
        install_macos(asset).map(InstallOutcome::Message)
    }
    #[cfg(target_os = "linux")]
    {
        install_linux(asset).map(InstallOutcome::Message)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        Err(format!("暂不支持在 {} 上自动更新", std::env::consts::OS))
    }
}

/// Windows：解压 zip 中的新 exe 并完成自替换，返回新版本可执行文件路径。
#[cfg(target_os = "windows")]
pub fn install_windows(zip_path: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let cur = std::env::current_exe().map_err(|e| e.to_string())?;
    let new_exe = cur.with_extension("exe.new");

    let file = std::fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut extracted = false;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        if entry.name().ends_with(".exe") {
            let mut out = std::fs::File::create(&new_exe).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
            extracted = true;
            break;
        }
    }
    if !extracted {
        return Err("更新包中没有 exe".into());
    }

    // 运行中的 exe 无法覆盖删除，但可以重命名让位。
    let old = cur.with_extension("exe.old");
    let _ = std::fs::remove_file(&old);
    std::fs::rename(&cur, &old).map_err(|e| format!("备份旧版本失败：{e}"))?;
    std::fs::rename(&new_exe, &cur).map_err(|e| format!("安装新版本失败：{e}"))?;
    Ok(cur)
}

/// Windows：启动前清理上次更新遗留的旧版本备份。
#[cfg(target_os = "windows")]
pub fn cleanup_old_backup() {
    if let Ok(cur) = std::env::current_exe() {
        let _ = std::fs::remove_file(cur.with_extension("exe.old"));
    }
}

#[cfg(not(target_os = "windows"))]
pub fn cleanup_old_backup() {}

/// macOS：挂载 dmg 并替换当前 .app（或退化为打开 dmg 手动安装）。
#[cfg(target_os = "macos")]
pub fn install_macos(dmg_path: &std::path::Path) -> Result<String, String> {
    use std::process::Command;

    let cur = std::env::current_exe().map_err(|e| e.to_string())?;
    // 找到当前运行所在的 .app 目录（开发裸跑二进制时不存在）。
    let app_dir = cur.ancestors().find(|p| {
        p.extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
    });

    let out = Command::new("hdiutil")
        .args(["attach", "-nobrowse", "-quiet", "-noautoopen"])
        .arg(dmg_path)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "挂载 dmg 失败：{}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }

    // 在挂载输出里找卷挂载点。
    let stdout = String::from_utf8_lossy(&out.stdout);
    let volume = stdout
        .lines()
        .filter_map(|line| line.split('\t').next_back())
        .find(|p| {
            std::path::Path::new(p.trim())
                .join("PixelSynthesisCalculator.app")
                .exists()
        })
        .map(|p| p.trim().to_string());

    let result = match (&volume, app_dir) {
        (Some(vol), Some(app)) => {
            let target = app.to_path_buf();
            let backup = app.with_extension("app.old");
            let _ = std::fs::remove_dir_all(&backup);
            let renamed = std::fs::rename(&target, &backup);
            let copy = Command::new("cp")
                .args(["-R", "-f"])
                .arg(std::path::Path::new(vol).join("PixelSynthesisCalculator.app"))
                .arg(app.parent().expect("app has parent"))
                .output()
                .map_err(|e| e.to_string())?;
            if copy.status.success() {
                let _ = std::fs::remove_dir_all(&backup);
                Ok("已更新应用，请重新启动".to_string())
            } else {
                // 复制失败则回滚旧版本。
                if renamed.is_ok() {
                    let _ = std::fs::rename(&backup, &target);
                }
                Err("替换 .app 失败（可能没有写入权限）".to_string())
            }
        }
        _ => {
            // 裸跑二进制或找不到卷：退化为直接打开 dmg 手动拖装。
            let _ = Command::new("open").arg(dmg_path).status();
            Ok("已打开更新镜像，请拖入「应用程序」完成安装".to_string())
        }
    };

    if let Some(vol) = &volume {
        let _ = Command::new("hdiutil")
            .args(["detach", "-quiet"])
            .arg(vol)
            .status();
    }
    result
}

/// Linux：把 deb 交给包管理器安装（pkexec 提权，失败则打开文件让用户手动装）。
#[cfg(target_os = "linux")]
pub fn install_linux(deb_path: &std::path::Path) -> Result<String, String> {
    use std::process::Command;
    let status = Command::new("pkexec")
        .args(["dpkg", "-i"])
        .arg(deb_path)
        .status();
    match status {
        Ok(s) if s.success() => Ok("已安装新版本，请重启应用".to_string()),
        _ => {
            let _ = Command::new("xdg-open")
                .arg(deb_path.parent().expect("deb has parent"))
                .status();
            Err(format!(
                "自动安装未完成，请手动执行：sudo dpkg -i {}",
                deb_path.display()
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::is_newer;

    #[test]
    fn version_compare() {
        assert!(is_newer("1.2.0", "1.1.0"));
        assert!(is_newer("v1.10.0", "1.9.9"));
        assert!(is_newer("2.0", "1.99.99"));
        assert!(!is_newer("1.1.0", "1.1.0"));
        assert!(!is_newer("1.0.9", "1.1.0"));
    }
}
