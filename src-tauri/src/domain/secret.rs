use std::ops::Range;

pub const REFERENCE_PREFIX: &str = "${secret:";
pub const REFERENCE_SUFFIX: &str = "}";

/// canonical 文件里对一条密钥的引用：`scope` 形如 `mcp.<资产 id>`，`key` 是环境变量名。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SecretRef {
    pub scope: String,
    pub key: String,
}

impl std::fmt::Display for SecretRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.scope, self.key)
    }
}

/// 文本里的一处 `${secret:…}` 候选，`body` 是花括号内的原文。
/// 解析失败要由调用方决定怎么报错，所以这里不预先丢掉坏引用。
pub struct Candidate<'a> {
    pub span: Range<usize>,
    pub body: &'a str,
}

pub fn reference(scope: &str, key: &str) -> String {
    format!("${{secret:{scope}.{key}}}")
}

/// 按**最后一个点**切分：资产 ID 本身允许含点（见 store 的 is_safe_id），
/// 而密钥名是环境变量名，不含点。scope 至少要 `kind.id` 两段，避免把残缺引用当合法。
pub fn parse_reference(raw: &str) -> Option<SecretRef> {
    let body = raw
        .strip_prefix(REFERENCE_PREFIX)?
        .strip_suffix(REFERENCE_SUFFIX)?;
    parse_reference_body(body)
}

/// 直接解析花括号内的 `scope.key` 原文，供已经拿到 body 的调用方使用。
pub fn parse_reference_body(body: &str) -> Option<SecretRef> {
    if body.chars().any(char::is_whitespace) {
        return None;
    }
    let (scope, key) = body.rsplit_once('.')?;
    if key.is_empty() || !scope.contains('.') {
        return None;
    }
    Some(SecretRef {
        scope: scope.to_string(),
        key: key.to_string(),
    })
}

pub fn scan_candidates(text: &str) -> Vec<Candidate<'_>> {
    let mut out = Vec::new();
    let mut cursor = 0usize;
    while let Some(offset) = text[cursor..].find(REFERENCE_PREFIX) {
        let start = cursor + offset;
        let body_start = start + REFERENCE_PREFIX.len();
        let Some(suffix_offset) = text[body_start..].find(REFERENCE_SUFFIX) else {
            break;
        };
        out.push(Candidate {
            span: start..body_start + suffix_offset + REFERENCE_SUFFIX.len(),
            body: &text[body_start..body_start + suffix_offset],
        });
        cursor = body_start + suffix_offset + REFERENCE_SUFFIX.len();
    }
    out
}

pub fn find_references(text: &str) -> Vec<SecretRef> {
    scan_candidates(text)
        .into_iter()
        .filter_map(|candidate| parse_reference_body(candidate.body))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_roundtrip() {
        let text = reference("mcp.playwright", "API_TOKEN");
        assert_eq!(text, "${secret:mcp.playwright.API_TOKEN}");
        let parsed = parse_reference(&text).unwrap();
        assert_eq!(parsed.scope, "mcp.playwright");
        assert_eq!(parsed.key, "API_TOKEN");
    }

    #[test]
    fn finds_every_reference_in_a_url() {
        let url = "https://api.tavily.com/mcp?tavilyApiKey=${secret:mcp.tavily.tavilyApiKey}&v=2";
        let refs = find_references(url);
        assert_eq!(refs.len(), 1);
        assert_eq!(
            refs[0],
            SecretRef {
                scope: "mcp.tavily".into(),
                key: "tavilyApiKey".into()
            }
        );
    }

    #[test]
    fn rejects_incomplete_reference() {
        assert!(parse_reference("${secret:nocolon}").is_none());
        assert!(parse_reference("${secret:a.b}").is_none()); // scope 不足 kind.id 两段
        assert!(parse_reference("${secret:mcp.pw.}").is_none()); // key 为空
        assert!(parse_reference("mcp.pw.TOKEN").is_none()); // 缺 ${} 包裹
        assert!(parse_reference("${secret:mcp.pw.API TOKEN}").is_none()); // 含空白
    }

    #[test]
    fn splits_on_the_last_dot_so_dotted_asset_ids_work() {
        // 资产 ID 本身允许 '.'（store 的 is_safe_id），故按最后一个点切：scope=mcp.a.b, key=TOKEN
        assert_eq!(
            parse_reference("${secret:mcp.a.b.TOKEN}").unwrap(),
            SecretRef {
                scope: "mcp.a.b".into(),
                key: "TOKEN".into()
            }
        );
    }

    #[test]
    fn scan_keeps_broken_candidates_so_callers_can_fail() {
        let text = r#"{"a":"${secret:mcp.x.T1}","b":"${secret:broken}"}"#;
        let found = scan_candidates(text);
        assert_eq!(found.len(), 2, "坏引用不能被静默丢掉");
        assert_eq!(found[0].body, "mcp.x.T1");
        assert_eq!(found[1].body, "broken");
        assert_eq!(find_references(text).len(), 1);
    }
}
