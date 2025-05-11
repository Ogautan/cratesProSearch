use crate::search::core::{RecommendCrate, SearchSortCriteria};
use std::env;
use tokio_postgres::Client as PgClient;

/// 传统搜索模块 - 不使用任何LLM技术，完全基于关键词匹配和经典排序算法
pub struct TraditionalSearchModule<'a> {
    pg_client: &'a PgClient,
    table_name: String,
}

impl<'a> TraditionalSearchModule<'a> {
    pub async fn new(pg_client: &'a PgClient) -> Self {
        let table_name = env::var("TABLE_NAME").unwrap_or_else(|_| "crates".to_string());
        TraditionalSearchModule {
            pg_client,
            table_name,
        }
    }

    /// 传统搜索函数 - 使用多种经典IR技术而不是LLM
    pub async fn search(
        &self,
        query: &str,
        sort_by: SearchSortCriteria,
    ) -> Result<Vec<RecommendCrate>, Box<dyn std::error::Error>> {
        // 1. 查询预处理
        let processed_query = self.preprocess_query(query);

        // 2. 执行多种搜索策略并合并结果
        let mut all_results = Vec::new();

        // 2.1 精确匹配搜索 - 高优先级
        let exact_results = self.exact_match_search(&processed_query).await?;
        for result in exact_results {
            all_results.push((result, 1.0)); // 精确匹配有最高权重
        }

        // 2.2 前缀匹配搜索 - 中优先级
        let prefix_results = self.prefix_match_search(&processed_query).await?;
        for result in prefix_results {
            // 检查结果是否已经在all_results中
            if !all_results.iter().any(|(r, _)| r.id == result.id) {
                all_results.push((result, 0.8)); // 前缀匹配有中等权重
            }
        }

        // 2.3 全文搜索 - 低优先级
        let fulltext_results = self.fulltext_search(&processed_query).await?;
        for result in fulltext_results {
            if !all_results.iter().any(|(r, _)| r.id == result.id) {
                all_results.push((result, 0.6)); // 全文搜索有较低权重
            }
        }

        // 3. 结果排序
        let mut final_results = self.rank_results(all_results, sort_by);

        // 4. 只返回前100个结果
        if final_results.len() > 100 {
            final_results.truncate(100);
        }

        Ok(final_results)
    }

    /// 查询预处理 - 清理和标准化查询
    fn preprocess_query(&self, query: &str) -> String {
        // 移除特殊字符，统一为小写
        let query = query.to_lowercase();

        // 停用词过滤
        let stopwords = [
            "a", "an", "the", "in", "on", "at", "by", "for", "with", "is", "are", "was", "were",
            "of", "to", "from", "and", "or", "but", "how", "what", "which", "who", "when", "where",
            "why", "can", "could", "need", "want", "rust", "crate", "library", "package",
        ];

        let mut processed = query.clone();

        // 处理中文查询
        let has_chinese = query.chars().any(|c| '\u{4e00}' <= c && c <= '\u{9fff}');

        if has_chinese {
            // 移除中文常见问句词
            let chinese_question_words = [
                "如何",
                "怎么",
                "什么",
                "哪个",
                "为什么",
                "能否",
                "可以",
                "请问",
                "有没有",
            ];
            for word in &chinese_question_words {
                processed = processed.replace(word, " ");
            }
        } else {
            // 英文查询的停用词处理
            for word in &stopwords {
                // 确保只替换完整的单词
                processed = processed
                    .replace(&format!(" {} ", word), " ")
                    .replace(&format!(" {}", word), "")
                    .replace(&format!("{} ", word), "");
            }
        }

        // 规范化空白字符
        let processed = processed
            .split_whitespace()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ");

        if processed.is_empty() {
            return query; // 如果处理后为空，则返回原始查询
        }

        processed
    }

    /// 精确匹配搜索 - 搜索名称和描述中出现的完整查询词
    async fn exact_match_search(
        &self,
        query: &str,
    ) -> Result<Vec<RecommendCrate>, Box<dyn std::error::Error>> {
        let statement = format!(
            "SELECT id, name, description
             FROM {}
             WHERE name ILIKE $1 OR name ILIKE $2 OR description ILIKE $3
             LIMIT 50",
            self.table_name
        );

        let exact_pattern = format!("%{}%", query);
        let word_pattern = query.split_whitespace().collect::<Vec<_>>().join("%");
        let word_pattern = format!("%{}%", word_pattern);

        let rows = self
            .pg_client
            .query(&statement, &[&exact_pattern, &word_pattern, &exact_pattern])
            .await?;

        let mut results = Vec::new();

        for row in rows {
            let id: String = row.get("id");
            let name: String = row.get("name");
            let description: String = row.get("description");

            // 计算一个基于匹配位置的简单分数
            let name_lower = name.to_lowercase();
            let desc_lower = description.to_lowercase();
            let query_lower = query.to_lowercase();

            let mut score = 0.0;

            // 名称中精确匹配得高分
            if name_lower == query_lower {
                score = 1.0;
            } else if name_lower.contains(&query_lower) {
                score = 0.9;
            } else if desc_lower.contains(&query_lower) {
                score = 0.7;
            } else {
                // 检查部分匹配
                let query_words: Vec<_> = query_lower.split_whitespace().collect();
                for word in &query_words {
                    if name_lower.contains(word) {
                        score += 0.2 / query_words.len() as f32;
                    } else if desc_lower.contains(word) {
                        score += 0.1 / query_words.len() as f32;
                    }
                }
            }

            results.push(RecommendCrate {
                id,
                name,
                description,
                rank: score,
                vector_score: 0.0, // 不使用向量得分
                final_score: score,
            });
        }

        // 按得分排序
        results.sort_by(|a, b| b.rank.partial_cmp(&a.rank).unwrap());

        Ok(results)
    }

    /// 前缀匹配搜索 - 使用PostgreSQL的前缀匹配功能
    async fn prefix_match_search(
        &self,
        query: &str,
    ) -> Result<Vec<RecommendCrate>, Box<dyn std::error::Error>> {
        // 将查询分解为单词并构建tsquery
        let words: Vec<_> = query.split_whitespace().collect();
        if words.is_empty() {
            return Ok(Vec::new());
        }

        let prefix_terms: Vec<_> = words.iter().map(|w| format!("{}:*", w)).collect();
        let tsquery = prefix_terms.join(" | ");

        let statement = format!(
            "SELECT id, name, description, ts_rank(tsv, to_tsquery($1)) AS rank
             FROM {}
             WHERE tsv @@ to_tsquery($1)
             ORDER BY rank DESC
             LIMIT 100",
            self.table_name
        );

        let rows = self.pg_client.query(&statement, &[&tsquery]).await?;

        let mut results = Vec::new();

        for row in rows {
            let id: String = row.get("id");
            let name: String = row.get("name");
            let description: String = row.get("description");
            let rank: f32 = row.get("rank");

            results.push(RecommendCrate {
                id,
                name,
                description,
                rank,
                vector_score: 0.0,
                final_score: rank,
            });
        }

        Ok(results)
    }

    /// 全文搜索 - 使用PostgreSQL的全文搜索功能
    async fn fulltext_search(
        &self,
        query: &str,
    ) -> Result<Vec<RecommendCrate>, Box<dyn std::error::Error>> {
        // 使用plainto_tsquery，更宽松的全文匹配
        let statement = format!(
            "SELECT id, name, description, ts_rank(tsv, plainto_tsquery($1)) AS rank
             FROM {}
             WHERE tsv @@ plainto_tsquery($1)
             ORDER BY rank DESC
             LIMIT 150",
            self.table_name
        );

        let rows = self.pg_client.query(&statement, &[&query]).await?;

        let mut results = Vec::new();

        for row in rows {
            let id: String = row.get("id");
            let name: String = row.get("name");
            let description: String = row.get("description");
            let rank: f32 = row.get("rank");

            results.push(RecommendCrate {
                id,
                name,
                description,
                rank,
                vector_score: 0.0,
                final_score: rank,
            });
        }

        Ok(results)
    }

    /// 对搜索结果进行排序
    fn rank_results(
        &self,
        results: Vec<(RecommendCrate, f32)>,
        sort_criteria: SearchSortCriteria,
    ) -> Vec<RecommendCrate> {
        let mut final_results = Vec::new();

        for (mut crate_item, weight) in results {
            // 应用排序策略的权重

            crate_item.final_score = crate_item.rank * weight;

            final_results.push(crate_item);
        }

        // 根据最终得分排序
        final_results.sort_by(|a, b| b.final_score.partial_cmp(&a.final_score).unwrap());

        final_results
    }
}
