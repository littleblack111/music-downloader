use crate::{
    error::{DownloadError, Result},
    types::*,
};
use colored::Colorize;
use futures::StreamExt;
use reqwest::{Client, StatusCode};
use std::{
    fs::File,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

const MAX_RETRIES: usize = 5;
const BACKOFF_DELAYS: [Duration; 5] = [
    Duration::from_secs(5),
    Duration::from_secs(15),
    Duration::from_secs(30),
    Duration::from_secs(60),
    Duration::from_secs(120),
];

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DirMode {
    Playlist,
    Album,
    Artist,
    Flat,
}

#[derive(Debug, Clone)]
pub struct DownloadProgressUpdate {
    pub bytes_downloaded: u64,
    pub total_bytes: Option<u64>,
    pub speed_bps: Option<f64>,
}

pub type ProgressCallback = Arc<Mutex<Box<dyn FnMut(DownloadProgressUpdate) + Send>>>;

#[derive(Clone)]
pub struct DownloadManager {
    client: Client,
    output_dir: PathBuf,
    album: Option<String>,
    playlist: Option<String>,
    dir_mode: DirMode,
}

impl DownloadManager {
    pub fn new(output_dir: &Path) -> Self {
        DownloadManager {
            client: Client::builder()
                .user_agent("SquidDownloader/0.1.0")
                .build()
                .unwrap(),
            output_dir: output_dir.to_path_buf(),
            album: None,
            playlist: None,
            dir_mode: DirMode::Playlist,
        }
    }

    pub fn with_album(mut self, album: Option<&str>) -> Self {
        self.album = album.map(|s| s.to_string());
        self
    }

    pub fn with_playlist(mut self, playlist: Option<&str>) -> Self {
        self.playlist = playlist.map(|s| s.to_string());
        self
    }

    pub fn with_dir_mode(mut self, mode: DirMode) -> Self {
        self.dir_mode = mode;
        self
    }

    fn sanitize_filename(name: &str) -> String {
        sanitize_filename::sanitize(name)
    }

    pub fn generate_filename(&self, track: &TrackInfo, quality: &AudioQuality) -> PathBuf {
        let artist = Self::sanitize_filename(&track.artist);
        let title = Self::sanitize_filename(&track.title);
        let extension = quality.file_extension();
        let filename = format!(
            "{} - {}.{}",
            artist, title, extension
        );

        match self.dir_mode {
            DirMode::Playlist => {
                if let Some(ref playlist) = self.playlist {
                    let playlist_dir = Self::sanitize_filename(playlist);
                    self.output_dir
                        .join(playlist_dir)
                        .join(filename)
                } else if let Some(ref album) = self.album {
                    let album_dir = format!(
                        "{} - {}",
                        artist,
                        Self::sanitize_filename(album)
                    );
                    self.output_dir
                        .join(album_dir)
                        .join(filename)
                } else {
                    self.output_dir
                        .join(filename)
                }
            }
            DirMode::Album => {
                if let Some(ref album) = self.album {
                    let album_dir = format!(
                        "{} - {}",
                        artist,
                        Self::sanitize_filename(album)
                    );
                    self.output_dir
                        .join(album_dir)
                        .join(filename)
                } else {
                    self.output_dir
                        .join(filename)
                }
            }
            DirMode::Artist => self
                .output_dir
                .join(&artist)
                .join(filename),
            DirMode::Flat => self
                .output_dir
                .join(filename),
        }
    }

    fn should_retry(status: StatusCode) -> bool {
        status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
    }

    pub async fn download_file(&self, url: &str, dest: &Path) -> Result<()> {
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let mut last_error = None;

        for attempt in 0..MAX_RETRIES {
            let response = self
                .client
                .get(url)
                .send()
                .await;

            match response {
                Ok(resp) => {
                    let status = resp.status();

                    if status.is_success() {
                        let mut file = File::create(dest)?;
                        let bytes = resp
                            .bytes()
                            .await?;
                        file.write_all(&bytes)?;
                        return Ok(());
                    }

                    let error = DownloadError::DownloadFailed(
                        format!(
                            "HTTP status: {}",
                            status
                        ),
                    );

                    if !Self::should_retry(status) || attempt == MAX_RETRIES - 1 {
                        return Err(error);
                    }

                    last_error = Some(error);
                }
                Err(e) => {
                    if attempt == MAX_RETRIES - 1 {
                        return Err(DownloadError::NetworkError(e));
                    }
                    last_error = Some(DownloadError::NetworkError(e));
                }
            }

            tokio::time::sleep(BACKOFF_DELAYS[attempt]).await;
        }

        Err(last_error.unwrap_or(DownloadError::DownloadFailed("Max retries exceeded".to_string())))
    }

    pub async fn download_file_with_progress<F>(&self, url: &str, dest: &Path, mut progress_callback: F) -> Result<u64>
    where
        F: FnMut(DownloadProgressUpdate) + Send,
    {
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let mut last_error = None;

        for attempt in 0..MAX_RETRIES {
            let response = self
                .client
                .get(url)
                .send()
                .await;

            match response {
                Ok(resp) => {
                    let status = resp.status();

                    if status.is_success() {
                        let total_size = resp.content_length();
                        let mut file = File::create(dest)?;
                        let mut downloaded: u64 = 0;
                        let start_time = Instant::now();
                        let mut last_update = Instant::now();

                        let mut stream = resp.bytes_stream();

                        while let Some(chunk_result) = stream
                            .next()
                            .await
                        {
                            let chunk = chunk_result.map_err(DownloadError::NetworkError)?;
                            file.write_all(&chunk)?;
                            downloaded += chunk.len() as u64;

                            let now = Instant::now();
                            let elapsed = now
                                .duration_since(last_update)
                                .as_millis();
                            if elapsed >= 100 || downloaded == total_size.unwrap_or(0) {
                                let total_elapsed = start_time
                                    .elapsed()
                                    .as_secs_f64();
                                let speed = if total_elapsed > 0.0 {
                                    Some((downloaded as f64) / total_elapsed)
                                } else {
                                    None
                                };

                                progress_callback(
                                    DownloadProgressUpdate {
                                        bytes_downloaded: downloaded,
                                        total_bytes: total_size,
                                        speed_bps: speed,
                                    },
                                );

                                last_update = now;
                            }
                        }

                        return Ok(downloaded);
                    }

                    let error = DownloadError::DownloadFailed(
                        format!(
                            "HTTP status: {}",
                            status
                        ),
                    );

                    if !Self::should_retry(status) || attempt == MAX_RETRIES - 1 {
                        return Err(error);
                    }

                    last_error = Some(error);
                }
                Err(e) => {
                    if attempt == MAX_RETRIES - 1 {
                        return Err(DownloadError::NetworkError(e));
                    }
                    last_error = Some(DownloadError::NetworkError(e));
                }
            }

            tokio::time::sleep(BACKOFF_DELAYS[attempt]).await;
        }

        Err(last_error.unwrap_or(DownloadError::DownloadFailed("Max retries exceeded".to_string())))
    }

    pub async fn download_segments(&self, segment_urls: &[String], dest: &Path) -> Result<()> {
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let mut file = File::create(dest)?;
        let total = segment_urls.len();

        for (i, url) in segment_urls
            .iter()
            .enumerate()
        {
            let mut segment_error = None;

            for attempt in 0..MAX_RETRIES {
                let response = self
                    .client
                    .get(url)
                    .send()
                    .await;

                match response {
                    Ok(resp) => {
                        let status = resp.status();

                        if status.is_success() {
                            let bytes = resp
                                .bytes()
                                .await?;
                            file.write_all(&bytes)?;
                            break;
                        }

                        let error = DownloadError::SegmentDownloadFailed(
                            format!(
                                "Segment {}/{} failed: {}",
                                i + 1,
                                total,
                                status
                            ),
                        );

                        if !Self::should_retry(status) || attempt == MAX_RETRIES - 1 {
                            let _ = std::fs::remove_file(dest);
                            return Err(error);
                        }

                        segment_error = Some(error);
                    }
                    Err(e) => {
                        let error = DownloadError::NetworkError(e);
                        if attempt == MAX_RETRIES - 1 {
                            let _ = std::fs::remove_file(dest);
                            return Err(error);
                        }
                        segment_error = Some(
                            DownloadError::SegmentDownloadFailed(
                                format!(
                                    "Segment {}/{} network error",
                                    i + 1,
                                    total
                                ),
                            ),
                        );
                    }
                }

                tokio::time::sleep(BACKOFF_DELAYS[attempt]).await;
            }

            if let Some(e) = segment_error {
                let _ = std::fs::remove_file(dest);
                return Err(e);
            }
        }

        Ok(())
    }

    pub async fn download_segments_with_progress<F>(&self, segment_urls: &[String], dest: &Path, mut progress_callback: F) -> Result<u64>
    where
        F: FnMut(DownloadProgressUpdate) + Send,
    {
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let mut file = File::create(dest)?;
        let total = segment_urls.len();
        let mut downloaded: u64 = 0;
        let start_time = Instant::now();
        let mut last_update = Instant::now();

        for (i, url) in segment_urls
            .iter()
            .enumerate()
        {
            let mut segment_error = None;

            for attempt in 0..MAX_RETRIES {
                let response = self
                    .client
                    .get(url)
                    .send()
                    .await;

                match response {
                    Ok(resp) => {
                        let status = resp.status();

                        if status.is_success() {
                            let bytes = resp
                                .bytes()
                                .await?;
                            file.write_all(&bytes)?;
                            downloaded += bytes.len() as u64;

                            let now = Instant::now();
                            let elapsed = now
                                .duration_since(last_update)
                                .as_millis();
                            if elapsed >= 100 || i == total - 1 {
                                let total_elapsed = start_time
                                    .elapsed()
                                    .as_secs_f64();
                                let speed = if total_elapsed > 0.0 {
                                    Some((downloaded as f64) / total_elapsed)
                                } else {
                                    None
                                };

                                progress_callback(
                                    DownloadProgressUpdate {
                                        bytes_downloaded: downloaded,
                                        total_bytes: None,
                                        speed_bps: speed,
                                    },
                                );

                                last_update = now;
                            }
                            break;
                        }

                        let error = DownloadError::SegmentDownloadFailed(
                            format!(
                                "Segment {}/{} failed: {}",
                                i + 1,
                                total,
                                status
                            ),
                        );

                        if !Self::should_retry(status) || attempt == MAX_RETRIES - 1 {
                            let _ = std::fs::remove_file(dest);
                            return Err(error);
                        }

                        segment_error = Some(error);
                    }
                    Err(e) => {
                        let error = DownloadError::NetworkError(e);
                        if attempt == MAX_RETRIES - 1 {
                            let _ = std::fs::remove_file(dest);
                            return Err(error);
                        }
                        segment_error = Some(
                            DownloadError::SegmentDownloadFailed(
                                format!(
                                    "Segment {}/{} network error",
                                    i + 1,
                                    total
                                ),
                            ),
                        );
                    }
                }

                tokio::time::sleep(BACKOFF_DELAYS[attempt]).await;
            }

            if let Some(e) = segment_error {
                let _ = std::fs::remove_file(dest);
                return Err(e);
            }
        }

        Ok(downloaded)
    }

    pub async fn download_from_manifest(&self, manifest: &DownloadManifest, track: &TrackInfo) -> Result<PathBuf> {
        let dest = self.generate_filename(
            track,
            &manifest.quality,
        );

        if let Some(ref bit_depth) = manifest.bit_depth {
            if let Some(ref sample_rate) = manifest.sample_rate {
                let sample_str = format_sample_rate(*sample_rate);
                println!(
                    "  {} {}, {}-bit FLAC",
                    "Format:".cyan(),
                    sample_str.white(),
                    bit_depth
                        .to_string()
                        .white()
                );
            }
        }

        println!(
            "  {} {}",
            "Output:".cyan(),
            dest.display()
                .to_string()
                .green()
        );

        let result = if let Some(ref url) = manifest.url {
            self.download_file(
                url, &dest,
            )
            .await
        } else if let Some(ref segments) = manifest.segment_urls {
            println!(
                "  {} {} segments",
                "Downloading:".cyan(),
                segments
                    .len()
                    .to_string()
                    .white()
            );
            self.download_segments(
                segments, &dest,
            )
            .await
        } else {
            Err(DownloadError::NoDownloadUrl)
        };

        if result.is_err() {
            let _ = std::fs::remove_file(&dest);
        }

        result?;
        Ok(dest)
    }

    pub async fn download_from_manifest_with_progress<F>(&self, manifest: &DownloadManifest, track: &TrackInfo, mut progress_callback: F) -> Result<PathBuf>
    where
        F: FnMut(DownloadProgressUpdate) + Send,
    {
        let dest = self.generate_filename(
            track,
            &manifest.quality,
        );

        let result = if let Some(ref url) = manifest.url {
            self.download_file_with_progress(
                url,
                &dest,
                &mut progress_callback,
            )
            .await?;
        } else if let Some(ref segments) = manifest.segment_urls {
            self.download_segments_with_progress(
                segments,
                &dest,
                &mut progress_callback,
            )
            .await?;
        } else {
            return Err(DownloadError::NoDownloadUrl);
        };

        Ok(dest)
    }
}

fn format_sample_rate(sample_rate: u32) -> String {
    if sample_rate >= 1000 {
        let khz = sample_rate as f64 / 1000.0;
        format!(
            "{:.1}kHz",
            khz
        )
    } else {
        format!(
            "{}Hz",
            sample_rate
        )
    }
}
