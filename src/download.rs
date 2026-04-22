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
    embed_cover: bool,
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
            embed_cover: true
        }
    }

    pub fn with_embed_cover(mut self, embed_cover: bool) -> Self {
        self.embed_cover = embed_cover;
        self
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

    pub fn generate_filename(&self, track: &TrackInfo, extension: &str) -> PathBuf {
        let title = Self::sanitize_filename(&track.title);
        let filename = format!("{}.{}", title, extension);

        let sub_dir = match self.dir_mode {
            DirMode::Playlist => {
                let p = if let Some(ref playlist) = self.playlist {
                    Self::sanitize_filename(playlist)
                } else {
                    String::new()
                };
                if p.is_empty() {
                    "Unknown Playlist".to_string()
                } else {
                    p
                }
            }
            DirMode::Artist => {
                let a = Self::sanitize_filename(&track.artist);
                if a.is_empty() {
                    "Unknown Artist".to_string()
                } else {
                    a
                }
            }
            DirMode::Album => {
                let p = if let Some(ref album) = track.album {
                    Self::sanitize_filename(album)
                } else if let Some(ref album) = self.album {
                    Self::sanitize_filename(album)
                } else {
                    String::new()
                };
                if p.is_empty() {
                    "Unknown Album".to_string()
                } else {
                    p
                }
            }
            DirMode::Flat => "".to_string(),
        };

        if sub_dir.is_empty() {
            self.output_dir.join(filename)
        } else {
            self.output_dir.join(sub_dir).join(filename)
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
        let extension = match manifest.mime_type.as_deref() {
            Some(mime) if mime.contains("mp4") || mime.contains("m4a") || mime.contains("aac") => {
                if manifest.quality == AudioQuality::HiResLossless || manifest.quality == AudioQuality::Lossless {
                    "flac"
                } else {
                    "m4a"
                }
            },
            Some(mime) if mime.contains("flac") => "flac",
            _ => manifest.quality.file_extension(),
        };

        let dest = self.generate_filename(
            track,
            extension,
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

        let is_mp4 = manifest.mime_type.as_deref().is_some_and(|m| m.contains("mp4") || m.contains("m4a") || m.contains("aac"));
        let is_lossless = manifest.quality == AudioQuality::HiResLossless || manifest.quality == AudioQuality::Lossless;
        if is_mp4 && is_lossless {
            let temp_path = dest.with_extension("temp.flac");
            let status = std::process::Command::new("ffmpeg")
                .arg("-y")
                .arg("-loglevel")
                .arg("error")
                .arg("-i")
                .arg(&dest)
                .arg("-c:a")
                .arg("flac")
                .arg(&temp_path)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map_err(|e| DownloadError::DownloadFailed(format!("Failed to run ffmpeg: {}", e)))?;
            
            if status.success() {
                std::fs::rename(&temp_path, &dest).map_err(|e| DownloadError::DownloadFailed(format!("Failed to rename temp file: {}", e)))?;
            } else {
                let _ = std::fs::remove_file(&temp_path);
                return Err(DownloadError::DownloadFailed(format!("ffmpeg failed with status: {}", status)));
            }
        }

        self.embed_metadata_and_cover(&dest, track).await;
        Ok(dest)
    }

    pub async fn download_from_manifest_with_progress<F>(&self, manifest: &DownloadManifest, track: &TrackInfo, mut progress_callback: F) -> Result<PathBuf>
    where
        F: FnMut(DownloadProgressUpdate) + Send,
    {
        let extension = match manifest.mime_type.as_deref() {
            Some(mime) if mime.contains("mp4") || mime.contains("m4a") || mime.contains("aac") => {
                if manifest.quality == AudioQuality::HiResLossless || manifest.quality == AudioQuality::Lossless {
                    "flac"
                } else {
                    "m4a"
                }
            },
            Some(mime) if mime.contains("flac") => "flac",
            _ => manifest.quality.file_extension(),
        };

        let dest = self.generate_filename(
            track,
            extension,
        );

        let result = if let Some(ref url) = manifest.url {
            self.download_file_with_progress(
                url,
                &dest,
                &mut progress_callback,
            )
            .await
        } else if let Some(ref segments) = manifest.segment_urls {
            self.download_segments_with_progress(
                segments,
                &dest,
                &mut progress_callback,
            )
            .await
        } else {
            Err(DownloadError::NoDownloadUrl)
        };

        if result.is_err() {
            let _ = std::fs::remove_file(&dest);
        }

        result?;

        let is_mp4 = manifest.mime_type.as_deref().is_some_and(|m| m.contains("mp4") || m.contains("m4a") || m.contains("aac"));
        let is_lossless = manifest.quality == AudioQuality::HiResLossless || manifest.quality == AudioQuality::Lossless;
        if is_mp4 && is_lossless {
            let temp_path = dest.with_extension("temp.flac");
            let status = std::process::Command::new("ffmpeg")
                .arg("-y")
                .arg("-loglevel")
                .arg("error")
                .arg("-i")
                .arg(&dest)
                .arg("-c:a")
                .arg("flac")
                .arg(&temp_path)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map_err(|e| DownloadError::DownloadFailed(format!("Failed to run ffmpeg: {}", e)))?;
            
            if status.success() {
                std::fs::rename(&temp_path, &dest).map_err(|e| DownloadError::DownloadFailed(format!("Failed to rename temp file: {}", e)))?;
            } else {
                let _ = std::fs::remove_file(&temp_path);
                return Err(DownloadError::DownloadFailed(format!("ffmpeg failed with status: {}", status)));
            }
        }

        self.embed_metadata_and_cover(&dest, track).await;

        Ok(dest)
    }

    async fn embed_metadata_and_cover(&self, dest: &Path, track: &TrackInfo) {
        if let Err(e) = embed_metadata(dest, &track.title, &track.artist, track.album.as_deref()) {
            eprintln!("Warning: Failed to embed metadata: {}", e);
        }

        if self.embed_cover {
            if let Some(ref cover_url) = track.cover_url {
                let cover_ext = cover_url.rsplit('.').next().unwrap_or("jpg").split('?').next().unwrap_or("jpg");
                let cover_path = dest.with_file_name(format!("cover.{}", cover_ext));

                if let Ok(_) = self.download_file(cover_url, &cover_path).await {
                    if let Err(e) = embed_cover_in_audio(dest, &cover_path).await {
                        eprintln!("Warning: Failed to embed cover art: {}", e);
                    }
                    let _ = tokio::fs::remove_file(&cover_path).await;
                }
            }
        }
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

use lofty::probe::Probe;
use lofty::file::{TaggedFileExt, AudioFile};
use lofty::tag::{Accessor, Tag, TagExt};
use lofty::picture::{Picture, PictureType, MimeType};
use lofty::config::WriteOptions;

pub fn embed_metadata(
    path: &Path,
    title: &str,
    artist: &str,
    album: Option<&str>,
) -> Result<()> {
    let mut tagged_file = match Probe::open(path) {
        Ok(p) => match p.read() {
            Ok(tf) => tf,
            Err(e) => return Err(DownloadError::DownloadFailed(format!("Failed to read file: {}", e))),
        },
        Err(e) => return Err(DownloadError::DownloadFailed(format!("Failed to open file: {}", e))),
    };

    let tag_type = tagged_file.primary_tag_type();
    let tag = match tagged_file.primary_tag_mut() {
        Some(t) => t,
        None => {
            tagged_file.insert_tag(Tag::new(tag_type));
            tagged_file.primary_tag_mut().unwrap()
        }
    };

    tag.set_title(title.to_string());
    tag.set_artist(artist.to_string());
    if let Some(a) = album {
        tag.set_album(a.to_string());
    }

    if let Err(e) = tagged_file.save_to_path(path, WriteOptions::default()) {
        return Err(DownloadError::DownloadFailed(format!("Failed to save tags: {}", e)));
    }

    Ok(())
}

pub async fn embed_cover_in_audio(
    audio_path: &Path,
    cover_path: &Path,
) -> Result<()> {
    let cover_data = match tokio::fs::read(cover_path).await {
        Ok(data) => data,
        Err(e) => return Err(DownloadError::DownloadFailed(format!("Failed to read cover: {}", e))),
    };

    let mut tagged_file = match Probe::open(audio_path) {
        Ok(p) => match p.read() {
            Ok(tf) => tf,
            Err(e) => return Err(DownloadError::DownloadFailed(format!("Failed to read audio file: {}", e))),
        },
        Err(e) => return Err(DownloadError::DownloadFailed(format!("Failed to open audio file: {}", e))),
    };

    let tag_type = tagged_file.primary_tag_type();
    let tag = match tagged_file.primary_tag_mut() {
        Some(t) => t,
        None => {
            tagged_file.insert_tag(Tag::new(tag_type));
            tagged_file.primary_tag_mut().unwrap()
        }
    };

    let mime_type = if cover_path.extension().unwrap_or_default().to_string_lossy().eq_ignore_ascii_case("png") {
        MimeType::Png
    } else {
        MimeType::Jpeg
    };

    let picture = Picture::new_unchecked(
        PictureType::CoverFront,
        Some(mime_type),
        None,
        cover_data,
    );

    tag.push_picture(picture);

    if let Err(e) = tagged_file.save_to_path(audio_path, WriteOptions::default()) {
        return Err(DownloadError::DownloadFailed(format!("Failed to save cover art: {}", e)));
    }

    Ok(())
}
