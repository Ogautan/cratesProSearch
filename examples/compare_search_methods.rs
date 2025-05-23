use cratespro_search::search::{
    RecommendCrate, SearchModule, SearchSortCriteria, TraditionalSearchModule,
};
use dotenv::dotenv;
use prettytable::{format, Cell, Row, Table};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs::File;
use std::io::{BufReader, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_postgres::NoTls;

#[derive(Debug, Deserialize, Serialize)]
struct TestCase {
    query: String,
    description: String,
    relevant_packages: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ComparisonResult {
    query: String,
    description: String,
    method: String,
    precision_at_1: f64,
    precision_at_3: f64,
    precision_at_5: f64,
    precision_at_10: f64,
    recall: f64,
    latency_ms: f64,
}

#[derive(Debug, Deserialize, Serialize)]
struct LLMMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct LLMRequest {
    model: String,
    messages: Vec<LLMMessage>,
    temperature: f32,
}

#[derive(Debug, Deserialize, Serialize)]
struct LLMResponseChoice {
    message: LLMMessage,
}

#[derive(Debug, Deserialize, Serialize)]
struct LLMResponse {
    choices: Vec<LLMResponseChoice>,
}

#[derive(Debug, Deserialize, Serialize)]
struct RelevanceJudgment {
    crate_name: String,
    is_relevant: bool,
    confidence: f32,
    reasoning: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct LLMJudgmentResponse {
    judgments: Vec<RelevanceJudgment>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 加载环境变量
    dotenv().ok();

    println!("🔍 开始搜索方法对比实验 (使用LLM进行相关性判断)");

    // 确保OpenAI API密钥已配置
    let api_key = env::var("OPENAI_API_KEY").expect("需要设置OPENAI_API_KEY环境变量");

    // 连接到数据库
    let db_url = env::var("DATABASE_URL").expect("DATABASE_URL 环境变量未设置");
    let (pg_client, connection) = tokio_postgres::connect(&db_url, NoTls).await?;

    // 在后台运行连接
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("数据库连接错误: {}", e);
        }
    });

    // 创建不同的搜索模块
    let llm_search = SearchModule::new(&pg_client).await;
    let traditional_search = TraditionalSearchModule::new(&pg_client).await;

    // 加载测试用例
    let test_cases = load_test_cases();
    println!("📋 已加载 {} 个测试用例", test_cases.len());

    // 创建HTTP客户端
    let http_client = Arc::new(Client::new());

    // 创建缓存以避免重复LLM调用
    let mut relevance_cache = HashMap::new();

    // 存储比较结果
    let mut results = Vec::new();

    // 执行测试
    for test_case in &test_cases {
        println!(
            "\n📝 测试用例: {} - \"{}\"",
            test_case.description, test_case.query
        );

        // LLM增强搜索
        println!("\n  🧠 LLM增强搜索:");
        let llm_start = Instant::now();
        let llm_results = match llm_search
            .search_crate(&test_case.query, SearchSortCriteria::Comprehensive)
            .await
        {
            Ok(res) => res,
            Err(e) => {
                eprintln!("LLM搜索错误: {}", e);
                continue;
            }
        };
        let llm_duration = llm_start.elapsed();

        // 使用LLM评估相关性
        let llm_eval_start = Instant::now();
        println!("  🔍 使用LLM评估搜索结果相关性...");
        let llm_relevance = evaluate_with_llm(
            &http_client,
            &test_case.query,
            &llm_results[..20.min(llm_results.len())],
            &api_key,
            &mut relevance_cache,
        )
        .await?;
        let llm_eval_duration = llm_eval_start.elapsed();

        // 使用LLM相关性判断计算指标
        let llm_metrics = calculate_metrics_from_llm_judgments(&llm_results, &llm_relevance);

        println!(
            "    ⏱️ 搜索耗时: {:.2?}, 相关性评估耗时: {:.2?}",
            llm_duration, llm_eval_duration
        );
        println!(
            "    P@1: {:.2}, P@3: {:.2}, P@5: {:.2}, P@10: {:.2}, 相关结果: {}",
            llm_metrics.0, llm_metrics.1, llm_metrics.2, llm_metrics.3, llm_metrics.4
        );

        // 打印LLM搜索的前5个结果及其相关性
        print_results_with_llm_judgments("LLM增强搜索", &llm_results, &llm_relevance, 5);

        // 传统搜索
        println!("\n  📚 传统搜索:");
        let trad_start = Instant::now();
        let trad_results = match traditional_search
            .search(&test_case.query, SearchSortCriteria::Comprehensive)
            .await
        {
            Ok(res) => res,
            Err(e) => {
                eprintln!("传统搜索错误: {}", e);
                continue;
            }
        };
        let trad_duration = trad_start.elapsed();

        // 使用LLM评估传统搜索结果相关性
        let trad_eval_start = Instant::now();
        println!("  🔍 使用LLM评估传统搜索结果相关性...");
        let trad_relevance = evaluate_with_llm(
            &http_client,
            &test_case.query,
            &trad_results[..20.min(trad_results.len())],
            &api_key,
            &mut relevance_cache,
        )
        .await?;
        let trad_eval_duration = trad_eval_start.elapsed();

        // 使用LLM相关性判断计算指标
        let trad_metrics = calculate_metrics_from_llm_judgments(&trad_results, &trad_relevance);

        println!(
            "    ⏱️ 搜索耗时: {:.2?}, 相关性评估耗时: {:.2?}",
            trad_duration, trad_eval_duration
        );
        println!(
            "    P@1: {:.2}, P@3: {:.2}, P@5: {:.2}, P@10: {:.2}, 相关结果: {}",
            trad_metrics.0, trad_metrics.1, trad_metrics.2, trad_metrics.3, trad_metrics.4
        );

        // 打印传统搜索的前5个结果及其相关性
        print_results_with_llm_judgments("传统搜索", &trad_results, &trad_relevance, 5);

        // 记录结果
        results.push(ComparisonResult {
            query: test_case.query.clone(),
            description: test_case.description.clone(),
            method: "LLM增强搜索".to_string(),
            precision_at_1: llm_metrics.0,
            precision_at_3: llm_metrics.1,
            precision_at_5: llm_metrics.2,
            precision_at_10: llm_metrics.3,
            recall: llm_metrics.4 as f64, // 使用相关结果数量作为召回指标
            latency_ms: llm_duration.as_millis() as f64,
        });

        results.push(ComparisonResult {
            query: test_case.query.clone(),
            description: test_case.description.clone(),
            method: "传统搜索".to_string(),
            precision_at_1: trad_metrics.0,
            precision_at_3: trad_metrics.1,
            precision_at_5: trad_metrics.2,
            precision_at_10: trad_metrics.3,
            recall: trad_metrics.4 as f64, // 使用相关结果数量作为召回指标
            latency_ms: trad_duration.as_millis() as f64,
        });
    }

    // 生成报告
    generate_report(&results);

    // 保存结果到文件
    if let Ok(mut file) = File::create("search_comparison_llm_judged.json") {
        let json = serde_json::to_string_pretty(&results)?;
        file.write_all(json.as_bytes())?;
        println!("\n💾 结果已保存到 search_comparison_llm_judged.json");
    }

    println!("\n✅ 对比实验完成");
    Ok(())
}

async fn evaluate_with_llm(
    client: &Client,
    query: &str,
    results: &[RecommendCrate],
    api_key: &str,
    cache: &mut HashMap<String, HashMap<String, bool>>,
) -> Result<HashMap<String, bool>, Box<dyn std::error::Error>> {
    let cache_key = query.to_lowercase();
    if let Some(cached_judgments) = cache.get(&cache_key) {
        let all_cached = results
            .iter()
            .all(|r| cached_judgments.contains_key(&r.name.to_lowercase()));
        if all_cached {
            let mut filtered_judgments = HashMap::new();
            for result in results {
                if let Some(&is_relevant) = cached_judgments.get(&result.name.to_lowercase()) {
                    filtered_judgments.insert(result.name.clone(), is_relevant);
                }
            }
            return Ok(filtered_judgments);
        }
    }

    let batch_size = 5;
    let mut all_judgments = HashMap::new();

    for chunk in results.chunks(batch_size) {
        let mut crates_description = String::new();
        for (i, crate_item) in chunk.iter().enumerate() {
            crates_description.push_str(&format!(
                "Crate {}: {} - {}\n",
                i + 1,
                crate_item.name,
                crate_item.description
            ));
        }

        let system_prompt = "你是一个专业的Rust编程助手，负责评估搜索结果与查询的相关性。请根据查询和每个crate的描述，判断它们是否相关。";
        let user_prompt = format!(
            "查询: \"{}\"\n\n以下是搜索结果:\n{}\n请对每个crate进行相关性判断，返回JSON格式:\n{{\"judgments\": [{{\n  \"crate_name\": \"crate名称\",\n  \"is_relevant\": true/false,\n  \"confidence\": 0.0-1.0,\n  \"reasoning\": \"判断理由\"\n}}, ...]}}\n只返回JSON，不要有其他文字。",
            query, crates_description
        );

        let openai_url = env::var("OPEN_AI_CHAT_URL")
            .unwrap_or_else(|_| "https://api.openai.com/v1/chat/completions".to_string());

        let request = LLMRequest {
            model: "gpt-4-turbo".to_string(),
            messages: vec![
                LLMMessage {
                    role: "system".to_string(),
                    content: system_prompt.to_string(),
                },
                LLMMessage {
                    role: "user".to_string(),
                    content: user_prompt,
                },
            ],
            temperature: 0.2,
        };

        let response = client
            .post(&openai_url)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            eprintln!("OpenAI API错误: {}", error_text);
            return Err(format!("OpenAI API返回错误: {}", error_text).into());
        }

        let response_data: LLMResponse = response.json().await?;
        if response_data.choices.is_empty() {
            return Err("LLM没有返回选择结果".into());
        }

        let content = &response_data.choices[0].message.content;

        let json_start = content.find('{');
        let json_end = content.rfind('}');

        if let (Some(start), Some(end)) = (json_start, json_end) {
            let json_content = &content[start..=end];
            match serde_json::from_str::<LLMJudgmentResponse>(json_content) {
                Ok(judgment_data) => {
                    for judgment in judgment_data.judgments {
                        all_judgments.insert(judgment.crate_name.clone(), judgment.is_relevant);

                        if !cache.contains_key(&cache_key) {
                            cache.insert(cache_key.clone(), HashMap::new());
                        }
                        if let Some(cache_map) = cache.get_mut(&cache_key) {
                            cache_map
                                .insert(judgment.crate_name.to_lowercase(), judgment.is_relevant);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("JSON解析错误: {}. 原始内容: {}", e, json_content);
                    if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(json_content)
                    {
                        if let Some(judgments) =
                            json_value.get("judgments").and_then(|j| j.as_array())
                        {
                            for judgment in judgments {
                                if let (Some(name), Some(relevant)) = (
                                    judgment.get("crate_name").and_then(|n| n.as_str()),
                                    judgment.get("is_relevant").and_then(|r| r.as_bool()),
                                ) {
                                    all_judgments.insert(name.to_string(), relevant);

                                    if !cache.contains_key(&cache_key) {
                                        cache.insert(cache_key.clone(), HashMap::new());
                                    }
                                    if let Some(cache_map) = cache.get_mut(&cache_key) {
                                        cache_map.insert(name.to_lowercase(), relevant);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        } else {
            eprintln!("无法解析LLM响应中的JSON: {}", content);
        }
    }

    Ok(all_judgments)
}

fn calculate_metrics_from_llm_judgments(
    results: &[RecommendCrate],
    judgments: &HashMap<String, bool>,
) -> (f64, f64, f64, f64, usize) {
    let relevant_flags: Vec<bool> = results
        .iter()
        .map(|r| judgments.get(&r.name).copied().unwrap_or(false))
        .collect();

    let p1 = calculate_precision_at_k(&relevant_flags, 1);
    let p3 = calculate_precision_at_k(&relevant_flags, 3);
    let p5 = calculate_precision_at_k(&relevant_flags, 5);
    let p10 = calculate_precision_at_k(&relevant_flags, 10);

    let relevant_count = relevant_flags
        .iter()
        .filter(|&&is_relevant| is_relevant)
        .count();

    (p1, p3, p5, p10, relevant_count)
}

fn print_results_with_llm_judgments(
    method: &str,
    results: &[RecommendCrate],
    judgments: &HashMap<String, bool>,
    count: usize,
) {
    println!("    📋 {}的前{}个结果及相关性:", method, count);

    for (i, result) in results.iter().take(count).enumerate() {
        let is_relevant = judgments.get(&result.name).copied().unwrap_or(false);
        let mark = if is_relevant { "✓" } else { "✗" };

        println!(
            "      {}. {} {} - {} (得分: {:.4})",
            i + 1,
            mark,
            result.name,
            truncate_text(&result.description, 40),
            result.final_score
        );
    }
}

// 其余函数保持不变
fn load_test_cases() -> Vec<TestCase> {
    // 尝试从文件加载测试用例
    if let Ok(file) = File::open(Path::new("data/test_cases.json")) {
        let reader = BufReader::new(file);
        if let Ok(cases) = serde_json::from_reader::<_, Vec<TestCase>>(reader) {
            return cases;
        }
    }

    // 默认测试用例
    vec![
        TestCase {
            query: "http client".to_string(),
            description: "HTTP客户端库".to_string(),
            relevant_packages: vec![
                "reqwest".to_string(),
                "hyper".to_string(),
                "surf".to_string(),
                "ureq".to_string(),
                "isahc".to_string(),
            ],
        },
        TestCase {
            query: "json serde".to_string(),
            description: "JSON序列化库".to_string(),
            relevant_packages: vec![
                "serde_json".to_string(),
                "serde".to_string(),
                "json".to_string(),
            ],
        },
        TestCase {
            query: "如何解析JSON数据".to_string(),
            description: "中文自然语言查询".to_string(),
            relevant_packages: vec![
                "serde_json".to_string(),
                "serde".to_string(),
                "json".to_string(),
            ],
        },
        TestCase {
            query: "database orm".to_string(),
            description: "数据库ORM".to_string(),
            relevant_packages: vec![
                "diesel".to_string(),
                "sqlx".to_string(),
                "sea-orm".to_string(),
                "rusqlite".to_string(),
            ],
        },
        TestCase {
            query: "command line arguments parser".to_string(),
            description: "命令行参数解析".to_string(),
            relevant_packages: vec![
                "clap".to_string(),
                "structopt".to_string(),
                "argh".to_string(),
                "pico-args".to_string(),
            ],
        },
        TestCase {
            query: "need a web server framework".to_string(),
            description: "Web框架自然语言".to_string(),
            relevant_packages: vec![
                "actix-web".to_string(),
                "rocket".to_string(),
                "warp".to_string(),
                "axum".to_string(),
                "tide".to_string(),
            ],
        },
        TestCase {
            query: "我需要一个好用的日志库".to_string(),
            description: "中文日志库查询".to_string(),
            relevant_packages: vec![
                "log".to_string(),
                "tracing".to_string(),
                "env_logger".to_string(),
                "slog".to_string(),
            ],
        },
    ]
}

fn calculate_precision_at_k(relevant_flags: &[bool], k: usize) -> f64 {
    if relevant_flags.is_empty() || k == 0 {
        return 0.0;
    }

    let k_actual = k.min(relevant_flags.len());
    let relevant_count = relevant_flags
        .iter()
        .take(k_actual)
        .filter(|&&is_relevant| is_relevant)
        .count();

    relevant_count as f64 / k_actual as f64
}

fn generate_report(results: &[ComparisonResult]) {
    let mut table = Table::new();
    table.set_format(*format::consts::FORMAT_BOX_CHARS);

    table.add_row(Row::new(vec![
        Cell::new("查询"),
        Cell::new("方法"),
        Cell::new("P@1"),
        Cell::new("P@5"),
        Cell::new("P@10"),
        Cell::new("召回率"),
        Cell::new("延迟(ms)"),
    ]));

    for result in results {
        table.add_row(Row::new(vec![
            Cell::new(&truncate_text(
                &format!("{}({})", &result.query, &result.description),
                30,
            )),
            Cell::new(&result.method),
            Cell::new(&format!("{:.2}", result.precision_at_1)),
            Cell::new(&format!("{:.2}", result.precision_at_5)),
            Cell::new(&format!("{:.2}", result.precision_at_10)),
            Cell::new(&format!("{:.2}", result.recall)),
            Cell::new(&format!("{:.1}", result.latency_ms)),
        ]));
    }

    println!("\n📊 搜索方法对比结果:");
    table.printstd();

    let llm_results: Vec<_> = results
        .iter()
        .filter(|r| r.method == "LLM增强搜索")
        .collect();
    let trad_results: Vec<_> = results.iter().filter(|r| r.method == "传统搜索").collect();

    if !llm_results.is_empty() && !trad_results.is_empty() {
        let avg_llm_p1 =
            llm_results.iter().map(|r| r.precision_at_1).sum::<f64>() / llm_results.len() as f64;
        let avg_llm_p5 =
            llm_results.iter().map(|r| r.precision_at_5).sum::<f64>() / llm_results.len() as f64;
        let avg_llm_recall =
            llm_results.iter().map(|r| r.recall).sum::<f64>() / llm_results.len() as f64;
        let avg_llm_latency =
            llm_results.iter().map(|r| r.latency_ms).sum::<f64>() / llm_results.len() as f64;

        let avg_trad_p1 =
            trad_results.iter().map(|r| r.precision_at_1).sum::<f64>() / trad_results.len() as f64;
        let avg_trad_p5 =
            trad_results.iter().map(|r| r.precision_at_5).sum::<f64>() / trad_results.len() as f64;
        let avg_trad_recall =
            trad_results.iter().map(|r| r.recall).sum::<f64>() / trad_results.len() as f64;
        let avg_trad_latency =
            trad_results.iter().map(|r| r.latency_ms).sum::<f64>() / trad_results.len() as f64;

        println!("\n📈 平均性能:");
        println!(
            "  LLM增强搜索: P@1={:.4}, P@5={:.4}, 召回率={:.4}, 延迟={:.1}ms",
            avg_llm_p1, avg_llm_p5, avg_llm_recall, avg_llm_latency
        );
        println!(
            "  传统搜索:    P@1={:.4}, P@5={:.4}, 召回率={:.4}, 延迟={:.1}ms",
            avg_trad_p1, avg_trad_p5, avg_trad_recall, avg_trad_latency
        );

        let p1_improve = (avg_llm_p1 / avg_trad_p1 - 1.0) * 100.0;
        let p5_improve = (avg_llm_p5 / avg_trad_p5 - 1.0) * 100.0;
        let recall_improve = (avg_llm_recall / avg_trad_recall - 1.0) * 100.0;
        let latency_increase = (avg_llm_latency / avg_trad_latency - 1.0) * 100.0;

        println!("\n📊 LLM搜索相比传统搜索的提升:");
        println!("  P@1: {:+.1}%", p1_improve);
        println!("  P@5: {:+.1}%", p5_improve);
        println!("  召回率: {:+.1}%", recall_improve);
        println!("  延迟开销: {:+.1}%", latency_increase);
    }
}

fn truncate_text(s: &str, max_chars: usize) -> String {
    let chars: Vec<char> = s.chars().collect();

    if chars.len() <= max_chars {
        s.to_string()
    } else {
        chars.into_iter().take(max_chars).collect::<String>() + "..."
    }
}
