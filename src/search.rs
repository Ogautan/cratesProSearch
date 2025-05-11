use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::env;
use tokio_postgres::Client as PgClient;

pub struct SearchModule<'a> {
    pg_client: &'a PgClient,
    table_name: String,
}

pub enum SearchSortCriteria {
    Comprehensive,
    Relavance,
    Downloads,
}

#[derive(Debug, Clone)]
pub struct RecommendCrate {
    pub id: String,
    pub name: String,
    pub description: String,
    pub rank: f32,
}

impl<'a> SearchModule<'a> {
    pub async fn new(pg_client: &'a PgClient) -> Self {
        let table_name = env::var("TABLE_NAME").unwrap_or_else(|_| "crates".to_string());
        SearchModule {
            pg_client: pg_client,
            table_name,
        }
    }

    pub async fn search_crate(
        &self,
        query: &str,
        sort_by: SearchSortCriteria,
    ) -> Result<Vec<RecommendCrate>, Box<dyn std::error::Error>> {
        let processed_query = process_query(query).await;

        // 使用处理后的查询进行改写
        let rewritten_query = match rewrite_query(&processed_query).await {
            Ok(q) => q,
            Err(e) => {
                eprintln!("查询改写失败: {}", e);
                processed_query // 如果改写失败则使用处理后的查询
            }
        };

        println!("改写后的查询: {}", rewritten_query);
        retrive_crates(self.pg_client, &self.table_name, &rewritten_query).await
    }
}

// 处理查询，判断是否为自然语言并相应地处理
async fn process_query(query: &str) -> String {
    // 检测是否为自然语言查询
    let is_natural_language = is_natural_language_query(query);

    if is_natural_language {
        println!("检测到自然语言查询: {}", query);
        // 如果是自然语言查询，先提取关键词
        match extract_keywords_from_query(query).await {
            Ok(keywords) => {
                println!("从自然语言中提取的关键词: {}", keywords);
                keywords
            }
            Err(e) => {
                eprintln!("提取关键词失败: {}", e);
                query.to_string() // 提取失败则使用原始查询
            }
        }
    } else {
        query.to_string() // 如果是常规查询，直接使用原始查询
    }
}

// 检测查询是否为自然语言句子
fn is_natural_language_query(query: &str) -> bool {
    // 简单判断：包含特定语法结构的查询可能是自然语言
    // 1. 包含多个单词（超过3个单词）
    // 2. 包含标点符号如问号、句号等
    // 3. 包含常见的疑问词或指令词

    let word_count = query.split_whitespace().count();
    let contains_question_mark = query.contains('?');
    let contains_period = query.contains('.');
    let contains_common_question_words = query.to_lowercase().split_whitespace().any(|word| {
        [
            "how", "what", "which", "where", "who", "why", "can", "could", "help", "find", "need",
            "want", "looking",
        ]
        .contains(&word)
    });

    // 如果满足以下任一条件，则认为是自然语言查询
    word_count > 3 || contains_question_mark || contains_period || contains_common_question_words
}

// 从自然语言查询中提取关键词
pub async fn extract_keywords_from_query(
    query: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    // 检查是否配置了OpenAI API密钥
    if let Ok(api_key) = env::var("OPENAI_API_KEY") {
        if !api_key.is_empty() {
            let client = Client::new();
            let open_ai_chat_url = env::var("OPEN_AI_CHAT_URL")
                .unwrap_or_else(|_| "https://api.openai.com/v1/chat/completions".to_string());

            // 构建消息 - 专门针对从自然语言中提取关键词
            let messages = vec![
                Message {
                    role: "system".to_string(),
                    content: "你是一个从自然语言查询中提取Rust软件包关键词的专家。请分析用户的问题，识别与Rust生态系统相关的核心概念和功能需求。仅返回逗号分隔的关键词列表。".to_string(),
                },
                Message {
                    role: "user".to_string(),
                    content: format!("从以下查询中提取用于搜索Rust包的关键词（返回逗号分隔的列表）: {}", query),
                },
            ];

            let request_body = RequestBody {
                model: "gpt-3.5-turbo".to_string(),
                messages,
                temperature: 0.3,
                max_tokens: 100,
            };

            match client
                .post(&open_ai_chat_url)
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", api_key))
                .json(&request_body)
                .send()
                .await
            {
                Ok(response) => {
                    if let Ok(response_body) = response.json::<ResponseBody>().await {
                        if !response_body.choices.is_empty() {
                            return Ok(response_body.choices[0].message.content.trim().to_string());
                        }
                    }
                }
                Err(e) => {
                    eprintln!("访问OpenAI API提取关键词失败: {}", e);
                }
            }
        }
    }

    // 后备方案：使用简单的关键词提取
    Ok(basic_keyword_extraction(query))
}

// 基本的关键词提取（无需OpenAI API）
fn basic_keyword_extraction(query: &str) -> String {
    let query = query.to_lowercase();

    // 从文件中读取停用词
    let stop_words = load_stop_words();

    // 分割查询并移除停用词
    let keywords: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric() && c != '_') // 分割非字母数字和下划线的字符
        .filter(|word| {
            !word.is_empty() && !stop_words.contains(&word.to_string()) && word.len() > 2
        }) // 移除空字符串、停用词和极短单词
        .map(|word| word.to_string())
        .collect();

    // 返回逗号分隔的关键词
    keywords.join(", ")
}

// 加载停用词列表
fn load_stop_words() -> Vec<String> {
    use std::fs::File;
    use std::io::{BufRead, BufReader};
    use std::path::Path;

    let stop_words_path =
        env::var("STOP_WORDS_PATH").unwrap_or_else(|_| "resources/stopwords.txt".to_string());

    // 尝试从文件中读取停用词
    if let Ok(file) = File::open(Path::new(&stop_words_path)) {
        let reader = BufReader::new(file);
        let stop_words: Vec<String> = reader
            .lines()
            .filter_map(Result::ok)
            .filter(|line| !line.trim().is_empty() && !line.starts_with("//"))
            .map(|line| line.trim().to_string())
            .collect();

        if !stop_words.is_empty() {
            println!("已从文件加载 {} 个停用词", stop_words.len());
            return stop_words;
        }
    }

    // 文件不存在或为空时使用默认停用词列表
    println!("无法从文件加载停用词，使用默认停用词列表");
    vec![
        "a".to_string(),
        "an".to_string(),
        "the".to_string(),
        "is".to_string(),
        "are".to_string(),
        "was".to_string(),
        "were".to_string(),
        "be".to_string(),
        "in".to_string(),
        "on".to_string(),
        "at".to_string(),
        "by".to_string(),
        "for".to_string(),
        "with".to_string(),
        "about".to_string(),
        "against".to_string(),
        "how".to_string(),
        "what".to_string(),
        "where".to_string(),
        "when".to_string(),
        "why".to_string(),
        "who".to_string(),
        "which".to_string(),
        "and".to_string(),
        "or".to_string(),
        "if".to_string(),
        "but".to_string(),
        "because".to_string(),
        "as".to_string(),
        "until".to_string(),
        "while".to_string(),
        "of".to_string(),
        "to".to_string(),
        "from".to_string(),
        "need".to_string(),
        "want".to_string(),
        "find".to_string(),
        "looking".to_string(),
        "search".to_string(),
        "rust".to_string(),
        "crate".to_string(),
    ]
}

#[derive(Serialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct RequestBody {
    model: String,
    messages: Vec<Message>,
    temperature: f32,
    max_tokens: u32,
}

#[derive(Deserialize)]
struct ResponseChoice {
    message: ResponseMessage,
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: String,
}

#[derive(Deserialize)]
struct ResponseBody {
    choices: Vec<ResponseChoice>,
}

pub async fn rewrite_query(query: &str) -> Result<String, Box<dyn std::error::Error>> {
    // 检查是否配置了OpenAI API密钥
    if let Ok(api_key) = env::var("OPENAI_API_KEY") {
        if !api_key.is_empty() {
            let client = Client::new();
            // 从环境变量获取自定义API端点
            let open_ai_chat_url = env::var("OPEN_AI_CHAT_URL")
                .unwrap_or_else(|_| "https://api.openai.com/v1/chat/completions".to_string());

            // 构建消息 - 更新系统提示以处理各种类型的查询
            let messages = vec![
                Message {
                    role: "system".to_string(),
                    content: "你是一个专门改写Rust软件包查询的助手。分析输入并生成适合在crates.io搜索引擎中使用的关键词。无论输入是关键词还是自然语言问题，都将其转换为相关技术术语和同义词的列表。返回逗号分隔的关键词列表，不要添加解释。".to_string(),
                },
                Message {
                    role: "user".to_string(),
                    content: format!("生成以下内容的Rust包关键词列表（以逗号分隔）: {}", query),
                },
            ];

            // 构建请求体
            let request_body = RequestBody {
                model: "gpt-3.5-turbo".to_string(),
                messages,
                temperature: 0.3,
                max_tokens: 150,
            };

            // 发送请求
            match client
                .post(&open_ai_chat_url)
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", api_key))
                .json(&request_body)
                .send()
                .await
            {
                Ok(response) => {
                    // 解析响应
                    if let Ok(response_body) = response.json::<ResponseBody>().await {
                        if !response_body.choices.is_empty() {
                            return Ok(response_body.choices[0].message.content.trim().to_string());
                        }
                    }
                }
                Err(e) => {
                    eprintln!("访问OpenAI API失败: {}", e);
                }
            }
        }
    }

    // 后备方案：简单的查询增强
    Ok(basic_query_enhancement(query))
}

fn basic_query_enhancement(query: &str) -> String {
    // 简单的查询处理，当无法使用LLM时
    let query = query.trim().to_lowercase();

    // 移除常见的无用词
    let stop_words = ["the", "a", "an", "in", "for", "with", "by"];
    let mut enhanced = query.to_string();

    for word in stop_words.iter() {
        // 确保只替换完整的单词
        enhanced = enhanced
            .replace(&format!(" {} ", word), " ")
            .replace(&format!(" {}", word), "")
            .replace(&format!("{} ", word), "");
    }

    enhanced.trim().to_string()
}

async fn rerank_crates(crates: Vec<RecommendCrate>) -> Vec<RecommendCrate> {
    // 这里可以实现更复杂的排序逻辑
    // 例如根据下载量、评分等进行排序
    // 目前只是简单地按rank排序
    let mut sorted_crates = crates.clone();
    sorted_crates.sort_by(|a, b| b.rank.partial_cmp(&a.rank).unwrap());
    sorted_crates
        .into_iter()
        .take(10) // 只返回前10个结果
        .collect()
}

async fn retrive_crates(
    client: &PgClient,
    table_name: &str,
    keyword: &str,
) -> Result<Vec<RecommendCrate>, Box<dyn std::error::Error>> {
    // 处理关键词
    let keywords: Vec<&str> = keyword.split(',').collect();

    // 处理每个关键词以生成有效的tsquery
    let mut processed_terms = Vec::new();

    for kw in keywords.iter().take(5) {
        // 限制为前5个关键词以提高性能
        let term = kw.trim().to_lowercase();

        // 如果关键词包含空格，则将空格替换为&（AND操作符）
        // 例如："http client" => "http & client"
        let processed_term = term.replace(" ", " & ");

        // 为每个处理后的术语添加:*以实现前缀匹配
        processed_terms.push(format!("{}:*", processed_term));
    }

    // 使用OR操作符连接所有处理后的术语
    let tsquery = processed_terms.join(" | ");

    println!("执行PostgreSQL查询: {}", tsquery);

    let statement = format!(
        "SELECT {0}.id, {0}.name, {0}.description, ts_rank({0}.tsv, to_tsquery($1)) AS rank
        FROM {0}
        WHERE {0}.tsv @@ to_tsquery($1)
        ORDER BY rank DESC
        LIMIT 200",
        table_name
    );
    let rows = client.query(statement.as_str(), &[&tsquery]).await?;
    let mut recommend_crates = Vec::<RecommendCrate>::new();

    for row in rows.iter() {
        let id: Option<String> = row.get("id");
        let name: Option<String> = row.get("name");
        let description: Option<String> = row.get("description");
        let rank: Option<f32> = row.get("rank");

        recommend_crates.push(RecommendCrate {
            id: id.unwrap_or_default(),
            name: name.unwrap_or_default(),
            description: description.unwrap_or_default(),
            rank: rank.unwrap_or(0.0),
        });
    }

    Ok(recommend_crates)
}
