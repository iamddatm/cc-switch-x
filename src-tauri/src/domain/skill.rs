use super::frontmatter::split_frontmatter;
use serde::Deserialize;

/// `SKILL.md` frontmatter 里我们关心的字段。描述只用于列表展示，解析失败不能拦住资产。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkillMetadata {
    pub name: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SkillFrontmatter {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

/// 宽松解析：没有 frontmatter、YAML 损坏、字段缺失都返回空元数据。
pub fn parse_metadata(raw: &str) -> SkillMetadata {
    let Ok(parts) = split_frontmatter(raw) else {
        return SkillMetadata::default();
    };
    let Some(yaml) = parts.yaml else {
        return SkillMetadata::default();
    };
    let Ok(meta) = serde_yaml::from_str::<SkillFrontmatter>(&yaml) else {
        return SkillMetadata::default();
    };
    SkillMetadata {
        name: non_empty(meta.name),
        description: non_empty(meta.description),
    }
}

fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_name_and_description_but_tolerates_anything_else() {
        let meta = parse_metadata(
            "---\nname: lark-wiki\ndescription: 查飞书知识库\nunknown: 1\n---\n\n正文\n",
        );
        assert_eq!(meta.name.as_deref(), Some("lark-wiki"));
        assert_eq!(meta.description.as_deref(), Some("查飞书知识库"));
    }

    #[test]
    fn broken_frontmatter_is_not_an_error() {
        assert_eq!(parse_metadata("没有 frontmatter"), SkillMetadata::default());
        assert_eq!(
            parse_metadata("---\n: : :\n---\n"),
            SkillMetadata::default()
        );
    }
}
