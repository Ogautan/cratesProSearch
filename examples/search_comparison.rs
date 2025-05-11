use cratespro_search::search::{RecommendCrate, SearchModule, SearchSortCriteria};
use dotenv::dotenv;
use prettytable::{format, Cell, Row, Table};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::env;
use std::fs::File;
use std::io::{BufReader, Write};
use std::path::Path;
use std::time::Instant;
use tokio_postgres::NoTls;

/// 预标注的查询和相关包的数据结构
#[derive(Debug, Deserialize, Serialize)]
struct QueryRelevance {
    /// 查询内容
    query: String,
    /// 查询描述/类别
    description: String,
    /// 预标注的相关包ID或名称列表
    relevant_packages: Vec<String>,
}

/// 单次评估结果
#[derive(Debug, Serialize)]
struct EvaluationResult {
    query: String,
    description: String,
    method: String,
    precision_at_1: f64,
    precision_at_3: f64,
    precision_at_5: f64,
    precision_at_10: f64,
    relevant_found: usize,
    total_relevant: usize,
    execution_time_ms: f64,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 加载环境变量
    dotenv().ok();

    println!("🔍 开始搜索方法对比实验");

    // 连接到数据库
    let db_url = env::var("DATABASE_URL").expect("DATABASE_URL 环境变量未设置");
    let (pg_client, connection) = tokio_postgres::connect(&db_url, NoTls).await?;

    // 在后台运行连接
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("数据库连接错误: {}", e);
        }
    });

    // 创建搜索模块
    let search_module = SearchModule::new(&pg_client).await;

    // 加载测试数据集
    let dataset = get_test_dataset();
    println!("📋 已加载 {} 个带标注的查询", dataset.len());

    // 存储实验结果
    let mut results = Vec::new();

    // 对每个查询进行评估
    for query_data in &dataset {
        println!(
            "\n📝 评估查询: \"{}\" ({})",
            query_data.query, query_data.description
        );
        println!(
            "👀 标注的相关包数量: {}",
            query_data.relevant_packages.len()
        );

        let relevant_packages: HashSet<String> = query_data
            .relevant_packages
            .iter()
            .map(|p| p.to_lowercase())
            .collect();

        // 测试常规LLM增强搜索
        println!("\n  🧠 使用LLM增强搜索方法:");
        let llm_start = Instant::now();
        let llm_results = match search_module
            .search_crate(&query_data.query, SearchSortCriteria::Comprehensive)
            .await
        {
            Ok(res) => res,
            Err(e) => {
                eprintln!("LLM搜索错误: {}", e);
                continue;
            }
        };
        let llm_duration = llm_start.elapsed();

        // 评估LLM搜索结果
        let (p1_llm, p3_llm, p5_llm, p10_llm, found_llm) =
            evaluate_results(&llm_results, &relevant_packages);

        println!("    ⏱️ 执行时间: {:.2?}", llm_duration);
        println!(
            "    📊 P@1: {:.2}, P@3: {:.2}, P@5: {:.2}, P@10: {:.2}",
            p1_llm, p3_llm, p5_llm, p10_llm
        );
        println!(
            "    ✓ 找到相关包: {}/{}",
            found_llm,
            relevant_packages.len()
        );

        // 打印前5个LLM结果
        print_top_results(&llm_results, &relevant_packages, 5);

        // 测试无LLM搜索
        println!("\n  🔎 使用非LLM搜索方法:");
        let no_llm_start = Instant::now();
        let no_llm_results = match search_module
            .search_crate_without_ai(&query_data.query, SearchSortCriteria::Comprehensive)
            .await
        {
            Ok(res) => res,
            Err(e) => {
                eprintln!("非LLM搜索错误: {}", e);
                continue;
            }
        };
        let no_llm_duration = no_llm_start.elapsed();

        // 评估无LLM搜索结果
        let (p1_no_llm, p3_no_llm, p5_no_llm, p10_no_llm, found_no_llm) =
            evaluate_results(&no_llm_results, &relevant_packages);

        println!("    ⏱️ 执行时间: {:.2?}", no_llm_duration);
        println!(
            "    📊 P@1: {:.2}, P@3: {:.2}, P@5: {:.2}, P@10: {:.2}",
            p1_no_llm, p3_no_llm, p5_no_llm, p10_no_llm
        );
        println!(
            "    ✓ 找到相关包: {}/{}",
            found_no_llm,
            relevant_packages.len()
        );

        // 打印前5个无LLM结果
        print_top_results(&no_llm_results, &relevant_packages, 5);

        // 记录结果
        results.push(EvaluationResult {
            query: query_data.query.clone(),
            description: query_data.description.clone(),
            method: "LLM增强搜索".to_string(),
            precision_at_1: p1_llm,
            precision_at_3: p3_llm,
            precision_at_5: p5_llm,
            precision_at_10: p10_llm,
            relevant_found: found_llm,
            total_relevant: relevant_packages.len(),
            execution_time_ms: llm_duration.as_secs_f64() * 1000.0,
        });

        results.push(EvaluationResult {
            query: query_data.query.clone(),
            description: query_data.description.clone(),
            method: "非LLM搜索".to_string(),
            precision_at_1: p1_no_llm,
            precision_at_3: p3_no_llm,
            precision_at_5: p5_no_llm,
            precision_at_10: p10_no_llm,
            relevant_found: found_no_llm,
            total_relevant: relevant_packages.len(),
            execution_time_ms: no_llm_duration.as_secs_f64() * 1000.0,
        });
    }

    // 生成对比报告
    generate_comparison_report(&results);

    // 保存结果到JSON文件
    if let Ok(mut file) = File::create("search_comparison_results.json") {
        if let Ok(json) = serde_json::to_string_pretty(&results) {
            let _ = file.write_all(json.as_bytes());
            println!("\n💾 实验结果已保存到search_comparison_results.json");
        }
    }

    println!("\n✅ 实验完成");
    Ok(())
}

/// 预定义的测试数据集
fn get_test_dataset() -> Vec<QueryRelevance> {
    // 如果有JSON文件，从文件加载数据
    if let Ok(file) = File::open(Path::new("data/relevance_dataset.json")) {
        let reader = BufReader::new(file);
        if let Ok(dataset) = serde_json::from_reader(reader) {
            return dataset;
        }
    }

    // 否则返回硬编码的测试数据集
    vec![
        QueryRelevance {
            query: "http client".to_string(),
            description: "HTTP客户端库".to_string(),
            relevant_packages: vec![
                "reqwest".to_string(),
                "hyper".to_string(),
                "surf".to_string(),
                "ureq".to_string(),
                "isahc".to_string(),
                "http".to_string(),
                "curl".to_string(),
            ],
        },
        QueryRelevance {
            query: "json parser".to_string(),
            description: "JSON解析库".to_string(),
            relevant_packages: vec![
                "serde_json".to_string(),
                "json".to_string(),
                "simd-json".to_string(),
                "jsonpath".to_string(),
                "serde".to_string(),
            ],
        },
        QueryRelevance {
            query: "async runtime".to_string(),
            description: "异步运行时".to_string(),
            relevant_packages: vec![
                "tokio".to_string(),
                "async-std".to_string(),
                "smol".to_string(),
                "futures".to_string(),
                "embassy".to_string(),
            ],
        },
        QueryRelevance {
            query: "command line arguments".to_string(),
            description: "命令行参数处理".to_string(),
            relevant_packages: vec![
                "clap".to_string(),
                "structopt".to_string(),
                "argh".to_string(),
                "pico-args".to_string(),
                "dialoguer".to_string(),
            ],
        },
        QueryRelevance {
            query: "database orm".to_string(),
            description: "数据库ORM".to_string(),
            relevant_packages: vec![
                "diesel".to_string(),
                "sqlx".to_string(),
                "sea-orm".to_string(),
                "rusqlite".to_string(),
                "tokio-postgres".to_string(),
            ],
        },
        QueryRelevance {
            query: "我需要一个HTTP客户端库".to_string(),
            description: "HTTP客户端（中文自然语言）".to_string(),
            relevant_packages: vec![
                "reqwest".to_string(),
                "hyper".to_string(),
                "surf".to_string(),
                "ureq".to_string(),
            ],
        },
        QueryRelevance {
            query: "如何解析JSON数据？".to_string(),
            description: "JSON解析（中文自然语言）".to_string(),
            relevant_packages: vec![
                "serde_json".to_string(),
                "json".to_string(),
                "serde".to_string(),
            ],
        },
        QueryRelevance {
            query: "How to connect to a database in Rust?".to_string(),
            description: "数据库连接（英文自然语言）".to_string(),
            relevant_packages: vec![
                "sqlx".to_string(),
                "diesel".to_string(),
                "tokio-postgres".to_string(),
                "rusqlite".to_string(),
            ],
        },
        QueryRelevance {
            query: "推荐一个日志库".to_string(),
            description: "日志库（中文自然语言）".to_string(),
            relevant_packages: vec![
                "log".to_string(),
                "env_logger".to_string(),
                "tracing".to_string(),
                "slog".to_string(),
                "fern".to_string(),
            ],
        },
        QueryRelevance {
            query: "web框架".to_string(),
            description: "Web框架（中文）".to_string(),
            relevant_packages: vec![
                "actix-web".to_string(),
                "rocket".to_string(),
                "warp".to_string(),
                "axum".to_string(),
                "tide".to_string(),
            ],
        },
    ]
}

/// 评估搜索结果并计算各种P@K指标
fn evaluate_results(
    results: &[RecommendCrate],
    relevant_packages: &HashSet<String>,
) -> (f64, f64, f64, f64, usize) {
    // 标记结果中的相关项
    let relevant_flags: Vec<bool> = results
        .iter()
        .map(|r| relevant_packages.contains(&r.name.to_lowercase()))
        .collect();

    // 计算不同K值的精确度
    let p1 = calculate_precision_at_k(&relevant_flags, 1);
    let p3 = calculate_precision_at_k(&relevant_flags, 3);
    let p5 = calculate_precision_at_k(&relevant_flags, 5);
    let p10 = calculate_precision_at_k(&relevant_flags, 10);

    // 找到的相关包总数
    let found_relevant = results
        .iter()
        .filter(|r| relevant_packages.contains(&r.name.to_lowercase()))
        .count();

    (p1, p3, p5, p10, found_relevant)
}

/// 计算Precision@K
fn calculate_precision_at_k(relevant_flags: &[bool], k: usize) -> f64 {
    if relevant_flags.is_empty() || k == 0 {
        return 0.0;
    }

    let k_actual = std::cmp::min(k, relevant_flags.len());
    let relevant_count = relevant_flags
        .iter()
        .take(k_actual)
        .filter(|&&is_relevant| is_relevant)
        .count();

    relevant_count as f64 / k_actual as f64
}

/// 打印结果前几项，并标记哪些是相关的
fn print_top_results(
    results: &[RecommendCrate],
    relevant_packages: &HashSet<String>,
    count: usize,
) {
    println!("    📋 前{}个结果:", count);

    for (i, result) in results.iter().take(count).enumerate() {
        let is_relevant = relevant_packages.contains(&result.name.to_lowercase());
        let mark = if is_relevant { "✓" } else { "✗" };

        println!(
            "      {}. {} {} - {} (得分: {:.4})",
            i + 1,
            mark,
            result.name,
            truncate_utf8(&result.description, 40),
            result.final_score
        );
    }
}

/// 生成比较报告和统计信息
fn generate_comparison_report(results: &[EvaluationResult]) {
    // 创建表格
    let mut table = Table::new();
    table.set_format(*format::consts::FORMAT_BOX_CHARS);

    // 添加表头
    table.add_row(Row::new(vec![
        Cell::new("查询"),
        Cell::new("搜索方法"),
        Cell::new("P@1"),
        Cell::new("P@3"),
        Cell::new("P@5"),
        Cell::new("P@10"),
        Cell::new("找到/总数"),
        Cell::new("耗时(ms)"),
    ]));

    // 添加数据行
    for result in results {
        table.add_row(Row::new(vec![
            Cell::new(&truncate_utf8(
                &format!("{}({})", &result.query, &result.description),
                25,
            )),
            Cell::new(&result.method),
            Cell::new(&format!("{:.2}", result.precision_at_1)),
            Cell::new(&format!("{:.2}", result.precision_at_3)),
            Cell::new(&format!("{:.2}", result.precision_at_5)),
            Cell::new(&format!("{:.2}", result.precision_at_10)),
            Cell::new(&format!(
                "{}/{}",
                result.relevant_found, result.total_relevant
            )),
            Cell::new(&format!("{:.1}", result.execution_time_ms)),
        ]));
    }

    // 打印表格
    println!("\n📊 搜索方法对比结果:");
    table.printstd();

    // 计算平均指标
    let llm_results: Vec<_> = results
        .iter()
        .filter(|r| r.method == "LLM增强搜索")
        .collect();
    let no_llm_results: Vec<_> = results.iter().filter(|r| r.method == "非LLM搜索").collect();

    if !llm_results.is_empty() && !no_llm_results.is_empty() {
        // LLM平均指标
        let avg_llm_p1 =
            llm_results.iter().map(|r| r.precision_at_1).sum::<f64>() / llm_results.len() as f64;
        let avg_llm_p3 =
            llm_results.iter().map(|r| r.precision_at_3).sum::<f64>() / llm_results.len() as f64;
        let avg_llm_p5 =
            llm_results.iter().map(|r| r.precision_at_5).sum::<f64>() / llm_results.len() as f64;
        let avg_llm_p10 =
            llm_results.iter().map(|r| r.precision_at_10).sum::<f64>() / llm_results.len() as f64;
        let avg_llm_time =
            llm_results.iter().map(|r| r.execution_time_ms).sum::<f64>() / llm_results.len() as f64;

        // 非LLM平均指标
        let avg_no_llm_p1 = no_llm_results.iter().map(|r| r.precision_at_1).sum::<f64>()
            / no_llm_results.len() as f64;
        let avg_no_llm_p3 = no_llm_results.iter().map(|r| r.precision_at_3).sum::<f64>()
            / no_llm_results.len() as f64;
        let avg_no_llm_p5 = no_llm_results.iter().map(|r| r.precision_at_5).sum::<f64>()
            / no_llm_results.len() as f64;
        let avg_no_llm_p10 = no_llm_results
            .iter()
            .map(|r| r.precision_at_10)
            .sum::<f64>()
            / no_llm_results.len() as f64;
        let avg_no_llm_time = no_llm_results
            .iter()
            .map(|r| r.execution_time_ms)
            .sum::<f64>()
            / no_llm_results.len() as f64;

        // 创建平均值表格
        let mut avg_table = Table::new();
        avg_table.set_format(*format::consts::FORMAT_BOX_CHARS);

        // 添加表头
        avg_table.add_row(Row::new(vec![
            Cell::new("搜索方法"),
            Cell::new("平均P@1"),
            Cell::new("平均P@3"),
            Cell::new("平均P@5"),
            Cell::new("平均P@10"),
            Cell::new("平均耗时(ms)"),
        ]));

        // 添加LLM行
        avg_table.add_row(Row::new(vec![
            Cell::new("LLM增强搜索"),
            Cell::new(&format!("{:.4}", avg_llm_p1)),
            Cell::new(&format!("{:.4}", avg_llm_p3)),
            Cell::new(&format!("{:.4}", avg_llm_p5)),
            Cell::new(&format!("{:.4}", avg_llm_p10)),
            Cell::new(&format!("{:.1}", avg_llm_time)),
        ]));

        // 添加非LLM行
        avg_table.add_row(Row::new(vec![
            Cell::new("非LLM搜索"),
            Cell::new(&format!("{:.4}", avg_no_llm_p1)),
            Cell::new(&format!("{:.4}", avg_no_llm_p3)),
            Cell::new(&format!("{:.4}", avg_no_llm_p5)),
            Cell::new(&format!("{:.4}", avg_no_llm_p10)),
            Cell::new(&format!("{:.1}", avg_no_llm_time)),
        ]));

        // 添加提升百分比行
        let p1_improve = if avg_no_llm_p1 > 0.0 {
            (avg_llm_p1 / avg_no_llm_p1 - 1.0) * 100.0
        } else {
            0.0
        };
        let p3_improve = if avg_no_llm_p3 > 0.0 {
            (avg_llm_p3 / avg_no_llm_p3 - 1.0) * 100.0
        } else {
            0.0
        };
        let p5_improve = if avg_no_llm_p5 > 0.0 {
            (avg_llm_p5 / avg_no_llm_p5 - 1.0) * 100.0
        } else {
            0.0
        };
        let p10_improve = if avg_no_llm_p10 > 0.0 {
            (avg_llm_p10 / avg_no_llm_p10 - 1.0) * 100.0
        } else {
            0.0
        };
        let time_increase = if avg_no_llm_time > 0.0 {
            (avg_llm_time / avg_no_llm_time - 1.0) * 100.0
        } else {
            0.0
        };

        avg_table.add_row(Row::new(vec![
            Cell::new("提升百分比"),
            Cell::new(&format!("{:+.1}%", p1_improve)),
            Cell::new(&format!("{:+.1}%", p3_improve)),
            Cell::new(&format!("{:+.1}%", p5_improve)),
            Cell::new(&format!("{:+.1}%", p10_improve)),
            Cell::new(&format!("{:+.1}%", time_increase)),
        ]));

        // 打印平均指标表格
        println!("\n📈 平均性能指标:");
        avg_table.printstd();
    }
}

// 安全截断UTF-8字符串
fn truncate_utf8(s: &str, max_chars: usize) -> String {
    let chars: Vec<char> = s.chars().collect();

    if chars.len() <= max_chars {
        s.to_string()
    } else {
        chars.into_iter().take(max_chars).collect::<String>() + "..."
    }
}
