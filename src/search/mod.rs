mod core;
mod rerank;
mod retrieve;
mod rewrite;
mod traditional_search;
mod utils; // 添加新模块

pub mod embedding;

// 重新导出公共接口
pub use core::{RecommendCrate, SearchModule, SearchSortCriteria};
pub use rerank::rerank_crates;
pub use retrieve::retrive_crates;
pub use rewrite::{extract_keywords_from_query, rewrite_query};
pub use traditional_search::TraditionalSearchModule; // 导出传统搜索模块
