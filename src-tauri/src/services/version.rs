//! 版本号的解析与比较，只服务一件事：判断「上游是不是比本机这份新」。
//!
//! 不做 semver 的全部事，也刻意不引 `regex`：CLI 的 `--version` 输出形状太杂
//! （`2.1.10 (Claude Code)`、`codex-cli v0.42.0`），能稳定取出第一个数字段就够了；
//! 取不出来就报「不知道」，绝不猜成「已最新」或「可更新」。

use std::cmp::Ordering;

/// 已解析的版本号。`prerelease` 为空表示正式版；`+` 后面的构建元数据按 semver 既不参与
/// 比较也不参与相等，所以不存进结构体（存了会让 derive 出来的 `Eq` 与手写的 `Ord` 打架）。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Version {
    core: [u64; 3],
    prerelease: Vec<String>,
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    /// 按 semver 优先级比较：正式版 > 同核心的预发布版。
    fn cmp(&self, other: &Self) -> Ordering {
        self.core
            .cmp(&other.core)
            .then_with(|| compare_prerelease(&self.prerelease, &other.prerelease))
    }
}

/// 预发布段比较：无预发布 > 有预发布；逐标识符比，数字按数值、其余按字典序；
/// 前面都相等时短的更小（semver B.11）。
fn compare_prerelease(a: &[String], b: &[String]) -> Ordering {
    match (a.is_empty(), b.is_empty()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => {
            for (left, right) in a.iter().zip(b.iter()) {
                let order = compare_identifier(left, right);
                if order != Ordering::Equal {
                    return order;
                }
            }
            a.len().cmp(&b.len())
        }
    }
}

fn compare_identifier(a: &str, b: &str) -> Ordering {
    match (a.parse::<u64>(), b.parse::<u64>()) {
        (Ok(left), Ok(right)) => left.cmp(&right),
        // 数字标识符优先级低于字母标识符（semver 原文：alphanumerics always have
        // lower precedence than numeric）
        (Ok(_), Err(_)) => Ordering::Less,
        (Err(_), Ok(_)) => Ordering::Greater,
        (Err(_), Err(_)) => a.cmp(b),
    }
}

/// 从 `--version` 的输出里取第一个像版本号的 token（`v` 前缀会被剥掉）。
pub fn first_version_token(text: &str) -> Option<&str> {
    text.split_whitespace().find_map(|token| {
        let body = token.strip_prefix(['v', 'V']).unwrap_or(token);
        let head = body.split(['-', '+']).next().unwrap_or(body);
        let parts: Vec<&str> = head.split('.').collect();
        if parts.len() < 2 || parts.iter().any(|part| part.is_empty() || part.parse::<u64>().is_err())
        {
            return None;
        }
        Some(body)
    })
}

/// 解析 `x.y.z[-prerelease][+build]`。核心段得是 2~3 个纯数字（`first_version_token` 已经
/// 保证 2 段起、且每段可解析），四段不是 semver，宁可不判。
fn parse_version(text: &str) -> Option<Version> {
    let token = first_version_token(text)?;
    // 构建元数据必须先剥掉，否则 `+` 后面的内容会被当成预发布段。
    let head = token.split('+').next().unwrap_or(token);
    let (core, prerelease) = match head.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (head, None),
    };
    let parts: Vec<&str> = core.split('.').collect();
    if parts.len() > 3 {
        return None;
    }
    let mut core_numbers = [0_u64; 3];
    for (index, part) in parts.iter().enumerate() {
        core_numbers[index] = part.parse().ok()?;
    }
    Some(Version {
        core: core_numbers,
        prerelease: prerelease
            .map(|raw| raw.split('.').map(str::to_string).collect())
            .unwrap_or_default(),
    })
}

/// 开发构建：`+238.gdeadbeef` 这类 git describe 戳，或带 canary/nightly/dev/dirty/snapshot。
/// 这些版本和 registry 的 `latest` 不同源，比出来的「旧」是假信号。
fn is_dev_build(text: &str) -> bool {
    let lower = text.to_lowercase();
    if ["canary", "nightly", "snapshot", "dirty", "dev"]
        .iter()
        .any(|marker| lower.contains(marker))
    {
        return true;
    }
    // git describe 形态：<领先提交数>.g<短哈希>，可能直接落在核心段里（1.2.3+238.gabc）
    let Some((_, build)) = lower.split_once('+') else {
        return false;
    };
    let parts: Vec<&str> = build.split('.').collect();
    parts.len() == 2
        && parts[0].parse::<u64>().is_ok()
        && parts[1].strip_prefix('g').is_some_and(|hash| {
            !hash.is_empty() && hash.len() >= 4 && hash.chars().all(|c| c.is_ascii_hexdigit())
        })
}

/// 只有「两边都解析得出来、本机这份不是开发构建、上游严格更新」才算可更新。
/// 任何一环拿不准都返回 false —— 报错位比误催用户升级要好。
pub fn is_update_available(installed: &str, latest: &str) -> bool {
    if is_dev_build(installed) {
        return false;
    }
    let (Some(current), Some(candidate)) = (parse_version(installed), parse_version(latest))
    else {
        return false;
    };
    candidate > current
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_version_out_of_noisy_cli_output() {
        assert_eq!(first_version_token("2.1.10 (Claude Code)"), Some("2.1.10"));
        assert_eq!(first_version_token("codex-cli v0.42.0"), Some("0.42.0"));
        assert_eq!(first_version_token("gemini version 1.2.3"), Some("1.2.3"));
        assert_eq!(first_version_token("no digits here"), None);
    }

    #[test]
    fn parses_core_and_prerelease() {
        let parsed = parse_version("1.2.3-beta.7+exp.sha.5114f85").unwrap();
        assert_eq!(parsed.core, [1, 2, 3]);
        assert_eq!(parsed.prerelease, vec!["beta".to_string(), "7".to_string()]);
        // 构建元数据不入库：`1.2.3+a` 与 `1.2.3+b` 必须既相等又判等，才和 semver 一致。
        assert_eq!(
            parse_version("1.2.3+a").unwrap(),
            parse_version("1.2.3+b").unwrap()
        );
        // 段数不足也认：1.2 视为 1.2.0
        assert_eq!(parse_version("1.2").unwrap().core, [1, 2, 0]);
        // 四段核心不是 semver，宁可不判
        assert!(parse_version("1.2.3.4").is_none());
    }

    #[test]
    fn orders_versions_the_way_semver_does() {
        assert!(parse_version("1.10.0").unwrap() > parse_version("1.9.9").unwrap());
        assert!(parse_version("1.2.3").unwrap() > parse_version("1.2.3-beta.1").unwrap());
        assert!(parse_version("1.2.3-beta.2").unwrap() > parse_version("1.2.3-beta.1").unwrap());
        assert!(parse_version("1.2.3-alpha").unwrap() > parse_version("1.2.3-1").unwrap());
        assert!(parse_version("1.2.3-beta.1").unwrap() > parse_version("1.2.3-beta").unwrap());
    }

    #[test]
    fn dev_builds_are_never_marked_outdated() {
        assert!(is_dev_build("0.22.1+238.gdeadbeef"));
        assert!(is_dev_build("1.2.3-canary.1"));
        assert!(is_dev_build("1.2.3-dirty"));
        assert!(!is_dev_build("1.2.3"));
        assert!(!is_update_available("0.22.1+238.gdeadbeef", "9.9.9"));
    }

    #[test]
    fn update_needs_both_sides_parseable_and_strictly_newer() {
        assert!(is_update_available("1.2.3", "1.2.4"));
        assert!(!is_update_available("1.2.4", "1.2.4"));
        assert!(!is_update_available("1.2.5", "1.2.4"));
        assert!(!is_update_available("读不确定的输出", "1.2.4"));
        assert!(!is_update_available("1.2.4", ""));
    }
}
