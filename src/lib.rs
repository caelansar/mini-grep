use clap::Parser;
use colored::*;
use futures::{Future, StreamExt};
use itertools::Itertools;
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use regex::Regex;
use std::{
    fs::File,
    io::{self, BufRead, BufReader, Write},
    ops::Range,
    path::{Path, PathBuf},
    pin::Pin,
};
use tokio::{
    fs,
    io::{AsyncBufReadExt, AsyncWriteExt},
};

mod error;
pub use error::GrepError;

pub type StrategyFn = fn(&Path, &mut dyn BufRead, &Regex, &mut dyn Write) -> Result<(), GrepError>;

/// mini grep
#[derive(Parser, Debug)]
#[clap(version = "0.1", author = "Caelansar")]
pub struct GrepConfig {
    /// A regular expression used for searching
    pattern: String,
    /// A pattern used during the search of the input
    glob: String,
}

impl GrepConfig {
    pub fn grep(&self) -> Result<(), GrepError> {
        self.grep_with(default_strategy)
    }

    pub fn grep_with(&self, strategy: StrategyFn) -> Result<(), GrepError> {
        let regex = Regex::new(&self.pattern)?;
        let files: Vec<_> = glob::glob(&self.glob)?.collect();
        files.into_par_iter().for_each(|v| {
            if let Ok(filename) = v {
                if let Ok(file) = File::open(&filename) {
                    let mut reader = BufReader::new(file);
                    let mut stdout = io::stdout();

                    if let Err(e) = strategy(filename.as_path(), &mut reader, &regex, &mut stdout) {
                        println!("Internal error: {:?}", e);
                    }
                }
            }
        });
        Ok(())
    }

    pub async fn async_grep(&self) -> Result<(), GrepError> {
        self.async_grep_with(default_async_strategy).await
    }

    pub async fn async_grep_with<'a, F, Fut>(&self, strategy: F) -> Result<(), GrepError>
    where
        F: FnOnce(
                PathBuf,
                Pin<Box<dyn tokio::io::AsyncBufRead + 'a>>,
                Regex,
                Pin<Box<dyn tokio::io::AsyncWrite + 'a>>,
            ) -> Fut
            + Copy,
        Fut: Future<Output = Result<(), GrepError>>,
    {
        let files: Vec<_> = glob::glob(&self.glob)?.collect();
        let regex = Regex::new(&self.pattern).unwrap();

        let mut stream = futures::stream::iter(
            files
                .into_iter()
                .filter(|x| {
                    if let Ok(path) = x {
                        path.is_file()
                    } else {
                        false
                    }
                })
                .map(|x| async move {
                    let path = x.unwrap();
                    let res = fs::File::open(&path).await.unwrap();
                    (path, res)
                }),
        )
        .buffer_unordered(8);

        while let Some(pair) = stream.next().await {
            let reader = tokio::io::BufReader::new(pair.1);
            let writer = tokio::io::stdout();
            let regex = regex.clone();
            let _ = strategy(pair.0, Box::pin(reader), regex, Box::pin(writer)).await;
        }

        Ok(())
    }
}

pub fn default_strategy(
    path: &Path,
    reader: &mut dyn BufRead,
    pattern: &Regex,
    writer: &mut dyn Write,
) -> Result<(), GrepError> {
    let matches: String = reader
        .lines()
        .enumerate()
        .map(|(lineno, line)| {
            line.ok()
                .map(|line| {
                    pattern
                        .find(&line)
                        .map(|m| format_line(&line, lineno + 1, m.range()))
                })
                .flatten()
        })
        .filter_map(|v| v.ok_or(()).ok())
        .join("\n");

    if !matches.is_empty() {
        writer.write_all(path.display().to_string().green().as_bytes())?;
        writer.write_all(b"\n")?;
        writer.write_all(matches.as_bytes())?;
        writer.write_all(b"\n")?;
    }

    Ok(())
}

pub async fn default_async_strategy<'a>(
    path: PathBuf,
    reader: Pin<Box<dyn tokio::io::AsyncBufRead>>,
    pattern: Regex,
    mut writer: Pin<Box<dyn tokio::io::AsyncWrite + 'a>>,
) -> Result<(), GrepError> {
    let mut lines = reader.lines();
    let mut lineno = 0;

    let mut finds = Vec::new();
    while let Some(line) = lines.next_line().await? {
        lineno += 1;
        if let Some(f) = pattern.find(&line) {
            finds.push(format_line(&line, lineno, f.range()))
        }
    }
    let matches = finds.join("\n");

    if !matches.is_empty() {
        writer
            .write_all(path.display().to_string().green().as_bytes())
            .await?;
        writer.write_all(b"\n").await?;
        writer.write_all(matches.as_bytes()).await?;
        writer.write_all(b"\n").await?;
    }

    Ok(())
}

pub fn format_line(line: &str, lineno: usize, range: Range<usize>) -> String {
    let Range { start, end } = range;
    let prefix = &line[..start];
    format!(
        "{0: >6}:{1: <3} {2}{3}{4}",
        lineno.to_string().blue(),
        (prefix.chars().count() + 1).to_string().cyan(),
        prefix,
        &line[start..end].red(),
        &line[end..]
    )
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn format_line_should_work() {
        let result = format_line("Hello, Cae", 1000, 7..10);
        let expected = format!(
            "{0: >6}:{1: <3} Hello, {2}",
            "1000".blue(),
            "8".cyan(),
            "Cae".red()
        );
        assert_eq!(result, expected);
    }

    #[test]
    fn default_strategy_should_work() {
        let path = Path::new("src/main.rs");
        let input = b"hello world!\nbye world!";
        let mut reader = BufReader::new(&input[..]);
        let pattern = Regex::new(r"wo\w+").unwrap();
        let mut writer = Vec::new();
        default_strategy(path, &mut reader, &pattern, &mut writer).unwrap();
        let result = String::from_utf8(writer).unwrap();
        let expected = [
            String::from("src/main.rs"),
            format_line("hello world!", 1, 6..11),
            format_line("bye world!\n", 2, 4..9),
        ];

        assert_eq!(result, expected.join("\n"));
    }

    #[tokio::test]
    async fn default_async_strategy_should_work() {
        let path = Path::new("src/main.rs");
        let input = b"hello world!\nbye world!";
        let reader = tokio::io::BufReader::new(&input[..]);
        let pattern = Regex::new(r"wo\w+").unwrap();

        let mut writer = Vec::new();
        let pin_writer = Box::pin(&mut writer);

        default_async_strategy(path.to_path_buf(), Box::pin(reader), pattern, pin_writer)
            .await
            .unwrap();

        let result = String::from_utf8(writer).unwrap();
        let expected = [
            String::from("src/main.rs"),
            format_line("hello world!", 1, 6..11),
            format_line("bye world!\n", 2, 4..9),
        ];

        assert_eq!(result, expected.join("\n"));
    }
}
