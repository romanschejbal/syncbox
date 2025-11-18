use clap::Parser;
use std::error::Error;
use syncbox::{config::Args, sync_engine::SyncEngine, utils::HumanBytes};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
    dotenvy::from_filename(".env.syncbox").ok();
    dotenvy::dotenv().ok();

    let args = Args::parse();

    std::env::set_current_dir(args.directory.clone())?;

    // Create and run the sync engine
    let sync_engine = SyncEngine::new(args);
    let result = sync_engine.sync().await?;

    println!(
        "✨ Done. Transferred {} files ({}) in {:.2?}s",
        result.files_uploaded,
        result.bytes_transferred.to_human_size(),
        result.duration.as_secs_f64()
    );

    println!("📊 Summary:");
    println!("   Files uploaded: {}", result.files_uploaded);
    println!("   Files removed: {}", result.files_removed);
    println!("   Directories created: {}", result.directories_created);
    println!(
        "   Data transferred: {}",
        result.bytes_transferred.to_human_size()
    );

    if result.had_errors {
        return Err("Some operations failed during sync".into());
    }

    Ok(())
}
