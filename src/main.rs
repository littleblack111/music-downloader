mod adaptive;
mod cli;
mod csv;
mod download;
mod error;
mod progress;
mod services;
mod types;
mod ui;

use adaptive::{acquire_slot, AdaptiveConcurrency};
use cli::{Cli, Commands};
use colored::Colorize;
use csv::{CsvMatchResult, CsvMatcher};
use download::{DirMode, DownloadManager, DownloadProgressUpdate};
use error::{DownloadError, Result};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use progress::DownloadProgress;
use regex::Regex;
use services::{tidal::TidalService, MusicService};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::Mutex;
use types::{AudioQuality, Service};
use ui::Ui;

const NOT_FOUND_DELAY_MS: u64 = 1000;
const RATE_LIMIT_DELAY_MS: u64 = 15000;
const CONSECUTIVE_FAILURE_THRESHOLD: usize = 3;
const RETRY_BACKOFF_SECS: [u64; 5] = [
    5, 15, 30, 60, 120,
];

pub struct ProgressManager {
    multi: MultiProgress,
    download_bars: Vec<ProgressBar>,
    total_bar: ProgressBar,
    status_bar: ProgressBar,
    matched_count: Arc<AtomicU64>,
    downloaded_count: Arc<AtomicU64>,
    total_tracks: u64,
}

impl ProgressManager {
    pub fn new(max_concurrent: usize, total_tracks: u64) -> Self {
        let multi = MultiProgress::new();

        let mut download_bars = Vec::new();
        for i in 0..max_concurrent {
            let pb = multi.add(ProgressBar::new(0));
            pb.set_style(
                ProgressStyle::default_bar()
                    .template(
                        &format!(
                            "Slot {}: [{{bar:25.cyan/blue}}] {{msg}}",
                            i + 1
                        ),
                    )
                    .unwrap()
                    .progress_chars("█▓▒░ "),
            );
            pb.set_message(
                "Idle"
                    .dimmed()
                    .to_string(),
            );
            pb.enable_steady_tick(Duration::from_millis(100));
            download_bars.push(pb);
        }

        let separator = multi.add(ProgressBar::new_spinner());
        separator.set_style(
            ProgressStyle::default_spinner()
                .template("─────────────────────────────────────────────────────────────────────")
                .unwrap(),
        );
        separator.finish();

        let total_bar = multi.add(ProgressBar::new(total_tracks * 2));
        total_bar.set_style(
            ProgressStyle::default_bar()
                .template("Progress: [{bar:40.cyan/blue}] {pos}/{len} ({eta}) {percent}%")
                .unwrap()
                .progress_chars("█▓▒░ "),
        );

        let status_bar = multi.add(ProgressBar::new_spinner());
        status_bar.set_style(
            ProgressStyle::default_spinner()
                .template("Stage: {msg}")
                .unwrap(),
        );
        status_bar.set_message("1/2 - Matching tracks".to_string());
        status_bar.enable_steady_tick(Duration::from_millis(100));

        Self {
            multi,
            download_bars,
            total_bar,
            status_bar,
            matched_count: Arc::new(AtomicU64::new(0)),
            downloaded_count: Arc::new(AtomicU64::new(0)),
            total_tracks,
        }
    }

    pub fn set_stage(&self, stage: &str) {
        self.status_bar
            .set_message(stage.to_string());
        self.status_bar
            .tick();
    }

    pub fn inc_matched(&self) {
        self.matched_count
            .fetch_add(
                1,
                Ordering::Relaxed,
            );
        self.total_bar
            .inc(1);
        self.update_total_message();
    }

    pub fn inc_downloaded(&self) {
        self.downloaded_count
            .fetch_add(
                1,
                Ordering::Relaxed,
            );
        self.total_bar
            .inc(1);
        self.update_total_message();
    }

    fn update_total_message(&self) {
        let matched = self
            .matched_count
            .load(Ordering::Relaxed);
        let downloaded = self
            .downloaded_count
            .load(Ordering::Relaxed);
        self.total_bar
            .set_message(
                format!(
                    "{} matched + {} downloaded",
                    matched, downloaded
                ),
            );
    }

    pub fn set_downloading(&self, slot: usize, artist: &str, title: &str) {
        if slot
            < self
                .download_bars
                .len()
        {
            self.download_bars[slot].set_message(
                format!(
                    "{} - {}",
                    artist, title
                ),
            );
            self.download_bars[slot].set_length(0);
            self.download_bars[slot].set_position(0);
        }
    }

    pub fn update_download_progress(&self, slot: usize, progress: DownloadProgressUpdate) {
        if slot
            < self
                .download_bars
                .len()
        {
            if let Some(total) = progress.total_bytes {
                self.download_bars[slot].set_length(total);
            }
            self.download_bars[slot].set_position(progress.bytes_downloaded);
            if let Some(speed) = progress.speed_bps {
                let speed_str = format_speed(speed);
                self.download_bars[slot].set_message(
                    format!(
                        "{} ({})",
                        self.download_bars[slot]
                            .message()
                            .split_whitespace()
                            .next()
                            .unwrap_or(""),
                        speed_str
                    ),
                );
            }
        }
    }

    pub fn finish_download(&self, slot: usize, success: bool, artist: &str, title: &str) {
        if slot
            < self
                .download_bars
                .len()
        {
            if success {
                self.download_bars[slot].set_message(
                    format!(
                        "✓ {} - {}",
                        artist.green(),
                        title
                    ),
                );
            } else {
                self.download_bars[slot].set_message(
                    format!(
                        "✗ {} - {}",
                        artist.red(),
                        title
                    ),
                );
            }
            self.download_bars[slot].finish();
        }
    }

    pub fn clear_slot(&self, slot: usize) {
        if slot
            < self
                .download_bars
                .len()
        {
            self.download_bars[slot].set_message(
                "Idle"
                    .dimmed()
                    .to_string(),
            );
            self.download_bars[slot].set_length(0);
            self.download_bars[slot].set_position(0);
        }
    }

    pub fn update_download(&self, slot: usize, msg: String) {
        if slot
            < self
                .download_bars
                .len()
        {
            self.download_bars[slot].set_message(msg);
            self.download_bars[slot].tick();
        }
    }

    pub fn inc_total(&self) {
        self.total_bar
            .inc(1);
    }

    pub fn finish(&self) {
        for bar in &self.download_bars {
            bar.finish_and_clear();
        }
        self.total_bar
            .finish();
        self.status_bar
            .finish();
    }

    pub fn finish_with_message(&self, msg: &str) {
        for bar in &self.download_bars {
            bar.finish_and_clear();
        }
        self.total_bar
            .finish_with_message(msg.to_string());
        self.status_bar
            .finish();
    }

    pub fn clear(&self) {
        for bar in &self.download_bars {
            bar.finish_and_clear();
        }
        self.total_bar.finish_and_clear();
        self.status_bar.finish_and_clear();
    }

    pub fn reset_for_download(&self, download_count: u64) {
        for bar in &self.download_bars {
            bar.reset();
            bar.set_message("Idle".dimmed().to_string());
            bar.set_length(0);
            bar.set_position(0);
        }
        self.total_bar.set_length(self.total_tracks + download_count);
    }
}

fn format_speed(bytes_per_sec: f64) -> String {
    if bytes_per_sec >= 1_000_000.0 {
        format!(
            "{:.1} MB/s",
            bytes_per_sec / 1_000_000.0
        )
    } else if bytes_per_sec >= 1_000.0 {
        format!(
            "{:.1} KB/s",
            bytes_per_sec / 1_000.0
        )
    } else {
        format!(
            "{:.0} B/s",
            bytes_per_sec
        )
    }
}

pub struct SimpleProgressManager {
    multi: MultiProgress,
    download_bars: Vec<ProgressBar>,
    total_bar: ProgressBar,
}

impl SimpleProgressManager {
    pub fn new(max_concurrent: usize, total: u64, prefix: &str) -> Self {
        let multi = MultiProgress::new();

        let mut download_bars = Vec::new();
        for _ in 0..max_concurrent {
            let pb = multi.add(ProgressBar::new_spinner());
            pb.set_style(
                ProgressStyle::default_spinner()
                    .template("{msg}")
                    .unwrap(),
            );
            pb.set_message(
                "Idle"
                    .dimmed()
                    .to_string(),
            );
            pb.enable_steady_tick(Duration::from_millis(100));
            download_bars.push(pb);
        }

        let separator = multi.add(ProgressBar::new_spinner());
        separator.set_style(
            ProgressStyle::default_spinner()
                .template("─────────────────────────────────────────────────────────────────────")
                .unwrap(),
        );
        separator.finish();

        let total_bar = multi.add(ProgressBar::new(total));
        total_bar.set_style(
            ProgressStyle::default_bar()
                .template("{prefix}: [{bar:40.cyan/blue}] {pos}/{len} ({eta}) {percent}%")
                .unwrap()
                .progress_chars("█▓▒░ "),
        );
        total_bar.set_prefix(prefix.to_string());

        Self {
            multi,
            download_bars,
            total_bar,
        }
    }

    pub fn update_download(&self, slot: usize, msg: String) {
        if slot
            < self
                .download_bars
                .len()
        {
            self.download_bars[slot].set_message(msg);
            self.download_bars[slot].tick();
        }
    }

    pub fn clear_slot(&self, slot: usize) {
        if slot
            < self
                .download_bars
                .len()
        {
            self.download_bars[slot].set_message(
                "Idle"
                    .dimmed()
                    .to_string(),
            );
        }
    }

    pub fn inc_total(&self) {
        self.total_bar
            .inc(1);
    }

    pub fn finish_with_message(&self, msg: &str) {
        for bar in &self.download_bars {
            bar.finish_and_clear();
        }
        self.total_bar
            .finish_with_message(msg.to_string());
    }
}

#[tokio::main]
async fn main() {
    colored::control::set_override(true);
    let args = Cli::parse();

    if !args.quiet {
        Ui::print_banner();
    }

    let output_dir = args
        .output
        .clone()
        .unwrap_or_else(|| PathBuf::from("./downloads"));
    let quality = parse_quality(
        args.quality
            .as_deref(),
    );
    let service = parse_service(
        args.service
            .as_deref(),
    );
    let max_concurrent = args
        .concurrent
        .unwrap_or_else(num_cpus::get);
    let dir_mode = parse_dir_mode(
        args.dir_mode
            .as_deref(),
    );

    if let Err(e) = run_command(
        args,
        &output_dir,
        quality,
        service,
        max_concurrent,
        dir_mode,
    )
    .await
    {
        e.pretty_print();
        std::process::exit(1);
    }
}

fn parse_quality(quality: Option<&str>) -> AudioQuality {
    match quality {
        Some("hires") => AudioQuality::HiResLossless,
        Some("lossless") => AudioQuality::Lossless,
        Some("high") => AudioQuality::High,
        Some("low") => AudioQuality::Low,
        Some("mp3") => AudioQuality::Mp3_320,
        None => AudioQuality::HiResLossless,
        _ => AudioQuality::HiResLossless,
    }
}

fn parse_service(service: Option<&str>) -> Service {
    match service {
        Some("tidal") => Service::Tidal,
        Some("amazon") => Service::AmazonMusic,
        Some("soundcloud") => Service::SoundCloud,
        Some("khinsider") => Service::KHInsider,
        None => Service::Tidal,
        _ => Service::Tidal,
    }
}

fn parse_dir_mode(dir_mode: Option<&str>) -> DirMode {
    match dir_mode {
        Some("playlist") => DirMode::Playlist,
        Some("album") => DirMode::Album,
        Some("artist") => DirMode::Artist,
        Some("flat") => DirMode::Flat,
        None => DirMode::Playlist,
        _ => DirMode::Playlist,
    }
}

async fn ensure_output_dir(output_dir: &PathBuf) -> Result<()> {
    if !output_dir.exists() {
        tokio::fs::create_dir_all(output_dir).await?;
    }
    Ok(())
}

async fn run_command(args: Cli, output_dir: &PathBuf, quality: AudioQuality, service: Service, max_concurrent: usize, dir_mode: DirMode) -> Result<()> {
    match args.command {
        Commands::Search {
            query,
            artist,
            limit,
        } => {
            Ui::print_info(
                &format!(
                    "Searching for: {}",
                    query.cyan()
                ),
            );

            let tidal = TidalService::new();
            let results = tidal
                .search(
                    &query,
                    artist.as_deref(),
                )
                .await?;

            if results
                .tracks
                .is_empty()
            {
                Ui::print_warning("No results found");
                return Ok(());
            }

            Ui::print_info(
                &format!(
                    "Found {} results",
                    results
                        .tracks
                        .len()
                        .to_string()
                        .green()
                ),
            );
            Ui::print_search_results(
                &results.tracks,
                limit,
            );
        }

        Commands::Track {
            input,
            artist,
            first,
        } => {
            ensure_output_dir(output_dir).await?;
            download_track(
                &input,
                artist.as_deref(),
                first,
                output_dir,
                quality,
                service,
            )
            .await?;
        }

        Commands::Album {
            input,
        } => {
            ensure_output_dir(output_dir).await?;
            download_album(
                &input,
                output_dir,
                quality,
                service,
                max_concurrent,
            )
            .await?;
        }

        Commands::Artist {
            input,
        } => {
            download_artist(
                &input, output_dir, quality, service,
            )
            .await?;
        }

        Commands::Playlist {
            input,
        } => {
            ensure_output_dir(output_dir).await?;
            download_playlist(
                &input,
                output_dir,
                quality,
                service,
                max_concurrent,
            )
            .await?;
        }

        Commands::Csv {
            file,
            threshold,
            yes,
        } => {
            ensure_output_dir(output_dir).await?;
            download_from_csv(
                &file,
                threshold,
                yes,
                output_dir,
                quality,
                max_concurrent,
                dir_mode,
            )
            .await?;
        }

        Commands::Info {
            id,
        } => {
            let tidal = TidalService::new();
            let track = tidal
                .get_track_info(id)
                .await?;
            Ui::print_track_detail(
                &track, None,
            );
        }

        Commands::Recommend {
            id,
        } => {
            Ui::print_info("Fetching recommendations...");
            let tidal = TidalService::new();
            let results = tidal
                .search(
                    "recommendations",
                    None,
                )
                .await?;
            Ui::print_search_results(
                &results.tracks,
                10,
            );
            let _ = id;
        }

        Commands::List => {
            Ui::print_services();
            Ui::print_qualities();
        }
    }

    Ok(())
}

async fn download_track(input: &str, artist: Option<&str>, first: bool, output_dir: &PathBuf, quality: AudioQuality, _service: Service) -> Result<()> {
    let tidal = TidalService::new();

    let track_id = if input
        .parse::<u64>()
        .is_ok()
    {
        input
            .parse::<u64>()
            .unwrap()
    } else {
        Ui::print_info(
            &format!(
                "Searching for: {}",
                input.cyan()
            ),
        );

        let search_results = tidal
            .search(
                input, artist,
            )
            .await?;

        if search_results
            .tracks
            .is_empty()
        {
            return Err(DownloadError::TrackNotFound(input.to_string()));
        }

        let selected_index = if first {
            0
        } else {
            Ui::select_track(&search_results.tracks).ok_or_else(|| DownloadError::TrackNotFound("No track selected".to_string()))?
        };

        let track = &search_results.tracks[selected_index];
        Ui::print_info(
            &format!(
                "Selected: {} by {}",
                track
                    .title
                    .cyan(),
                track
                    .artist
                    .white()
            ),
        );

        track.id
    };

    let track_info = tidal
        .get_track_info(track_id)
        .await?;
    let manifest = tidal
        .get_manifest(
            track_id, quality,
        )
        .await?;

    let downloader = DownloadManager::new(output_dir);
    downloader
        .download_from_manifest(
            &manifest,
            &track_info,
        )
        .await?;

    Ui::print_success("Download complete!");

    Ok(())
}

async fn download_album(input: &str, output_dir: &PathBuf, quality: AudioQuality, _service: Service, max_concurrent: usize) -> Result<()> {
    let tidal = TidalService::new();

    let album_id = if input
        .parse::<u64>()
        .is_ok()
    {
        input
            .parse::<u64>()
            .unwrap()
    } else {
        Ui::print_info(
            &format!(
                "Searching for album: {}",
                input.cyan()
            ),
        );

        let search_results = tidal
            .search(
                input, None,
            )
            .await?;

        if search_results
            .tracks
            .is_empty()
        {
            return Err(DownloadError::AlbumNotFound(input.to_string()));
        }

        let track = &search_results.tracks[0];
        let album_name = track
            .album
            .as_ref()
            .ok_or_else(|| DownloadError::AlbumNotFound("No album info found".to_string()))?;

        Ui::print_info(
            &format!(
                "Found album: {} by {}",
                album_name.cyan(),
                track
                    .artist
                    .white()
            ),
        );

        if !Ui::confirm("Download this album?") {
            Ui::print_info("Cancelled");
            return Ok(());
        }

        return Ok(());
    };

    let album_info = tidal
        .get_album_info(album_id)
        .await?;
    Ui::print_album_detail(&album_info);

    let tracks = tidal
        .get_album_tracks(album_id)
        .await?;

    let total_tracks = tracks.len();
    let progress_mgr = Arc::new(
        SimpleProgressManager::new(
            max_concurrent,
            total_tracks as u64,
            "Album",
        ),
    );

    let album_name = Some(
        album_info
            .title
            .clone(),
    );
    let output_dir = output_dir.clone();
    let adaptive = Arc::new(Mutex::new(AdaptiveConcurrency::new(max_concurrent)));
    let semaphore = adaptive
        .lock()
        .await
        .semaphore();
    let success_count = Arc::new(Mutex::new(0usize));
    let fail_count = Arc::new(Mutex::new(0usize));
    let slot_counter = Arc::new(AtomicUsize::new(0));

    let mut handles = vec![];

    for track in tracks {
        let permit = acquire_slot(&semaphore).await;
        let tidal = TidalService::new();
        let output_dir = output_dir.clone();
        let quality = quality;
        let album_name = album_name.clone();
        let success_count = success_count.clone();
        let fail_count = fail_count.clone();
        let adaptive_inner = adaptive.clone();
        let progress_inner = progress_mgr.clone();
        let slot_counter_inner = slot_counter.clone();

        let handle = tokio::spawn(
            async move {
                let _permit = permit;
                let slot = slot_counter_inner.fetch_add(
                    1,
                    Ordering::Relaxed,
                ) % max_concurrent;
                let downloader = DownloadManager::new(&output_dir).with_album(album_name.as_deref());

                progress_inner.update_download(
                    slot,
                    format!(
                        "Matching: {} - {}",
                        track
                            .artist
                            .white(),
                        track
                            .title
                            .cyan()
                    ),
                );

                let result = retry_download(
                    || async {
                        let manifest = tidal
                            .get_manifest(
                                track.id, quality,
                            )
                            .await?;
                        downloader
                            .download_from_manifest(
                                &manifest, &track,
                            )
                            .await
                    },
                    adaptive_inner.clone(),
                )
                .await;

                match result {
                    Ok(_) => {
                        adaptive_inner
                            .lock()
                            .await
                            .on_success();
                        *success_count
                            .lock()
                            .await += 1;
                        progress_inner.update_download(
                            slot,
                            format!(
                                "✓ {} - {}",
                                track
                                    .artist
                                    .white(),
                                track
                                    .title
                                    .green()
                            ),
                        );
                    }
                    Err(e) => {
                        adaptive_inner
                            .lock()
                            .await
                            .on_failure();
                        *fail_count
                            .lock()
                            .await += 1;
                        progress_inner.update_download(
                            slot,
                            format!(
                                "✗ {}: {}",
                                track
                                    .title
                                    .white(),
                                e.to_string()
                                    .dimmed()
                            ),
                        );
                    }
                }
                progress_inner.inc_total();
            },
        );

        handles.push(handle);
    }

    for handle in handles {
        handle
            .await
            .unwrap();
    }

    let success = *success_count
        .lock()
        .await;
    let fail = *fail_count
        .lock()
        .await;

    let final_concurrent = adaptive
        .lock()
        .await
        .get_concurrent();

    progress_mgr.finish_with_message(
        &format!(
            "{} successful, {} failed",
            success
                .to_string()
                .green(),
            fail.to_string()
                .red(),
        ),
    );

    println!();
    println!(
        "Album download complete. Final concurrency: {}",
        final_concurrent
            .to_string()
            .yellow()
    );

    Ok(())
}

async fn download_artist(input: &str, _output_dir: &PathBuf, _quality: AudioQuality, _service: Service) -> Result<()> {
    let tidal = TidalService::new();

    Ui::print_info(
        &format!(
            "Searching for artist: {}",
            input.cyan()
        ),
    );

    let search_results = tidal
        .search(
            input, None,
        )
        .await?;

    if search_results
        .tracks
        .is_empty()
    {
        return Err(DownloadError::ArtistNotFound(input.to_string()));
    }

    Ui::print_search_results(
        &search_results.tracks,
        5,
    );
    Ui::print_warning("Artist downloads are not fully implemented yet.");
    Ui::print_info("Use 'track' command to download individual tracks.");

    Ok(())
}

async fn download_playlist(input: &str, output_dir: &PathBuf, quality: AudioQuality, _service: Service, max_concurrent: usize) -> Result<()> {
    let tidal = TidalService::new();

    let playlist_id = if input
        .parse::<u64>()
        .is_ok()
    {
        input
            .parse::<u64>()
            .unwrap()
    } else {
        return Err(DownloadError::PlaylistNotFound("Please provide a playlist ID".to_string()));
    };

    let playlist_info = tidal
        .get_playlist_info(playlist_id)
        .await?;
    Ui::print_playlist_detail(&playlist_info);

    if !Ui::confirm("Download this playlist?") {
        Ui::print_info("Cancelled");
        return Ok(());
    }

    let tracks = tidal
        .get_playlist_tracks(playlist_id)
        .await?;

    let total_tracks = tracks.len();
    let progress_mgr = Arc::new(
        SimpleProgressManager::new(
            max_concurrent,
            total_tracks as u64,
            "Playlist",
        ),
    );

    let output_dir = output_dir.clone();
    let adaptive = Arc::new(Mutex::new(AdaptiveConcurrency::new(max_concurrent)));
    let semaphore = adaptive
        .lock()
        .await
        .semaphore();
    let success_count = Arc::new(Mutex::new(0usize));
    let fail_count = Arc::new(Mutex::new(0usize));
    let slot_counter = Arc::new(AtomicUsize::new(0));

    let mut handles = vec![];

    for track in tracks {
        let permit = acquire_slot(&semaphore).await;
        let tidal = TidalService::new();
        let output_dir = output_dir.clone();
        let quality = quality;
        let success_count = success_count.clone();
        let fail_count = fail_count.clone();
        let adaptive_inner = adaptive.clone();
        let progress_inner = progress_mgr.clone();
        let slot_counter_inner = slot_counter.clone();

        let handle = tokio::spawn(
            async move {
                let _permit = permit;
                let slot = slot_counter_inner.fetch_add(
                    1,
                    Ordering::Relaxed,
                ) % max_concurrent;
                let downloader = DownloadManager::new(&output_dir);

                progress_inner.update_download(
                    slot,
                    format!(
                        "Matching: {} - {}",
                        track
                            .artist
                            .white(),
                        track
                            .title
                            .cyan()
                    ),
                );

                let result = retry_download(
                    || async {
                        let manifest = tidal
                            .get_manifest(
                                track.id, quality,
                            )
                            .await?;
                        downloader
                            .download_from_manifest(
                                &manifest, &track,
                            )
                            .await
                    },
                    adaptive_inner.clone(),
                )
                .await;

                match result {
                    Ok(_) => {
                        adaptive_inner
                            .lock()
                            .await
                            .on_success();
                        *success_count
                            .lock()
                            .await += 1;
                        progress_inner.update_download(
                            slot,
                            format!(
                                "✓ {} - {}",
                                track
                                    .artist
                                    .white(),
                                track
                                    .title
                                    .green()
                            ),
                        );
                    }
                    Err(e) => {
                        adaptive_inner
                            .lock()
                            .await
                            .on_failure();
                        *fail_count
                            .lock()
                            .await += 1;
                        progress_inner.update_download(
                            slot,
                            format!(
                                "✗ {}: {}",
                                track
                                    .title
                                    .white(),
                                e.to_string()
                                    .dimmed()
                            ),
                        );
                    }
                }
                progress_inner.inc_total();
            },
        );

        handles.push(handle);
    }

    for handle in handles {
        handle
            .await
            .unwrap();
    }

    let success = *success_count
        .lock()
        .await;
    let fail = *fail_count
        .lock()
        .await;

    let final_concurrent = adaptive
        .lock()
        .await
        .get_concurrent();

    progress_mgr.finish_with_message(
        &format!(
            "{} successful, {} failed",
            success
                .to_string()
                .green(),
            fail.to_string()
                .red(),
        ),
    );

    println!();
    println!(
        "Playlist download complete. Final concurrency: {}",
        final_concurrent
            .to_string()
            .yellow()
    );

    Ok(())
}

#[derive(Debug, Clone)]
struct DownloadTask {
    track: crate::types::TrackInfo,
    csv_track: csv::CsvTrack,
    confidence: f64,
    album: Option<String>,
}

#[derive(Debug, Clone)]
struct DownloadJob {
    track: crate::types::TrackInfo,
    album: Option<String>,
    playlist: Option<String>,
    dir_mode: DirMode,
    retry_count: usize,
    max_retries: usize,
    last_error: Option<String>,
}

async fn download_from_csv(file: &PathBuf, threshold: f64, skip_confirm: bool, output_dir: &PathBuf, quality: AudioQuality, max_concurrent: usize, dir_mode: DirMode) -> Result<()> {
    Ui::print_info(
        &format!(
            "Parsing CSV file: {}",
            file.display()
                .to_string()
                .cyan()
        ),
    );

    let csv_tracks = csv::parse_csv_file(file)?;
    let total = csv_tracks.len();
    let progress_file = output_dir.join(".download_progress.json");
    let source_str = file
        .display()
        .to_string();

    let (progress, resume) = if progress_file.exists() {
        if let Ok(existing) = DownloadProgress::load_from_file(&progress_file) {
            if existing.source == source_str {
                let completed = existing.get_completed_count();
                let failed_matches = existing
                    .failed_matches
                    .len();
                println!();
                println!(
                    "{} Previous progress found: {}/{} tracks completed, {} failed matches",
                    "→".cyan(),
                    completed
                        .to_string()
                        .green(),
                    total
                        .to_string()
                        .white(),
                    failed_matches
                        .to_string()
                        .red()
                );
                let resume_prompt = "Resume from where you left off?";
                if Ui::confirm(resume_prompt) {
                    (
                        existing, true,
                    )
                } else {
                    let _ = std::fs::remove_file(&progress_file);
                    (
                        DownloadProgress::new(
                            &source_str,
                            total,
                        ),
                        false,
                    )
                }
            } else {
                (
                    DownloadProgress::new(
                        &source_str,
                        total,
                    ),
                    false,
                )
            }
        } else {
            (
                DownloadProgress::new(
                    &source_str,
                    total,
                ),
                false,
            )
        }
    } else {
        (
            DownloadProgress::new(
                &source_str,
                total,
            ),
            false,
        )
    };

    Ui::print_info(
        &format!(
            "Found {} tracks in CSV",
            total
                .to_string()
                .green()
        ),
    );

    let matched_indices: std::collections::HashSet<usize> = if resume {
        progress
            .matched_csv_indices
            .iter()
            .copied()
            .collect()
    } else {
        std::collections::HashSet::new()
    };

    let completed_track_ids: std::collections::HashSet<u64> = if resume {
        progress
            .completed_track_ids
            .iter()
            .copied()
            .collect()
    } else {
        std::collections::HashSet::new()
    };

    Ui::print_info(
        &format!(
            "Matching with adaptive concurrency (max {})...",
            max_concurrent
                .to_string()
                .cyan()
        ),
    );

    let progress_mgr = Arc::new(
        ProgressManager::new(
            max_concurrent,
            total as u64,
        ),
    );
    let matcher = Arc::new(CsvMatcher::new());
    let adaptive = Arc::new(Mutex::new(AdaptiveConcurrency::new(max_concurrent)));
    let semaphore = adaptive
        .lock()
        .await
        .semaphore();
    let match_results = Arc::new(Mutex::new(Vec::new()));
    let progress_counter = Arc::new(Mutex::new(0usize));
    let consecutive_failures = Arc::new(Mutex::new(0usize));
    let failed_match_indices = Arc::new(Mutex::new(Vec::<usize>::new()));
    let matched_indices_shared = Arc::new(Mutex::new(matched_indices.clone()));
    let progress_shared = Arc::new(Mutex::new(progress.clone()));
    let csv_tracks_shared = Arc::new(csv_tracks.clone());
    let slot_counter = Arc::new(AtomicUsize::new(0));

    let mut handles = vec![];

    for (index, track) in csv_tracks
        .clone()
        .into_iter()
        .enumerate()
    {
        if resume && matched_indices.contains(&index) {
            let mut prog = progress_counter
                .lock()
                .await;
            *prog += 1;
            progress_mgr.inc_matched();
            continue;
        }

        let permit = acquire_slot(&semaphore).await;
        let matcher = matcher.clone();
        let match_results = match_results.clone();
        let progress_counter = progress_counter.clone();
        let consecutive_failures = consecutive_failures.clone();
        let failed_match_indices = failed_match_indices.clone();
        let matched_indices_inner = matched_indices_shared.clone();
        let progress_inner = progress_shared.clone();
        let csv_tracks_inner = csv_tracks_shared.clone();
        let track_name = track
            .track_name
            .clone();
        let progress_mgr_inner = progress_mgr.clone();
        let slot_counter_inner = slot_counter.clone();

        let handle = tokio::spawn(
            async move {
                let _permit = permit;
                let slot = slot_counter_inner.fetch_add(
                    1,
                    Ordering::Relaxed,
                ) % max_concurrent;
                progress_mgr_inner.update_download(
                    slot,
                    format!(
                        "Matching: {}",
                        track_name.white()
                    ),
                );

                let result = matcher
                    .find_best_match(&track)
                    .await
                    .unwrap_or_else(
                        |e| CsvMatchResult {
                            csv_track: track.clone(),
                            best_match: None,
                            confidence: 0.0,
                            search_attempts: vec![
                                format!(
                                    "Error: {}",
                                    e
                                ),
                            ],
                        },
                    );

                let is_match = result
                    .best_match
                    .is_some();

                if !is_match {
                    let mut failures = consecutive_failures
                        .lock()
                        .await;
                    *failures += 1;

                    if *failures >= CONSECUTIVE_FAILURE_THRESHOLD {
                        drop(failures);
                        progress_mgr_inner.update_download(
                            slot,
                            format!(
                                "Rate limited, waiting {}s...",
                                RATE_LIMIT_DELAY_MS / 1000
                            ),
                        );
                        tokio::time::sleep(Duration::from_millis(RATE_LIMIT_DELAY_MS)).await;
                    } else {
                        drop(failures);
                        tokio::time::sleep(Duration::from_millis(NOT_FOUND_DELAY_MS)).await;
                    }

                    failed_match_indices
                        .lock()
                        .await
                        .push(index);
                } else {
                    *consecutive_failures
                        .lock()
                        .await = 0;
                    matched_indices_inner
                        .lock()
                        .await
                        .insert(index);

                    if let Some(ref matched_track) = result.best_match {
                        let csv_track = &csv_tracks_inner[index];
                        progress_inner
                            .lock()
                            .await
                            .add_matched_track(
                                index,
                                matched_track,
                                result.confidence,
                                &csv_track.track_name,
                                &csv_track.artist_name,
                                csv_track
                                    .album
                                    .as_deref(),
                                csv_track
                                    .playlist_name
                                    .as_deref(),
                            );
                    }
                }

                let status = if is_match {
                    if result.confidence >= threshold {
                        format!(
                            "✓ [{:.0}%] {}",
                            result.confidence * 100.0,
                            track_name.green()
                        )
                    } else {
                        format!(
                            "? [{:.0}%] {}",
                            result.confidence * 100.0,
                            track_name.yellow()
                        )
                    }
                } else {
                    format!(
                        "✗ Not found: {}",
                        track_name.red()
                    )
                };

                let mut prog = progress_counter
                    .lock()
                    .await;
                *prog += 1;
                progress_mgr_inner.update_download(
                    slot, status,
                );
                progress_mgr_inner.inc_matched();

                match_results
                    .lock()
                    .await
                    .push(
                        (
                            index, result,
                        ),
                    );
            },
        );

        handles.push(handle);
    }

    for handle in handles {
        handle
            .await
            .unwrap();
    }

    let match_results_inner = Arc::try_unwrap(match_results)
        .unwrap()
        .into_inner();
    let match_results_sorted: Vec<(
        usize,
        CsvMatchResult,
    )> = match_results_inner;
    let mut match_results: Vec<Option<CsvMatchResult>> = vec![None; total];

    for (index, result) in match_results_sorted {
        match_results[index] = Some(result);
    }

    if resume {
        for m in &progress.matched_tracks {
            let csv_index = m.csv_index;
            if match_results[csv_index].is_none() {
                match_results[csv_index] = Some(
                    CsvMatchResult {
                        csv_track: csv::CsvTrack {
                            track_name: m
                                .csv_track_name
                                .clone(),
                            artist_name: m
                                .csv_artist_name
                                .clone(),
                            album: m
                                .csv_album
                                .clone(),
                            playlist_name: m
                                .playlist_name
                                .clone(),
                            track_type: None,
                            isrc: None,
                            spotify_id: None,
                        },
                        best_match: Some(
                            crate::types::TrackInfo {
                                id: m.track_id,
                                title: m
                                    .track_title
                                    .clone(),
                                artist: m
                                    .track_artist
                                    .clone(),
                                album: m
                                    .album
                                    .clone(),
                                duration: None,
                                quality: None,
                                cover_url: None,
                            },
                        ),
                        confidence: m.confidence,
                        search_attempts: vec![],
                    },
                );
            }
        }
    }

    let mut match_results: Vec<CsvMatchResult> = match_results
        .into_iter()
        .enumerate()
        .map(
            |(index, opt)| {
                opt.unwrap_or_else(
                    || CsvMatchResult {
                        csv_track: csv_tracks[index].clone(),
                        best_match: None,
                        confidence: 0.0,
                        search_attempts: vec![],
                    },
                )
            },
        )
        .collect();

    let failed_indices = Arc::try_unwrap(failed_match_indices)
        .unwrap()
        .into_inner();

    for idx in &failed_indices {
        let track = &csv_tracks[*idx];
        progress_shared
            .lock()
            .await
            .add_failed_match(
                &track.track_name,
                Some(&track.artist_name),
                track
                    .album
                    .as_deref(),
            );
    }

    {
        let mut prog = progress_shared
            .lock()
            .await;
        prog.total_matched = prog
            .matched_csv_indices
            .len();
        let _ = prog.save_to_file(&progress_file);
    }

    if !failed_indices.is_empty() {
        let retry_spinner = ProgressBar::new_spinner();
        retry_spinner.set_style(ProgressStyle::default_spinner());
        retry_spinner.set_message(
            format!(
                "Retrying {} failed matches...",
                failed_indices.len()
            ),
        );

        let retry_match_results = Arc::new(Mutex::new(Vec::new()));
        let retry_progress = Arc::new(Mutex::new(0usize));

        for idx in &failed_indices {
            let csv_track = csv_tracks[*idx].clone();
            let tidal = TidalService::new();
            let retry_match_results = retry_match_results.clone();
            let retry_progress = retry_progress.clone();

            let cleaned_name = clean_track_name(&csv_track.track_name);
            retry_spinner.set_message(
                format!(
                    "Retrying: '{}' by '{}'",
                    cleaned_name, csv_track.artist_name
                ),
            );
            retry_spinner.tick();

            let mut candidates: Vec<crate::types::TrackInfo> = Vec::new();

            let query1 = format!(
                "{} {}",
                cleaned_name, csv_track.artist_name
            );
            if let Ok(results) = tidal
                .search(
                    &query1, None,
                )
                .await
            {
                candidates.extend(results.tracks);
                tokio::time::sleep(Duration::from_millis(300)).await;
            }

            if let Ok(results) = tidal
                .search(
                    &cleaned_name,
                    Some(&csv_track.artist_name),
                )
                .await
            {
                candidates.extend(results.tracks);
                tokio::time::sleep(Duration::from_millis(300)).await;
            }

            if let Ok(results) = tidal
                .search(
                    &cleaned_name,
                    None,
                )
                .await
            {
                candidates.extend(results.tracks);
            }

            candidates.sort_by_key(|t| t.id);
            candidates.dedup_by_key(|t| t.id);

            let best = csv::CsvMatcher::score_and_select_best(
                &candidates,
                &csv_track,
            );

            let result = CsvMatchResult {
                csv_track: csv_track.clone(),
                best_match: best
                    .as_ref()
                    .map(|(t, _)| t.clone()),
                confidence: best
                    .map(|(_, s)| s)
                    .unwrap_or(0.0),
                search_attempts: vec![
                    query1,
                    cleaned_name.clone(),
                ],
            };

            let mut prog = retry_progress
                .lock()
                .await;
            *prog += 1;

            if result
                .best_match
                .is_some()
            {
                let mut prog = progress_shared
                    .lock()
                    .await;
                prog.total_matched += 1;
                prog.mark_csv_matched(*idx);
                if let Some(ref track) = result.best_match {
                    let csv_track = &csv_tracks[*idx];
                    prog.add_matched_track(
                        *idx,
                        track,
                        result.confidence,
                        &csv_track.track_name,
                        &csv_track.artist_name,
                        csv_track
                            .album
                            .as_deref(),
                        csv_track
                            .playlist_name
                            .as_deref(),
                    );
                }
            }

            retry_match_results
                .lock()
                .await
                .push(
                    (
                        *idx, result,
                    ),
                );
        }

        retry_spinner.finish_with_message("Retry complete");

        let retry_results = Arc::try_unwrap(retry_match_results)
            .unwrap()
            .into_inner();

        for (idx, result) in retry_results {
            match_results[idx] = result;
        }
    }

    progress_mgr.clear();

    csv::print_csv_summary(&match_results);

    let valid_matches: Vec<(
        usize,
        &CsvMatchResult,
    )> = match_results
        .iter()
        .enumerate()
        .filter(
            |(_, r)| {
                r.best_match
                    .is_some()
                    && r.confidence >= threshold
            },
        )
        .collect();

    if valid_matches.is_empty() {
        Ui::print_warning("No tracks matched with sufficient confidence.");
        progress_shared
            .lock()
            .await
            .save_to_file(&progress_file)?;
        return Ok(());
    }

    let skipped = match_results.len() - valid_matches.len();
    if skipped > 0 {
        Ui::print_warning(
            &format!(
                "{} tracks skipped (below {:.0}% confidence threshold)",
                skipped,
                threshold * 100.0
            ),
        );
    }

    let tracks_to_download: Vec<_> = valid_matches
        .iter()
        .filter(
            |(_, r)| {
                if let Some(ref track) = r.best_match {
                    !completed_track_ids.contains(&track.id)
                } else {
                    false
                }
            },
        )
        .collect();

    let already_completed = valid_matches.len() - tracks_to_download.len();
    if already_completed > 0 && resume {
        println!(
            "{} {} tracks already downloaded, skipping...",
            "→".cyan(),
            already_completed
                .to_string()
                .green()
        );
    }

    if tracks_to_download.is_empty() {
        Ui::print_info("All tracks already downloaded.");
        progress_shared
            .lock()
            .await
            .save_to_file(&progress_file)?;
        return Ok(());
    }

    println!();
    if !skip_confirm {
        if !Ui::confirm(
            &format!(
                "Download {} tracks with adaptive concurrency?",
                tracks_to_download.len()
            ),
        ) {
            Ui::print_info("Cancelled");
            progress_shared
                .lock()
                .await
                .save_to_file(&progress_file)?;
            return Ok(());
        }
    }

    Ui::print_info(
        &format!(
            "Downloading {} tracks with parallel retry queue (max {})...",
            tracks_to_download
                .len()
                .to_string()
                .green(),
            max_concurrent
                .to_string()
                .cyan()
        ),
    );

    progress_mgr.reset_for_download(tracks_to_download.len() as u64);
    progress_mgr.set_stage("2/2 - Downloading");

    let output_dir = output_dir.clone();
    let download_adaptive = Arc::new(Mutex::new(AdaptiveConcurrency::new(max_concurrent)));
    let success_count = Arc::new(Mutex::new(0usize));
    let final_fail_count = Arc::new(Mutex::new(0usize));

    let queue: Arc<Mutex<std::collections::VecDeque<DownloadJob>>> = Arc::new(Mutex::new(std::collections::VecDeque::new()));
    let in_flight = Arc::new(Mutex::new(0usize));
    let completed = Arc::new(Mutex::new(0usize));
    let failed_jobs = Arc::new(
        Mutex::new(
            Vec::<(
                DownloadJob,
                String,
            )>::new(),
        ),
    );
    let progress_file_shared = progress_file.clone();
    let download_slot_counter = Arc::new(AtomicUsize::new(0));

    for (_, result) in &tracks_to_download {
        if let Some(ref track) = result.best_match {
            let job = DownloadJob {
                track: track.clone(),
                album: result
                    .csv_track
                    .album
                    .clone(),
                playlist: result
                    .csv_track
                    .playlist_name
                    .clone(),
                dir_mode,
                retry_count: 0,
                max_retries: 5,
                last_error: None,
            };
            queue
                .lock()
                .await
                .push_back(job);
        }
    }

    let mut worker_handles = vec![];

    for _worker_id in 0..max_concurrent {
        let queue = queue.clone();
        let in_flight = in_flight.clone();
        let completed = completed.clone();
        let success_count = success_count.clone();
        let final_fail_count = final_fail_count.clone();
        let failed_jobs = failed_jobs.clone();
        let download_adaptive = download_adaptive.clone();
        let output_dir = output_dir.clone();
        let quality = quality;
        let progress_shared = progress_shared.clone();
        let progress_file_inner = progress_file_shared.clone();
        let progress_mgr_inner = progress_mgr.clone();
        let download_slot_counter_inner = download_slot_counter.clone();

        let handle = tokio::spawn(
            async move {
                loop {
                    let job_opt = {
                        let mut q = queue
                            .lock()
                            .await;
                        q.pop_front()
                    };

                    let job = match job_opt {
                        Some(j) => j,
                        None => {
                            let in_flight_val = *in_flight
                                .lock()
                                .await;
                            if in_flight_val == 0 {
                                break;
                            }
                            tokio::time::sleep(Duration::from_millis(50)).await;
                            continue;
                        }
                    };

                    *in_flight
                        .lock()
                        .await += 1;

                    let slot = download_slot_counter_inner.fetch_add(
                        1,
                        Ordering::Relaxed,
                    ) % max_concurrent;
                    let tidal = TidalService::new();
                    let downloader = DownloadManager::new(&output_dir)
                        .with_album(
                            job.album
                                .as_deref(),
                        )
                        .with_playlist(
                            job.playlist
                                .as_deref(),
                        )
                        .with_dir_mode(job.dir_mode);

                    progress_mgr_inner.set_downloading(
                        slot,
                        &job.track
                            .artist,
                        &job.track
                            .title,
                    );

                    let progress_mgr_clone = progress_mgr_inner.clone();
                    let slot_for_callback = slot;
                    let result = async {
                        let manifest = tidal
                            .get_manifest(
                                job.track
                                    .id,
                                quality,
                            )
                            .await?;
                        downloader
                            .download_from_manifest_with_progress(
                                &manifest,
                                &job.track,
                                |progress| {
                                    progress_mgr_clone.update_download_progress(
                                        slot_for_callback,
                                        progress,
                                    );
                                },
                            )
                            .await
                    }
                    .await;

                    match result {
                        Ok(_) => {
                            download_adaptive
                                .lock()
                                .await
                                .on_success();
                            *success_count
                                .lock()
                                .await += 1;
                            *completed
                                .lock()
                                .await += 1;
                            *in_flight
                                .lock()
                                .await -= 1;

                            {
                                let mut prog = progress_shared
                                    .lock()
                                    .await;
                                prog.mark_completed(
                                    job.track
                                        .id,
                                );
                                let _ = prog.save_to_file(&progress_file_inner);
                            }

                            progress_mgr_inner.finish_download(
                                slot,
                                true,
                                &job.track
                                    .artist,
                                &job.track
                                    .title,
                            );
                            progress_mgr_inner.inc_downloaded();
                        }
                        Err(e) => {
                            let error_str = e.to_string();
                            let is_retryable = matches!(
                                e,
                                DownloadError::RateLimited | DownloadError::NetworkError(_) | DownloadError::DownloadFailed(_) | DownloadError::SegmentDownloadFailed(_) | DownloadError::ServiceUnavailable(_)
                            );

                            download_adaptive
                                .lock()
                                .await
                                .on_failure();

                            if is_retryable && job.retry_count + 1 < job.max_retries {
                                let delay = Duration::from_secs(
                                    RETRY_BACKOFF_SECS[job
                                        .retry_count
                                        .min(4) as usize],
                                );
                                progress_mgr_inner.update_download(
                                    slot,
                                    format!(
                                        "Retrying: {} (attempt {}/{})",
                                        job.track
                                            .title
                                            .yellow(),
                                        job.retry_count + 1,
                                        job.max_retries
                                    ),
                                );
                                tokio::time::sleep(delay).await;

                                let retry_job = DownloadJob {
                                    track: job.track,
                                    album: job.album,
                                    playlist: job.playlist,
                                    dir_mode: job.dir_mode,
                                    retry_count: job.retry_count + 1,
                                    max_retries: job.max_retries,
                                    last_error: Some(error_str),
                                };
                                queue
                                    .lock()
                                    .await
                                    .push_back(retry_job);
                                *in_flight
                                    .lock()
                                    .await -= 1;
                            } else {
                                *completed
                                    .lock()
                                    .await += 1;
                                *in_flight
                                    .lock()
                                    .await -= 1;
                                *final_fail_count
                                    .lock()
                                    .await += 1;
                                failed_jobs
                                    .lock()
                                    .await
                                    .push(
                                        (
                                            job.clone(),
                                            error_str.clone(),
                                        ),
                                    );
                                progress_shared
                                    .lock()
                                    .await
                                    .add_failure(
                                        &job.track,
                                        job.album
                                            .as_deref(),
                                        error_str.clone(),
                                    );
                                progress_mgr_inner.finish_download(
                                    slot,
                                    false,
                                    &job.track
                                        .title,
                                    &error_str,
                                );
                                progress_mgr_inner.inc_downloaded();
                            }
                        }
                    }

                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            },
        );

        worker_handles.push(handle);
    }

    for handle in worker_handles {
        let _ = handle.await;
    }

    let _success = *success_count
        .lock()
        .await;
    let fail = *final_fail_count
        .lock()
        .await;
    let failed_list = Arc::try_unwrap(failed_jobs)
        .unwrap()
        .into_inner();

    let mut progress = Arc::try_unwrap(progress_shared)
        .unwrap()
        .into_inner();
    progress.total_downloaded = progress.get_completed_count();
    progress.successful = progress.get_completed_count();

    progress.save_to_file(&progress_file)?;

    let final_concurrent = download_adaptive
        .lock()
        .await
        .get_concurrent();

    progress_mgr.finish();

    println!();
    println!(
        "{}",
        "═"
            .repeat(60)
            .bright_blue()
    );
    println!(
        "{}",
        "Download Summary"
            .cyan()
            .bold()
    );
    println!(
        "{}",
        "═"
            .repeat(60)
            .bright_blue()
    );
    println!(
        "  {} {}",
        "Total tracks:".dimmed(),
        total
            .to_string()
            .white()
    );
    println!(
        "  {} {}",
        "Successfully matched:".green(),
        progress
            .total_matched
            .to_string()
            .green()
    );

    let failed_matches_count = progress
        .failed_matches
        .len();
    if failed_matches_count > 0 {
        println!(
            "  {} {}",
            "Failed matches:".red(),
            failed_matches_count
                .to_string()
                .red()
        );
        for fm in &progress.failed_matches {
            println!(
                "    {} {}",
                "•".red(),
                fm.track_name
                    .white()
            );
        }
    }

    println!(
        "  {} {}",
        "Successfully downloaded:".green(),
        progress
            .get_completed_count()
            .to_string()
            .green()
    );

    if fail > 0 {
        println!(
            "  {} {}",
            "Failed downloads:".red(),
            fail.to_string()
                .red()
        );
        for (job, error) in &failed_list {
            println!(
                "    {} {} - {}",
                "•".red(),
                job.track
                    .title
                    .white(),
                error.dimmed()
            );
        }
    }

    println!(
        "  {} {}",
        "Final concurrency:".dimmed(),
        final_concurrent
            .to_string()
            .yellow()
    );
    println!(
        "{}",
        "═"
            .repeat(60)
            .bright_blue()
    );
    println!(
        "{} {}",
        "Progress saved to:".dimmed(),
        progress_file
            .display()
            .to_string()
            .white()
    );

    Ok(())
}

fn clean_track_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-')
        .collect();
    let result = cleaned
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    let removed_brackets = Regex::new(r"\s*[\(\[][^)\]]*[\)\]]\s*").unwrap();
    removed_brackets
        .replace_all(
            &result, " ",
        )
        .trim()
        .to_string()
}

async fn retry_download<F, Fut, T>(f: F, adaptive: Arc<Mutex<AdaptiveConcurrency>>) -> Result<T>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    const MAX_RETRIES: usize = 5;
    const BACKOFF_DELAYS: [Duration; 5] = [
        Duration::from_secs(5),
        Duration::from_secs(15),
        Duration::from_secs(30),
        Duration::from_secs(60),
        Duration::from_secs(120),
    ];

    let mut last_error = None;

    for attempt in 0..MAX_RETRIES {
        match f().await {
            Ok(result) => return Ok(result),
            Err(e) => {
                let should_retry = matches!(
                    e,
                    DownloadError::RateLimited | DownloadError::NetworkError(_) | DownloadError::DownloadFailed(_) | DownloadError::SegmentDownloadFailed(_) | DownloadError::ServiceUnavailable(_)
                );

                if !should_retry || attempt == MAX_RETRIES - 1 {
                    return Err(e);
                }

                if should_retry {
                    adaptive
                        .lock()
                        .await
                        .on_failure();
                }

                last_error = Some(e);
                tokio::time::sleep(BACKOFF_DELAYS[attempt]).await;
            }
        }
    }

    Err(last_error.unwrap_or(DownloadError::DownloadFailed("Max retries exceeded".to_string())))
}
