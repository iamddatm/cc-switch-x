use super::DomainError;

pub struct Frontmatter {
    pub yaml: Option<String>,
    pub body: String,
}

/// 支持 UTF-8 BOM 与 CRLF；返回的 body 统一为 LF。
pub fn split_frontmatter(raw: &str) -> Result<Frontmatter, DomainError> {
    let raw = raw.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    if !raw.starts_with("---\n") {
        return Ok(Frontmatter {
            yaml: None,
            body: raw,
        });
    }
    let rest = &raw[4..];
    let Some(end) = rest.find("\n---") else {
        return Err(DomainError::UnclosedFrontmatter);
    };
    let yaml = rest[..end].to_string();
    let body = rest[end + 4..].trim_start_matches('\n');
    Ok(Frontmatter {
        yaml: Some(yaml),
        body: body.to_string(),
    })
}
