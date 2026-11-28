use crate::media;
use std::io::Read;
use std::path::PathBuf;

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const REPO_RELEASES_API: &str = "https://api.github.com/repos/penereya/Kadr/releases/latest";
pub const REPO_RELEASES_PAGE: &str = "https://github.com/penereya/Kadr/releases/latest";

#[derive(Clone, Debug)]
pub struct ReleaseInfo {
    pub tag: String,
    pub html_url: String,
    pub download_url: Option<String>,
    pub notes: String,
}

pub fn parse_semver(s: &str) -> Option<(u32, u32, u32)> {
    let clean = s.trim().trim_start_matches(['v', 'V']);
    let core = clean.split(['-', '+']).next()?.trim();
    if core.is_empty() {
        return None;
    }
    let mut parts = core.split('.');
    let major = parts.next()?.parse::<u32>().ok()?;
    let minor = parts.next().unwrap_or("0").parse::<u32>().ok()?;
    let patch = parts.next().unwrap_or("0").parse::<u32>().ok()?;
    Some((major, minor, patch))
}

pub fn is_newer_version(current: &str, latest: &str) -> bool {
    match (parse_semver(current), parse_semver(latest)) {
        (Some(cur), Some(lat)) => lat > cur,
        _ => false,
    }
}

fn read_json_string(bytes: &[u8], start_quote_idx: usize) -> Option<(String, usize)> {
    if bytes.get(start_quote_idx).copied() != Some(b'"') {
        return None;
    }
    let mut i = start_quote_idx + 1;
    let mut raw = Vec::new();
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\\' {
            i += 1;
            if i >= bytes.len() {
                break;
            }
            match bytes[i] {
                b'"' => raw.push(b'"'),
                b'\\' => raw.push(b'\\'),
                b'/' => raw.push(b'/'),
                b'n' => raw.push(b'\n'),
                b'r' => raw.push(b'\r'),
                b't' => raw.push(b'\t'),
                other => raw.push(other),
            }
            i += 1;
        } else if b == b'"' {
            let s = String::from_utf8_lossy(&raw).into_owned();
            return Some((s, i + 1));
        } else {
            raw.push(b);
            i += 1;
        }
    }
    None
}

fn find_json_string_values(json: &str, key: &str) -> Vec<String> {
    let bytes = json.as_bytes();
    let needle = format!("\"{}\"", key);
    let mut out = Vec::new();
    let mut search_from = 0;

    while let Some(rel) = json[search_from..].find(&needle) {
        let after_key = search_from + rel + needle.len();
        let mut i = after_key;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if bytes.get(i).copied() == Some(b':') {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if let Some((val, next_i)) = read_json_string(bytes, i) {
                out.push(val);
                search_from = next_i;
                continue;
            }
        }
        search_from = after_key;
    }

    out
}

pub fn parse_release_json(json: &str) -> Result<ReleaseInfo, String> {
    let tag = find_json_string_values(json, "tag_name")
        .into_iter()
        .next()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| "Не удалось определить версию релиза на GitHub".to_string())?;

    let html_url = find_json_string_values(json, "html_url")
        .into_iter()
        .find(|u| u.contains("/releases/tag/"))
        .unwrap_or_else(|| REPO_RELEASES_PAGE.to_string());

    let notes = find_json_string_values(json, "body")
        .into_iter()
        .next()
        .unwrap_or_default()
        .replace("\r\n", "\n")
        .trim()
        .to_string();

    let download_url = find_json_string_values(json, "browser_download_url")
        .into_iter()
        .find(|u| u.to_ascii_lowercase().ends_with("/kadr.exe"));

    Ok(ReleaseInfo {
        tag,
        html_url,
        download_url,
        notes,
    })
}

pub fn check_latest_release() -> Result<ReleaseInfo, String> {
    let output = media::hidden_command("curl.exe")
        .args([
            "-fsSL",
            "--connect-timeout",
            "6",
            "--max-time",
            "12",
            "-H",
            "User-Agent: Kadr-App",
            "-H",
            "Accept: application/vnd.github+json",
            REPO_RELEASES_API,
        ])
        .output()
        .map_err(|_| "Не удалось связаться с GitHub (проверь интернет или VPN)".to_string())?;

    if !output.status.success() {
        return Err("Не удалось связаться с GitHub (проверь интернет или VPN)".to_string());
    }

    let body = String::from_utf8_lossy(&output.stdout);
    parse_release_json(&body)
}

pub fn download_and_install_update(download_url: &str) -> Result<(), String> {
    let current_exe = std::env::current_exe()
        .map_err(|e| format!("Не удалось определить путь к программе: {}", e))?;
    let parent = current_exe
        .parent()
        .ok_or_else(|| "Не найдена папка программы".to_string())?;
    let exe_name = current_exe
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("Kadr.exe");

    let new_path = parent.join(format!("{}.new", exe_name));
    let old_path = parent.join(format!("{}.old", exe_name));

    let _ = std::fs::remove_file(&new_path);

    let output = media::hidden_command("curl.exe")
        .args([
            "-fsSL",
            "--connect-timeout",
            "10",
            "--max-time",
            "180",
            "-H",
            "User-Agent: Kadr-App",
            "-o",
        ])
        .arg(&new_path)
        .arg(download_url)
        .output()
        .map_err(|_| "Не удалось скачать обновление с GitHub".to_string())?;

    if !output.status.success() {
        let _ = std::fs::remove_file(&new_path);
        return Err("Не удалось скачать обновление с GitHub (проверь интернет или VPN)".to_string());
    }

    let valid = (|| -> bool {
        let Ok(meta) = std::fs::metadata(&new_path) else {
            return false;
        };
        if meta.len() < 1_000_000 {
            return false;
        }
        let Ok(mut f) = std::fs::File::open(&new_path) else {
            return false;
        };
        let mut magic = [0u8; 2];
        f.read_exact(&mut magic).is_ok() && &magic == b"MZ"
    })();

    if !valid {
        let _ = std::fs::remove_file(&new_path);
        return Err("Скачанный файл обновления повреждён".to_string());
    }

    if old_path.exists() {
        let _ = std::fs::remove_file(&old_path);
    }

    std::fs::rename(&current_exe, &old_path)
        .map_err(|e| format!("Не удалось заменить текущий файл: {}", e))?;

    if let Err(err) = std::fs::rename(&new_path, &current_exe) {
        let _ = std::fs::rename(&old_path, &current_exe);
        let _ = std::fs::remove_file(&new_path);
        return Err(format!("Не удалось завершить обновление: {}", err));
    }

    std::process::Command::new(&current_exe)
        .spawn()
        .map_err(|e| format!("Обновление установлено, но не удалось перезапустить: {}", e))?;

    Ok(())
}

pub fn open_url(url: &str) {
    let _ = media::hidden_command("cmd")
        .args(["/C", "start", "", url])
        .spawn();
}

pub fn cleanup_old_binary() {
    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(parent) = current_exe.parent() {
            let exe_name = current_exe
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("Kadr.exe");
            let old_path: PathBuf = parent.join(format!("{}.old", exe_name));
            if old_path.exists() {
                let _ = std::fs::remove_file(&old_path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_parsing_and_comparison() {
        assert_eq!(parse_semver("0.1.1"), Some((0, 1, 1)));
        assert_eq!(parse_semver("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_semver("V2.0"), Some((2, 0, 0)));
        assert_eq!(parse_semver("v1.2.3-beta"), Some((1, 2, 3)));
        assert_eq!(parse_semver("bad"), None);

        assert!(is_newer_version("0.1.1", "v0.1.2"));
        assert!(is_newer_version("0.1.1", "v0.2.0"));
        assert!(!is_newer_version("0.1.1", "v0.1.1"));
        assert!(!is_newer_version("0.1.2", "v0.1.1"));
    }

    #[test]
    fn release_json_picks_portable_exe_and_unescapes_notes() {
        let sample = r####"{
          "html_url": "https://github.com/penereya/Kadr/releases/tag/v0.1.2",
          "tag_name": "v0.1.2",
          "body": "### Что нового\r\n- Исправлен \"баг\" №1\n- Ускорен экспорт",
          "assets": [
            {
              "name": "KadrSetup.exe",
              "browser_download_url": "https://github.com/penereya/Kadr/releases/download/v0.1.2/KadrSetup.exe"
            },
            {
              "name": "Kadr.exe",
              "browser_download_url": "https://github.com/penereya/Kadr/releases/download/v0.1.2/Kadr.exe"
            }
          ]
        }"####;

        let info = parse_release_json(sample).expect("parse sample release");
        assert_eq!(info.tag, "v0.1.2");
        assert_eq!(
            info.html_url,
            "https://github.com/penereya/Kadr/releases/tag/v0.1.2"
        );
        assert_eq!(
            info.download_url.as_deref(),
            Some("https://github.com/penereya/Kadr/releases/download/v0.1.2/Kadr.exe")
        );
        assert!(info.notes.contains("Исправлен \"баг\" №1"));
        assert!(info.notes.contains('\n'));
    }
}
