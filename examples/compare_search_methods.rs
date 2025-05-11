use cratespro_search::search::{
    RecommendCrate, SearchModule, SearchSortCriteria, TraditionalSearchModule,
};
use dotenv::dotenv;
use prettytable::{format, Cell, Row, Table};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::env;
use std::fs::File;
use std::io::{BufReader, Write};
use std::path::Path;
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

    // 创建不同的搜索模块
    let llm_search = SearchModule::new(&pg_client).await;
    let traditional_search = TraditionalSearchModule::new(&pg_client).await;

    // 加载测试用例
    let test_cases = load_test_cases();
    println!("📋 已加载 {} 个测试用例", test_cases.len());

    // 存储比较结果
    let mut results = Vec::new();

    // 执行测试
    for test_case in &test_cases {
        println!(
            "\n📝 测试用例: {} - \"{}\"",
            test_case.description, test_case.query
        );

        // 创建相关包集合用于评估
        let relevant_packages: HashSet<String> = test_case
            .relevant_packages
            .iter()
            .map(|p| p.to_lowercase())
            .collect();

        println!("  👀 相关包集合: {:?}", test_case.relevant_packages);

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

        // 评估LLM搜索结果
        let llm_metrics = evaluate_results(&llm_results, &relevant_packages);
        println!("    ⏱️ 耗时: {:.2?}", llm_duration);
        println!(
            "    P@1: {:.2}, P@3: {:.2}, P@5: {:.2}, P@10: {:.2}, 召回率: {:.2}",
            llm_metrics.0, llm_metrics.1, llm_metrics.2, llm_metrics.3, llm_metrics.4
        );

        // 打印LLM搜索的前5个结果
        print_results("LLM增强搜索", &llm_results, &relevant_packages, 5);

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

        // 评估传统搜索结果
        let trad_metrics = evaluate_results(&trad_results, &relevant_packages);
        println!("    ⏱️ 耗时: {:.2?}", trad_duration);
        println!(
            "    P@1: {:.2}, P@3: {:.2}, P@5: {:.2}, P@10: {:.2}, 召回率: {:.2}",
            trad_metrics.0, trad_metrics.1, trad_metrics.2, trad_metrics.3, trad_metrics.4
        );

        // 打印传统搜索的前5个结果
        print_results("传统搜索", &trad_results, &relevant_packages, 5);

        // 记录结果
        results.push(ComparisonResult {
            query: test_case.query.clone(),
            description: test_case.description.clone(),
            method: "LLM增强搜索".to_string(),
            precision_at_1: llm_metrics.0,
            precision_at_3: llm_metrics.1,
            precision_at_5: llm_metrics.2,
            precision_at_10: llm_metrics.3,
            recall: llm_metrics.4,
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
            recall: trad_metrics.4,
            latency_ms: trad_duration.as_millis() as f64,
        });
    }

    // 生成报告
    generate_report(&results);

    // 保存结果到文件
    if let Ok(mut file) = File::create("search_comparison.json") {
        let json = serde_json::to_string_pretty(&results)?;
        file.write_all(json.as_bytes())?;
        println!("\n💾 结果已保存到 search_comparison.json");
    }

    println!("\n✅ 对比实验完成");
    Ok(())
}

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

fn evaluate_results(
    results: &[RecommendCrate],
    relevant_set: &HashSet<String>,
) -> (f64, f64, f64, f64, f64) {
    // 计算所有相关性指标
    let relevant_flags: Vec<bool> = results
        .iter()
        .map(|r| relevant_set.contains(&r.name.to_lowercase()))
        .collect();

    // 计算P@K指标
    let p1 = calculate_precision_at_k(&relevant_flags, 1);
    let p3 = calculate_precision_at_k(&relevant_flags, 3);
    let p5 = calculate_precision_at_k(&relevant_flags, 5);
    let p10 = calculate_precision_at_k(&relevant_flags, 10);

    // 计算召回率
    let found_relevant = results
        .iter()
        .filter(|r| relevant_set.contains(&r.name.to_lowercase()))
        .count();
    let recall = if relevant_set.is_empty() {
        0.0
    } else {
        found_relevant as f64 / relevant_set.len() as f64
    };

    (p1, p3, p5, p10, recall)
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

fn print_results(
    method: &str,
    results: &[RecommendCrate],
    relevant_set: &HashSet<String>,
    count: usize,
) {
    println!("    📋 {}的前{}个结果:", method, count);

    for (i, result) in results.iter().take(count).enumerate() {
        let is_relevant = relevant_set.contains(&result.name.to_lowercase());
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

fn generate_report(results: &[ComparisonResult]) {
    // 创建表格
    let mut table = Table::new();
    table.set_format(*format::consts::FORMAT_BOX_CHARS);

    // 添加表头
    table.add_row(Row::new(vec![
        Cell::new("查询"),
        Cell::new("方法"),
        Cell::new("P@1"),
        Cell::new("P@5"),
        Cell::new("P@10"),
        Cell::new("召回率"),
        Cell::new("延迟(ms)"),
    ]));

    // 添加数据行
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

    // 打印表格
    println!("\n📊 搜索方法对比结果:");
    table.printstd();

    // 计算平均值
    let llm_results: Vec<_> = results
        .iter()
        .filter(|r| r.method == "LLM增强搜索")
        .collect();
    let trad_results: Vec<_> = results.iter().filter(|r| r.method == "传统搜索").collect();

    if !llm_results.is_empty() && !trad_results.is_empty() {
        // 计算平均值
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

        // 计算提升百分比
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
