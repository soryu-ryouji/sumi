//! 启动设置：书库路径、监听端口、访问 token。
//! 桌面版由 Electron 通过命令行 / 环境变量传入（见 docs/architecture.md）。
//! CLI 定义集中在 Cli（clap derive，单一权威），Settings 只承接业务校验与加工。
//! token 与对账间隔只走环境变量、不进命令行命名空间：token 避免出现在进程列表（ps 可见）。

use clap::Parser;

pub const DEFAULT_PORT: u16 = 27381;

/// 命令行接口（每项支持同名 SUMI_* 环境变量回退，命令行优先于环境变量）
#[derive(Parser)]
#[command(name = "sumi-daemon", version, about = "sumi 书库后端服务")]
pub struct Cli {
    /// 书库根目录（--dump-openapi 模式豁免必填校验）
    #[arg(long, env = "SUMI_LIBRARY", required_unless_present = "dump_openapi")]
    pub library: Option<String>,

    /// 本地监听端口（被占用时回退动态分配）
    #[arg(long, env = "SUMI_PORT", default_value_t = DEFAULT_PORT)]
    pub port: u16,

    /// 局域网 web 查看托管的前端静态文件目录；不存在则不托管
    #[arg(long, env = "SUMI_WEB_DIST")]
    pub web_dist: Option<String>,

    /// 全局缓存父目录；缺省用系统缓存目录
    #[arg(long, env = "SUMI_CACHE_PARENT")]
    pub cache_parent: Option<String>,

    /// 打印 OpenAPI schema 到 stdout 后退出（openapi.json 的固化来源）
    #[arg(long)]
    pub dump_openapi: bool,
}

impl Cli {
    /// clap 解析入口（clap 依赖集中在 settings 模块，main 不经手）
    pub fn parse_args() -> Cli {
        <Cli as Parser>::parse()
    }
}

#[derive(Clone)]
pub struct Settings {
    pub library_root: String,
    pub port: u16,
    pub token: String,
    /// 元数据对账间隔（秒），0 关闭：`.sumi/metadata/` 的外部变更（网盘同步等）并入
    pub reconcile_interval_seconds: u64,
    /// 全局缓存父目录（桌面端设置面板配置，主进程经 --cache-parent 传入）；None 用系统缓存目录
    pub cache_parent: Option<String>,
    /// 局域网 web 查看托管的前端静态文件目录（Electron 传入 web/dist）；不存在则不托管
    pub web_dist: Option<String>,
}

impl Settings {
    /// Cli 已完成词法解析（library 非空、端口合法由 clap 保证），此处承接业务校验与 env-only 参数
    pub fn from_cli(cli: Cli) -> Settings {
        let library = cli.library.unwrap_or_default().trim().to_string();
        if library.is_empty() {
            eprintln!("书库路径为空（--library 或 SUMI_LIBRARY）");
            std::process::exit(2);
        }
        if !std::path::Path::new(&library).is_dir() {
            eprintln!("书库目录不存在: {library}");
            std::process::exit(2);
        }

        // token 只存在于进程环境中；未传入时（开发场景）生成随机 token 并打印到 stdout
        let token = std::env::var("SUMI_TOKEN").ok().unwrap_or_else(|| {
            let mut bytes = [0u8; 32];
            getrandom::fill(&mut bytes).expect("生成随机 token 失败");
            let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            println!("{hex}");
            hex
        });

        Settings {
            library_root: library,
            port: cli.port,
            token,
            reconcile_interval_seconds: std::env::var("SUMI_RECONCILE_INTERVAL")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(60),
            cache_parent: cli.cache_parent,
            web_dist: cli.web_dist,
        }
    }
}
