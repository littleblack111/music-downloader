use colored::Colorize;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DownloadError {
    #[error("Network request failed: {0}")]
    NetworkError(#[from] reqwest::Error),

    #[error("Failed to parse JSON response: {0}")]
    JsonParseError(#[from] serde_json::Error),

    #[error("Failed to decode base64 manifest: {0}")]
    Base64DecodeError(#[from] base64::DecodeError),

    #[error("Failed to parse XML manifest: {0}")]
    XmlParseError(String),

    #[error("Track not found: {0}")]
    TrackNotFound(String),

    #[error("Album not found: {0}")]
    AlbumNotFound(String),

    #[error("Artist not found: {0}")]
    ArtistNotFound(String),

    #[error("Playlist not found: {0}")]
    PlaylistNotFound(String),

    #[error("No download URL available for the requested quality")]
    NoDownloadUrl,

    #[error("Rate limited. Please wait before making more requests")]
    RateLimited,

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Download failed: {0}")]
    DownloadFailed(String),

    #[error("Segment download failed: {0}")]
    SegmentDownloadFailed(String),

    #[error("CSV parsing error: {0}")]
    CsvError(String),

    #[error("Service unavailable: {0}")]
    ServiceUnavailable(String),
}

impl From<csv::Error> for DownloadError {
    fn from(err: csv::Error) -> Self {
        DownloadError::CsvError(err.to_string())
    }
}

impl DownloadError {
    pub fn pretty_print(&self) {
        let error_type = match self {
            DownloadError::NetworkError(_) => "Network Error",
            DownloadError::JsonParseError(_) => "Parse Error",
            DownloadError::Base64DecodeError(_) => "Decode Error",
            DownloadError::XmlParseError(_) => "XML Error",
            DownloadError::TrackNotFound(_) => "Not Found",
            DownloadError::AlbumNotFound(_) => "Not Found",
            DownloadError::ArtistNotFound(_) => "Not Found",
            DownloadError::PlaylistNotFound(_) => "Not Found",
            DownloadError::NoDownloadUrl => "Download Error",
            DownloadError::RateLimited => "Rate Limit",
            DownloadError::IoError(_) => "IO Error",
            DownloadError::DownloadFailed(_) => "Download Error",
            DownloadError::SegmentDownloadFailed(_) => "Download Error",
            DownloadError::CsvError(_) => "CSV Error",
            DownloadError::ServiceUnavailable(_) => "Service Error",
        };

        eprintln!();
        eprintln!(
            "{}",
            "╭─────────────────────────────────────╮".bright_red()
        );
        eprintln!(
            "{} {} {:>24} {}",
            "│".bright_red(),
            error_type
                .red()
                .bold(),
            "",
            "│".bright_red()
        );
        eprintln!(
            "{}",
            "├─────────────────────────────────────┤".bright_red()
        );
        eprintln!(
            "{} {} {}",
            "│".bright_red(),
            self.to_string()
                .white(),
            "│".bright_red()
        );
        eprintln!(
            "{}",
            "╰─────────────────────────────────────╯".bright_red()
        );
        eprintln!();
    }
}

pub type Result<T> = std::result::Result<T, DownloadError>;
