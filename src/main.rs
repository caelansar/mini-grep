use anyhow::Result;
use clap::Parser;
use grep::*;

#[tokio::main]
async fn main() -> Result<()> {
    let config: GrepConfig = GrepConfig::parse();

    // config.grep()?;
    //
    // config
    //     .async_grep_with(|path, _, pattern, _| async move {
    //         println!(
    //             "path: {} pattern: {}",
    //             path.as_path().to_str().unwrap(),
    //             pattern
    //         );
    //         Ok(())
    //     })
    //     .await
    //     .unwrap();
    //
    config.async_grep().await.unwrap();

    Ok(())
}
