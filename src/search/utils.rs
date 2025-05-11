use serde::{Deserialize, Serialize};
use std::env;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

#[derive(Serialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Serialize)]
pub struct RequestBody {
    pub model: String,
    pub messages: Vec<Message>,
    pub temperature: f32,
    pub max_tokens: u32,
}

#[derive(Deserialize)]
pub struct ResponseChoice {
    pub message: ResponseMessage,
}

#[derive(Deserialize)]
pub struct ResponseMessage {
    pub content: String,
}

#[derive(Deserialize)]
pub struct ResponseBody {
    pub choices: Vec<ResponseChoice>,
}

// 基本的关键词提取（无需OpenAI API）
pub fn basic_keyword_extraction(query: &str) -> String {
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
pub fn load_stop_words() -> Vec<String> {
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
