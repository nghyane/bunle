use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "bunle", about = "Bunle — Image container format")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Pack a directory of images into BNL
    Pack {
        /// Input directory
        dir: PathBuf,
        /// Output file
        #[arg(short, long)]
        output: PathBuf,
        /// WebP quality (1-100). Auto-passthrough if source is already smaller
        #[arg(short, long, default_value = "80")]
        quality: u8,
        /// Skip WebP cover (disables polyglot image/webp detection)
        #[arg(long)]
        no_cover: bool,
    },
    /// Show BNL file info
    Info {
        /// BNL file
        file: PathBuf,
        /// Emit JSON instead of human-readable text
        #[arg(long)]
        json: bool,
    },
    /// Extract a single page from BNL
    Extract {
        /// BNL file
        file: PathBuf,
        /// Page index (0-based)
        page: usize,
        /// Output file
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Unpack all pages from BNL into a directory
    Unpack {
        /// BNL file
        file: PathBuf,
        /// Output directory (created if missing)
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Validate a BNL archive: structural check (header + index bounds + dims)
    Validate {
        /// BNL file
        file: PathBuf,
    },
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Command::Pack { dir, output, quality, no_cover } => cmd_pack(&dir, &output, quality, !no_cover),
        Command::Info { file, json } => cmd_info(&file, json),
        Command::Extract { file, page, output } => cmd_extract(&file, page, &output),
        Command::Unpack { file, output } => cmd_unpack(&file, &output),
        Command::Validate { file } => cmd_validate(&file),
    }
}

fn cmd_pack(dir: &PathBuf, output: &PathBuf, quality: u8, cover: bool) {
    match bunle::pack_dir(dir, output, quality, cover) {
        Ok(index) => {
            println!("Packed {} pages → {}", index.pages.len(), output.display());
            let total: u64 = index.pages.iter().map(|p| p.size as u64).sum();
            println!("Total size: {} bytes", total + bunle::MCZIndex::data_offset(index.pages.len() as u16) as u64);
            for p in &index.pages {
                println!("  {:>3}: {:>4}×{:<4} {:>4} {:>8} bytes",
                    p.index, p.width, p.height, p.format, p.size);
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

fn cmd_info(file: &PathBuf, json: bool) {
    let data = match std::fs::read(file) {
        Ok(d) => d,
        Err(e) => { eprintln!("error: {e}"); std::process::exit(1); }
    };
    match bunle::read_index(&data) {
        Ok(index) => {
            if json {
                print_info_json(&data, &index);
            } else {
                print_info_text(&data, &index);
            }
        }
        Err(e) => { eprintln!("error: {e}"); std::process::exit(1); }
    }
}

fn print_info_text(data: &[u8], index: &bunle::MCZIndex) {
    let total: u64 = index.pages.iter().map(|p| p.size as u64).sum();
    println!("BNL v{} — {} pages, {} bytes", index.version, index.pages.len(), data.len());
    println!("Index size: {} bytes", bunle::MCZIndex::data_offset(index.pages.len() as u16));
    println!("Data size:  {} bytes", total);
    println!();
    for p in &index.pages {
        println!("  {:>3}: {:>4}×{:<4} {:>4}  offset={:<8} size={}",
            p.index, p.width, p.height, p.format, p.offset, p.size);
    }
}

fn print_info_json(data: &[u8], index: &bunle::MCZIndex) {
    let mut out = String::new();
    out.push('{');
    out.push_str(&format!("\"version\":{},", index.version));
    out.push_str(&format!("\"page_count\":{},", index.pages.len()));
    out.push_str(&format!("\"total_bytes\":{},", data.len()));
    out.push_str("\"pages\":[");
    for (i, p) in index.pages.iter().enumerate() {
        if i > 0 { out.push(','); }
        let fmt = match p.format {
            bunle::ImageFormat::WebP => "webp",
            bunle::ImageFormat::Jpeg => "jpeg",
            bunle::ImageFormat::Jxl => "jxl",
        };
        out.push_str(&format!(
            "{{\"index\":{},\"width\":{},\"height\":{},\"format\":\"{}\",\"offset\":{},\"size\":{}}}",
            p.index, p.width, p.height, fmt, p.offset, p.size
        ));
    }
    out.push_str("]}");
    println!("{out}");
}

fn cmd_extract(file: &PathBuf, page: usize, output: &PathBuf) {
    let data = match std::fs::read(file) {
        Ok(d) => d,
        Err(e) => { eprintln!("error: {e}"); std::process::exit(1); }
    };
    let index = match bunle::read_index(&data) {
        Ok(i) => i,
        Err(e) => { eprintln!("error: {e}"); std::process::exit(1); }
    };
    match bunle::extract_page(&data, &index, page) {
        Ok(page_data) => {
            if let Err(e) = std::fs::write(output, page_data) {
                eprintln!("error: {e}"); std::process::exit(1);
            }
            let info = &index.pages[page];
            println!("Extracted page {} ({}×{} {}) → {}", page, info.width, info.height, info.format, output.display());
        }
        Err(e) => { eprintln!("error: {e}"); std::process::exit(1); }
    }
}

fn cmd_unpack(file: &PathBuf, output: &PathBuf) {
    let data = match std::fs::read(file) {
        Ok(d) => d,
        Err(e) => { eprintln!("error: {e}"); std::process::exit(1); }
    };
    match bunle::unpack(&data, output) {
        Ok(index) => {
            println!("Unpacked {} pages → {}", index.pages.len(), output.display());
        }
        Err(e) => { eprintln!("error: {e}"); std::process::exit(1); }
    }
}

fn cmd_validate(file: &PathBuf) {
    let data = match std::fs::read(file) {
        Ok(d) => d,
        Err(e) => { eprintln!("error: {e}"); std::process::exit(1); }
    };
    match bunle::validate(&data) {
        Ok(report) => {
            println!(
                "OK — BNL v{}, {} pages, {} bytes",
                report.version, report.page_count, report.total_bytes
            );
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}
