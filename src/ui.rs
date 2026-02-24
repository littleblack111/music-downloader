use crate::types::*;
use colored::Colorize;
use std::io::{self, Write};
use tabled::{
    settings::{object::Rows, Alignment, Modify, Style},
    Table, Tabled,
};

#[derive(Tabled)]
struct TrackRow {
    #[tabled(rename = "#")]
    index: usize,
    #[tabled(rename = "Title")]
    title: String,
    #[tabled(rename = "Artist")]
    artist: String,
    #[tabled(rename = "Album")]
    album: String,
    #[tabled(rename = "Duration")]
    duration: String,
}

pub struct Ui;

impl Ui {
    pub fn print_banner() {
        println!();
        println!(
            "{}",
            r#"
  ██████  ██▓███   ▄▄▄       █     █░ ▄▄▄       ███▄    █ 
▒██    ▒ ▓██░  ██▒▒████▄    ▓█░ █ ░█░▒████▄     ██ ▀█   █ 
░ ▓██▄   ▓██░ ██▓▒▒██  ▀█▄  ▒█░ █ ░█ ▒██  ▀█▄  ▓██  ▀█ ██▒
  ▒   ██▒▒██▄█▓▒ ▒░██▄▄▄▄██ ░█░ █ ░█ ░██▄▄▄▄██ ▓██▒  ▐▌██▒
▒██████▒▒▒██▒ ░  ░ ▓█   ▓██▒░░██▒██▓  ▓█   ▓██▒▒██░   ▓██░
▒ ▒▓▒ ▒ ░▒▓▒░ ░  ░ ▒▒   ▓▒█░░ ▓░▒ ▒   ▒▒   ▓▒█░░ ▒░   ▒ ▒ 
░ ░▒  ░ ░░▒ ░       ▒   ▒▒ ░  ▒ ░ ░    ▒   ▒▒ ░░ ░░   ░ ▒░
░  ░  ░  ░░         ░   ▒     ░   ░    ░   ▒      ░   ░ ░ 
      ░                  ░  ░  ░            ░  ░         ░ 
"#
            .bright_cyan()
        );
        println!(
            "  {} {} • {} {}",
            "High-quality music downloader".white(),
            "v0.1.0".yellow(),
            "Supports:".white(),
            "Tidal".green()
        );
        println!();
    }

    pub fn print_info(message: &str) {
        println!(
            "{} {}",
            "ℹ"
                .blue()
                .bold(),
            message.white()
        );
    }

    pub fn print_success(message: &str) {
        println!(
            "{} {}",
            "✓"
                .green()
                .bold(),
            message.white()
        );
    }

    pub fn print_warning(message: &str) {
        println!(
            "{} {}",
            "!".yellow()
                .bold(),
            message.white()
        );
    }

    pub fn print_error(message: &str) {
        eprintln!(
            "{} {}",
            "✗"
                .red()
                .bold(),
            message.white()
        );
    }

    pub fn print_search_results(tracks: &[TrackInfo], limit: usize) {
        if tracks.is_empty() {
            Self::print_warning("No results found");
            return;
        }

        let rows: Vec<TrackRow> = tracks
            .iter()
            .take(limit)
            .enumerate()
            .map(
                |(i, t)| TrackRow {
                    index: i + 1,
                    title: t
                        .title
                        .clone(),
                    artist: t
                        .artist
                        .clone(),
                    album: t
                        .album
                        .clone()
                        .unwrap_or_else(|| "-".to_string()),
                    duration: t.format_duration(),
                },
            )
            .collect();

        let mut table = Table::new(rows);
        table
            .with(Style::rounded())
            .with(Modify::new(Rows::new(1..)).with(Alignment::left()));

        println!();
        println!(
            "{}",
            table
        );
        println!();
    }

    pub fn print_track_detail(track: &TrackInfo, index: Option<usize>) {
        let prefix = index
            .map(
                |i| {
                    format!(
                        "{}.",
                        i
                    )
                },
            )
            .unwrap_or_default();

        println!(
            "{} {}",
            prefix.cyan(),
            track
                .title
                .white()
                .bold()
        );
        println!(
            "    {} {}",
            "Artist:".dimmed(),
            track
                .artist
                .white()
        );
        if let Some(ref album) = track.album {
            println!(
                "    {} {}",
                "Album:".dimmed(),
                album.white()
            );
        }
        println!(
            "    {} {}",
            "Duration:".dimmed(),
            track
                .format_duration()
                .white()
        );
        if let Some(ref quality) = track.quality {
            println!(
                "    {} {}",
                "Quality:".dimmed(),
                quality
                    .to_string()
                    .green()
            );
        }
        println!(
            "    {} {}",
            "ID:".dimmed(),
            track
                .id
                .to_string()
                .yellow()
        );
    }

    pub fn print_album_detail(album: &AlbumInfo) {
        println!();
        println!(
            "{}",
            album
                .title
                .white()
                .bold()
        );
        println!(
            "  {} {}",
            "Artist:".cyan(),
            album
                .artist
                .white()
        );
        println!(
            "  {} {} tracks",
            "Tracks:".cyan(),
            album
                .track_count
                .to_string()
                .white()
        );
        if let Some(ref year) = album.year {
            println!(
                "  {} {}",
                "Year:".cyan(),
                year.to_string()
                    .white()
            );
        }
        println!();
    }

    pub fn print_playlist_detail(playlist: &PlaylistInfo) {
        println!();
        println!(
            "{}",
            playlist
                .title
                .white()
                .bold()
        );
        println!(
            "  {} {}",
            "Creator:".cyan(),
            playlist
                .creator
                .white()
        );
        println!(
            "  {} {} tracks",
            "Tracks:".cyan(),
            playlist
                .track_count
                .to_string()
                .white()
        );
        println!();
    }

    pub fn print_qualities() {
        println!();
        println!(
            "{}",
            "Available Audio Qualities:"
                .cyan()
                .bold()
        );
        println!();

        for quality in AudioQuality::all() {
            println!(
                "  {} {}",
                format!(
                    "{:15}",
                    quality.as_str()
                )
                .yellow(),
                quality
                    .description()
                    .white()
            );
        }
        println!();
    }

    pub fn print_services() {
        println!();
        println!(
            "{}",
            "Available Services:"
                .cyan()
                .bold()
        );
        println!();

        println!(
            "  {} {}",
            format!(
                "{:12}",
                "tidal"
            )
            .yellow(),
            "Tidal (Hi-Res FLAC)".white()
        );
        println!();
    }

    pub fn select_track(tracks: &[TrackInfo]) -> Option<usize> {
        if tracks.is_empty() {
            Self::print_warning("No tracks to select from");
            return None;
        }

        if tracks.len() == 1 {
            return Some(0);
        }

        Self::print_search_results(
            tracks,
            tracks.len(),
        );
        print!(
            "{} {}",
            "?".green()
                .bold(),
            "Select track (1-".white(),
        );
        print!(
            "{}): ",
            tracks
                .len()
                .to_string()
                .white()
        );

        io::stdout()
            .flush()
            .ok()?;

        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .ok()?;

        let selection: usize = input
            .trim()
            .parse()
            .ok()?;

        if selection == 0 || selection > tracks.len() {
            Self::print_error(
                &format!(
                    "Invalid selection: {}",
                    selection
                ),
            );
            return None;
        }

        Some(selection - 1)
    }

    pub fn confirm(message: &str) -> bool {
        print!(
            "{} {} [y/N]: ",
            "?".green()
                .bold(),
            message
        );
        io::stdout()
            .flush()
            .ok()
            .unwrap_or(());

        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .ok()
            .unwrap_or_default();

        matches!(
            input
                .trim()
                .to_lowercase()
                .as_str(),
            "y" | "yes"
        )
    }
}
