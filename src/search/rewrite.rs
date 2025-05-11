use crate::search::utils::{basic_keyword_extraction, Message, RequestBody, ResponseBody};
use reqwest::Client;
use std::env;

// 处理查询，判断是否为自然语言并相应地处理
pub async fn process_query(query: &str) -> String {
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
pub fn is_natural_language_query(query: &str) -> bool {
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

pub fn basic_query_enhancement(query: &str) -> String {
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
