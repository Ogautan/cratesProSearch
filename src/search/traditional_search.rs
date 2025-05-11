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
        let processed_queries = self.preprocess_query(query);
        println!("传统处理后的查询: {:?}", processed_queries);

        // 2. 执行多种搜索策略并合并结果
        let mut all_results = Vec::new();

        // 为每个处理后的查询变体执行搜索
        for processed_query in &processed_queries {
            // 2.1 精确匹配搜索 - 高优先级
            let exact_results = self.exact_match_search(processed_query).await?;
            for result in exact_results {
                // 避免重复添加
                if !all_results
                    .iter()
                    .any(|(r, _): &(RecommendCrate, f32)| r.id == result.id)
                {
                    all_results.push((result, 1.0)); // 精确匹配有最高权重
                }
            }

            // 2.2 前缀匹配搜索 - 中优先级
            let prefix_results = self.prefix_match_search(processed_query).await?;
            for result in prefix_results {
                // 检查结果是否已经在all_results中
                if !all_results
                    .iter()
                    .any(|(r, _): &(RecommendCrate, f32)| r.id == result.id)
                {
                    all_results.push((result, 0.8)); // 前缀匹配有中等权重
                }
            }

            // 2.3 全文搜索 - 低优先级
            let fulltext_results = self.fulltext_search(processed_query).await?;
            for result in fulltext_results {
                if !all_results
                    .iter()
                    .any(|(r, _): &(RecommendCrate, f32)| r.id == result.id)
                {
                    all_results.push((result, 0.6)); // 全文搜索有较低权重
                }
            }
        }

        // 如果结果太少，尝试完全的全文搜索
        if all_results.len() < 10 && !processed_queries.is_empty() {
            let fulltext_results = self.advanced_fulltext_search(query).await?;
            for result in fulltext_results {
                if !all_results
                    .iter()
                    .any(|(r, _): &(RecommendCrate, f32)| r.id == result.id)
                {
                    all_results.push((result, 0.5)); // 完全全文搜索权重较低
                }
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

    /// 改进的查询预处理 - 返回多个可能的查询变体
    fn preprocess_query(&self, query: &str) -> Vec<String> {
        let mut query_variants = Vec::new();
        let original_query = query.trim();

        // 添加原始查询（如果非空）
        if !original_query.is_empty() {
            query_variants.push(original_query.to_string());
        }

        // 统一为小写进行处理
        let query = original_query.to_lowercase();

        // 检测查询语言
        let has_chinese = query.chars().any(|c| '\u{4e00}' <= c && c <= '\u{9fff}');
        let has_english = query.chars().any(|c| c.is_ascii_alphabetic());

        // 停用词列表
        let english_stopwords = [
            "a",
            "an",
            "the",
            "in",
            "on",
            "at",
            "by",
            "for",
            "with",
            "is",
            "are",
            "was",
            "were",
            "of",
            "to",
            "from",
            "and",
            "or",
            "but",
            "how",
            "what",
            "which",
            "who",
            "when",
            "where",
            "why",
            "can",
            "could",
            "need",
            "want",
            "rust",
            "crate",
            "library",
            "package",
            "help",
            "please",
            "find",
            "looking",
            "search",
            "get",
            "use",
            "using",
            "implement",
        ];

        let chinese_stopwords = [
            "如何",
            "怎么",
            "什么",
            "哪个",
            "为什么",
            "能否",
            "可以",
            "请问",
            "有没有",
            "想要",
            "需要",
            "使用",
            "寻找",
            "查找",
            "搜索",
            "获取",
            "我要",
            "帮我",
            "推荐",
        ];

        // 短语变体处理
        let mut processed = query.clone();

        // 中文查询处理
        if has_chinese {
            // 移除中文停用词
            for word in &chinese_stopwords {
                processed = processed.replace(word, " ");
            }

            // 提取中文关键字
            query_variants.push(processed.trim().to_string());

            // 如果是中英混合，也提取英文部分
            if has_english {
                let english_parts: String = processed
                    .chars()
                    .filter(|&c| c.is_ascii_alphabetic() || c.is_ascii_whitespace())
                    .collect();

                if !english_parts.trim().is_empty() {
                    query_variants.push(english_parts.trim().to_string());
                }
            }
        }

        // 英文查询处理
        if has_english {
            let mut processed = query.clone();

            // 英文查询的停用词处理
            for word in &english_stopwords {
                // 确保只替换完整的单词
                processed = processed
                    .replace(&format!(" {} ", word), " ")
                    .replace(&format!(" {}", word), "")
                    .replace(&format!("{} ", word), "");
            }

            // 规范化空白字符
            let cleaned = processed
                .split_whitespace()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" ");

            if !cleaned.is_empty() && !query_variants.contains(&cleaned) {
                query_variants.push(cleaned);
            }

            // 添加关键词组合变体，特别是对于短语查询
            let words: Vec<&str> = processed.split_whitespace().collect();
            if words.len() >= 2 {
                // 二词组合
                for i in 0..words.len() - 1 {
                    let bigram = format!("{} {}", words[i], words[i + 1]);
                    if !query_variants.contains(&bigram) {
                        query_variants.push(bigram);
                    }
                }

                // 如果原始查询很长，添加三词组合
                if words.len() >= 3 {
                    for i in 0..words.len() - 2 {
                        let trigram = format!("{} {} {}", words[i], words[i + 1], words[i + 2]);
                        if !query_variants.contains(&trigram) {
                            query_variants.push(trigram);
                        }
                    }
                }

                // 对于特别长的查询，分别添加前半部分和后半部分
                if words.len() >= 4 {
                    let mid = words.len() / 2;
                    let first_half = words[..mid].join(" ");
                    let second_half = words[mid..].join(" ");

                    if !first_half.is_empty() && !query_variants.contains(&first_half) {
                        query_variants.push(first_half);
                    }
                    if !second_half.is_empty() && !query_variants.contains(&second_half) {
                        query_variants.push(second_half);
                    }
                }
            }
        }

        // 确保至少有一个查询变体
        if query_variants.is_empty() {
            query_variants.push(query);
        }

        query_variants
    }

    /// 精确匹配搜索 - 增强版，同时处理名称和描述中的匹配
    async fn exact_match_search(
        &self,
        query: &str,
    ) -> Result<Vec<RecommendCrate>, Box<dyn std::error::Error>> {
        // 如果查询为空，返回空结果
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }

        // 改进匹配模式，同时支持中英文
        let statement = format!(
            "SELECT id, name, description, 
                   (CASE 
                     WHEN name ILIKE $1 THEN 1.0
                     WHEN name ILIKE $2 THEN 0.9
                     WHEN description ILIKE $1 THEN 0.8
                     ELSE 0.7
                   END) AS rank
             FROM {}
             WHERE name ILIKE $2 OR description ILIKE $2
             ORDER BY rank DESC
             LIMIT 50",
            self.table_name
        );

        let exact_pattern = format!("{}%", query); // 前缀匹配
        let contains_pattern = format!("%{}%", query); // 包含匹配

        let rows = self
            .pg_client
            .query(&statement, &[&exact_pattern, &contains_pattern])
            .await?;

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
                vector_score: 0.0, // 不使用向量得分
                final_score: rank,
            });
        }

        Ok(results)
    }

    /// 前缀匹配搜索 - 优化版，更好地处理中英文
    async fn prefix_match_search(
        &self,
        query: &str,
    ) -> Result<Vec<RecommendCrate>, Box<dyn std::error::Error>> {
        // 如果查询为空，返回空结果
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }

        // 将查询分解为单词并构建tsquery
        let words: Vec<&str> = query.split_whitespace().collect();
        if words.is_empty() {
            return Ok(Vec::new());
        }

        // 为中英文混合查询准备前缀匹配查询
        let mut prefix_terms = Vec::new();

        // 前缀匹配
        for word in &words {
            if word.len() >= 2 {
                prefix_terms.push(format!("{}:*", word));
            }
        }

        // 如果没有有效的词项，返回空结果
        if prefix_terms.is_empty() {
            return Ok(Vec::new());
        }

        // 生成tsquery
        let tsquery = prefix_terms.join(" | "); // 使用OR操作符

        // 执行搜索
        let statement = format!(
            "SELECT id, name, description, ts_rank(tsv, to_tsquery($1)) AS rank
             FROM {}
             WHERE tsv @@ to_tsquery($1)
             ORDER BY rank DESC
             LIMIT 150",
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

    /// 全文搜索 - 使用更宽松的匹配
    async fn fulltext_search(
        &self,
        query: &str,
    ) -> Result<Vec<RecommendCrate>, Box<dyn std::error::Error>> {
        // 如果查询为空，返回空结果
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }

        // 使用websearch_to_tsquery，对用户输入更友好
        let statement = format!(
            "SELECT id, name, description, ts_rank(tsv, websearch_to_tsquery($1)) AS rank
             FROM {}
             WHERE tsv @@ websearch_to_tsquery($1)
             ORDER BY rank DESC
             LIMIT 150",
            self.table_name
        );

        let rows = match self.pg_client.query(&statement, &[&query]).await {
            Ok(r) => r,
            Err(_) => {
                // 如果websearch_to_tsquery不可用，回退到plainto_tsquery
                let fallback_statement = format!(
                    "SELECT id, name, description, ts_rank(tsv, plainto_tsquery($1)) AS rank
                     FROM {}
                     WHERE tsv @@ plainto_tsquery($1)
                     ORDER BY rank DESC
                     LIMIT 150",
                    self.table_name
                );
                self.pg_client.query(&fallback_statement, &[&query]).await?
            }
        };

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

    /// 高级全文搜索 - 用于补充结果
    async fn advanced_fulltext_search(
        &self,
        query: &str,
    ) -> Result<Vec<RecommendCrate>, Box<dyn std::error::Error>> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }

        // 对长句子使用更宽松的全文搜索
        let statement = format!(
            "SELECT id, name, description, 
                    ts_rank(tsv, phraseto_tsquery($1)) * 0.6 AS rank
             FROM {}
             WHERE 
                tsv @@ phraseto_tsquery($1) OR
                name ILIKE $2 OR
                description ILIKE $2
             ORDER BY rank DESC
             LIMIT 200",
            self.table_name
        );

        let pattern = format!(
            "%{}%",
            query.split_whitespace().collect::<Vec<_>>().join("%")
        );

        let rows = self
            .pg_client
            .query(&statement, &[&query, &pattern])
            .await?;

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
            // 计算最终得分，根据排序标准调整
            match sort_criteria {
                SearchSortCriteria::Comprehensive => {
                    // 综合评分保持原样
                    crate_item.final_score = crate_item.rank * weight;
                }
                SearchSortCriteria::Relavance => {
                    // 相关性优先，增强相关性权重
                    crate_item.final_score = crate_item.rank * weight * 1.2;
                }
                SearchSortCriteria::Downloads => {
                    // 下载量优先，减弱相关性权重
                    crate_item.final_score = crate_item.rank * weight * 0.8;
                    // 注意：理想情况下应结合下载量数据
                }
            }

            final_results.push(crate_item);
        }

        // 根据最终得分排序
        final_results.sort_by(|a, b| b.final_score.partial_cmp(&a.final_score).unwrap());

        final_results
    }
}
