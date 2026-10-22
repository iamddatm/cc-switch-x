use crate::services::apply::ApplyReport;

pub mod ledger;
pub mod mcp;
pub mod paths;
pub mod prompt;
pub mod rules;
pub mod skills;
pub mod subagents;

/// 整文件型资产（rules / subagents）的落盘结果。
/// 「什么都没写」由 `ApplyReport::skipped` 表达，不再单列变体（两处真相会打架）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Deployed {
    Written(ApplyReport),
    /// 目标位置上是不认识的内容，一个字节都没动
    Foreign,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Removed {
    Ours,
    /// 不是我们落的文件（内置的、用户自己写的、或用户改过的自家文件）
    Foreign,
    Absent,
}
