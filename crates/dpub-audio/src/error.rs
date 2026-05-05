use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("ffmpeg binary not found on PATH (try `brew install ffmpeg` or `apt install ffmpeg`)")]
    FfmpegMissing,

    #[error("failed to spawn ffmpeg: {0}")]
    Spawn(#[source] std::io::Error),

    #[error(
        "ffmpeg failed re-encoding {input} → {output} (exit code {})",
        exit_code.map_or_else(|| "?".into(), |c| c.to_string())
    )]
    Failed {
        input: PathBuf,
        output: PathBuf,
        exit_code: Option<i32>,
    },
}

pub type Result<T> = std::result::Result<T, Error>;
