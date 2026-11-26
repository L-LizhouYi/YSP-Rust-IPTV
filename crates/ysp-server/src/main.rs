//! 央视频直播/点播平台（Rust 版）。

mod app;
mod epgxml;
mod m3u;
mod playurl;
mod proxy;
mod routes;
mod settings;
mod web;

use std::sync::Arc;
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use serde_json::json;
use tokio::sync::Semaphore;
use ysp_core::{
    config,
    media::{self, DecryptMode},
};

use crate::{app::AppState, settings::Settings};

#[derive(Parser)]
#[command(name = "ysp-server", version, about = "央视频直播/点播平台")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 启动 HTTP 服务
    Serve {
        #[arg(long, default_value = "0.0.0.0")]
        host: String,
        #[arg(long, default_value_t = 8525)]
        port: u16,
        /// 启动时预热全部非 VIP 频道
        #[arg(long)]
        warm: bool,
        /// playurl 后台刷新间隔（秒）
        #[arg(long, default_value_t = config::PLAYURL_REFRESH_SECS)]
        refresh: u64,
    },
    /// 列出频道
    Channels {
        #[arg(long)]
        all: bool,
        #[arg(long)]
        json: bool,
    },
    /// 获取单个频道播放地址
    Playurl {
        channel: String,
        #[arg(long, default_value = "")]
        defn: String,
    },
    /// 打印节目单
    Epg {
        channel: String,
        #[arg(long, default_value = "")]
        date: String,
    },
    /// 批量探测可播性
    Probe {
        #[arg(long)]
        all: bool,
        #[arg(long, default_value = "")]
        filter: String,
        #[arg(long, default_value = "")]
        out: String,
        #[arg(long, default_value_t = 4)]
        workers: usize,
    },
    /// 解密一个 TS 分片（用于和 Python 版逐字节对比）
    Decrypt {
        /// enc2 | h5e | none
        mode: String,
        input: String,
        output: String,
    },
}

#[tokio::main]
async fn main() {
    init_tracing();
    let cli = Cli::parse();
    if let Err(err) = run(cli).await {
        eprintln!("错误: {err}");
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    match cli.command {
        Command::Serve {
            host,
            port,
            warm,
            refresh,
        } => serve(host, port, warm, refresh).await,
        Command::Channels { all, json } => list_channels(all, json).await,
        Command::Playurl { channel, defn } => show_playurl(&channel, &defn).await,
        Command::Epg { channel, date } => show_epg(&channel, &date).await,
        Command::Probe {
            all,
            filter,
            out,
            workers,
        } => probe(all, &filter, &out, workers).await,
        Command::Decrypt {
            mode,
            input,
            output,
        } => decrypt_file(&mode, &input, &output),
    }
}

fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}

fn base_settings() -> Arc<Settings> {
    Arc::new(Settings::from_env().normalized())
}

async fn serve(
    host: String,
    port: u16,
    warm: bool,
    refresh: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut cfg = Settings::from_env().normalized();
    cfg.host = host;
    cfg.port = port;
    cfg.refresh = Duration::from_secs(refresh.max(60));
    let cfg = Arc::new(cfg);

    let state = AppState::build(Arc::clone(&cfg))?;
    state.catalog.ensure_loaded(false).await?;
    let total = state.catalog.count().await;
    let visible = state.catalog.visible(false).await?;
    tracing::info!("频道: {} 个可播（共 {}）", visible.len(), total);
    tracing::info!(
        "缓存预算: 全局 {} MB / 单频道 {} MB",
        cfg.segment_mem_limit / 1024 / 1024,
        cfg.channel_mem_limit / 1024 / 1024
    );

    if warm {
        let warm_list = visible.clone();
        let playurls = Arc::clone(&state.playurls);
        tokio::spawn(async move {
            playurls.warm(warm_list, 4, false).await;
            tracing::info!("预热完成");
        });
    }

    state.playurls.start_background(
        Arc::clone(&state.catalog),
        cfg.refresh,
        cfg.refresh_workers,
    );
    state.proxies.start_reaper(Duration::from_secs(60));

    // 启动后预热 XMLTV，之后每 4 分钟刷新一次（缓存 5 分钟），
    // 这样 IPTV 客户端请求 /epg.xml 时基本都命中缓存。
    {
        let epg_state = Arc::clone(&state);
        tokio::spawn(async move {
            loop {
                let date = ysp_core::util::today_compact();
                epgxml::render_cached(&epg_state, &date).await;
                tokio::time::sleep(Duration::from_secs(240)).await;
            }
        });
    }

    let listener = tokio::net::TcpListener::bind((cfg.host.as_str(), cfg.port)).await?;
    tracing::info!("listening on http://{}:{}", cfg.host, cfg.port);
    axum::serve(listener, routes::router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    tracing::info!("bye");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{signal, SignalKind};
        if let Ok(mut stream) = signal(SignalKind::terminate()) {
            stream.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

async fn list_channels(all: bool, as_json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let state = AppState::build(base_settings())?;
    let channels = state.catalog.visible(all).await?;
    if as_json {
        println!("{}", serde_json::to_string_pretty(&channels)?);
        return Ok(());
    }
    for ch in &channels {
        let tier = match ch.tier.as_str() {
            "free" => "免费",
            "preview" => "限免",
            "vip" => "VIP",
            other => other,
        };
        println!(
            "{:<16} {:<24} {:<4} {}  {}",
            ch.id(),
            ch.name,
            tier,
            ch.livepid,
            ch.cnlid
        );
    }
    println!("\n共 {} 个频道", channels.len());
    Ok(())
}

async fn show_playurl(channel: &str, defn: &str) -> Result<(), Box<dyn std::error::Error>> {
    let state = AppState::build(base_settings())?;
    let ch = state
        .catalog
        .get(channel)
        .await?
        .ok_or_else(|| format!("未知频道: {channel}"))?;
    let info = if defn.is_empty() {
        state.playurls.get(&ch, true).await?
    } else {
        state.player.live_info(&ch, defn).await?
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "channel": ch.to_json(Some(true), &info.defn, &info.formats),
            "play": info.to_json(),
        }))?
    );
    Ok(())
}

async fn show_epg(channel: &str, date: &str) -> Result<(), Box<dyn std::error::Error>> {
    let state = AppState::build(base_settings())?;
    let ch = state
        .catalog
        .get(channel)
        .await?
        .ok_or_else(|| format!("未知频道: {channel}"))?;
    let programs = state.epg.programs(&ch.livepid, date).await?;
    for p in programs.iter() {
        println!("{}-{}  {}", p.start_time, p.end_time, p.name);
    }
    Ok(())
}

async fn probe(
    all: bool,
    filter: &str,
    out: &str,
    workers: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let state = AppState::build(base_settings())?;
    let mut channels = state.catalog.visible(all).await?;
    if !filter.is_empty() {
        channels.retain(|c| c.name.contains(filter));
    }
    tracing::info!("开始探测 {} 个频道（{} 并发）", channels.len(), workers);

    let sem = Arc::new(Semaphore::new(workers.max(1)));
    let mut handles = Vec::with_capacity(channels.len());
    for ch in channels {
        let sem = Arc::clone(&sem);
        let player = Arc::clone(&state.player);
        handles.push(tokio::spawn(async move {
            let Ok(_permit) = sem.acquire_owned().await else {
                return (ch, Err("信号量关闭".to_string()), Duration::ZERO);
            };
            let started = Instant::now();
            let result = player
                .pick_playable(&ch)
                .await
                .map_err(|e| e.to_string());
            (ch, result, started.elapsed())
        }));
    }

    let mut rows = Vec::new();
    let mut ok = 0usize;
    for handle in handles {
        let (ch, result, elapsed) = handle.await?;
        let mut row = json!({
            "name": ch.name,
            "id": ch.id(),
            "livepid": ch.livepid,
            "cnlid": ch.cnlid,
            "tier": ch.tier,
            "ms": elapsed.as_millis() as u64,
        });
        match result {
            Ok(info) => {
                ok += 1;
                row["ok"] = json!(true);
                row["defn"] = json!(info.defn);
                row["cdn"] = json!(info.cdn);
                row["formats"] = serde_json::to_value(&info.formats)?;
                println!("{:<24} {:<5} OK", ch.name, info.defn);
            }
            Err(message) => {
                row["ok"] = json!(false);
                row["error"] = json!(message);
                println!("{:<24} {:<5} FAIL {}", ch.name, "-", message);
            }
        }
        rows.push(row);
    }

    if !out.is_empty() {
        std::fs::write(out, serde_json::to_string_pretty(&rows)?)?;
        tracing::info!("结果写入 {out}");
    }
    tracing::info!("完成: {}/{} 可播", ok, rows.len());
    Ok(())
}

fn decrypt_file(
    mode: &str,
    input: &str,
    output: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let mode = DecryptMode::parse(mode).ok_or_else(|| format!("未知解密模式: {mode}"))?;
    let data = std::fs::read(input)?;
    let started = Instant::now();
    let (plain, stats) = media::decrypt(mode, &data);
    std::fs::write(output, &plain)?;
    println!(
        "{} -> {} bytes ({:.1} ms) nals={} cells={} pes={} decrypted={}",
        data.len(),
        plain.len(),
        started.elapsed().as_secs_f64() * 1000.0,
        stats.nals,
        stats.cells,
        stats.pes,
        stats.decrypted
    );
    Ok(())
}
