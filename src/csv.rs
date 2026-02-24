use crate::{
    error::Result,
    services::{tidal::TidalService, MusicService},
    types::TrackInfo,
};
use colored::Colorize;
use serde::Deserialize;
use std::{fs::File, io::BufReader, path::Path};

#[derive(Debug, Clone, Deserialize)]
pub struct CsvTrack {
    #[serde(rename = "Track name")]
    pub track_name: String,
    #[serde(rename = "Artist name")]
    pub artist_name: String,
    #[serde(rename = "Album")]
    pub album: Option<String>,
    #[serde(rename = "Playlist name")]
    pub playlist_name: Option<String>,
    #[serde(rename = "Type")]
    pub track_type: Option<String>,
    #[serde(rename = "ISRC")]
    pub isrc: Option<String>,
    #[serde(rename = "Spotify - id")]
    pub spotify_id: Option<String>,
}

impl CsvTrack {
    pub fn parse_artists(&self) -> Vec<String> {
        let artists: Vec<String> = self
            .artist_name
            .split(
                &[
                    ',', '&', '/', ';',
                ][..],
            )
            .map(
                |s| {
                    s.trim()
                        .to_string()
                },
            )
            .filter(|s| !s.is_empty())
            .collect();

        if artists.is_empty() {
            vec![
                self.artist_name
                    .clone(),
            ]
        } else {
            artists
        }
    }
}

#[derive(Debug, Clone)]
pub struct CsvMatchResult {
    pub csv_track: CsvTrack,
    pub best_match: Option<TrackInfo>,
    pub confidence: f64,
    pub search_attempts: Vec<String>,
}

#[derive(Clone)]
pub struct CsvMatcher {
    tidal: TidalService,
}

impl CsvMatcher {
    pub fn new() -> Self {
        CsvMatcher {
            tidal: TidalService::new(),
        }
    }

    pub async fn find_best_match(&self, csv_track: &CsvTrack) -> Result<CsvMatchResult> {
        let mut search_attempts = Vec::new();
        let mut candidates: Vec<TrackInfo> = Vec::new();

        let artists = csv_track.parse_artists();

        for artist in &artists {
            search_attempts.push(
                format!(
                    "'{}' by '{}'",
                    csv_track.track_name, artist
                ),
            );

            if let Ok(results) = self
                .tidal
                .search(
                    &csv_track.track_name,
                    Some(artist),
                )
                .await
            {
                candidates.extend(results.tracks);
            }
        }

        search_attempts.push(
            format!(
                "'{}' (track only)",
                csv_track.track_name
            ),
        );

        if let Ok(results) = self
            .tidal
            .search(
                &csv_track.track_name,
                None,
            )
            .await
        {
            candidates.extend(results.tracks);
        }

        candidates.sort_by_key(|t| t.id);
        candidates.dedup_by_key(|t| t.id);

        let best = Self::score_and_select_best(
            &candidates,
            csv_track,
        );

        Ok(
            CsvMatchResult {
                csv_track: csv_track.clone(),
                best_match: best
                    .as_ref()
                    .map(|(track, _)| track.clone()),
                confidence: best
                    .map(|(_, score)| score)
                    .unwrap_or(0.0),
                search_attempts,
            },
        )
    }

    pub fn score_and_select_best(
        candidates: &[TrackInfo],
        csv_track: &CsvTrack,
    ) -> Option<(
        TrackInfo,
        f64,
    )> {
        if candidates.is_empty() {
            return None;
        }

        let mut best_score = 0.0_f64;
        let mut best_match: Option<TrackInfo> = None;

        let csv_title_lower = csv_track
            .track_name
            .to_lowercase();
        let csv_artists = csv_track.parse_artists();
        let csv_artists_lower: Vec<String> = csv_artists
            .iter()
            .map(|a| a.to_lowercase())
            .collect();
        let csv_album_lower = csv_track
            .album
            .as_ref()
            .map(|a| a.to_lowercase());

        for candidate in candidates {
            let mut score = 0.0_f64;

            let title_lower = candidate
                .title
                .to_lowercase();

            let title_similarity: f64 = if title_lower == csv_title_lower {
                1.0
            } else if title_lower.contains(&csv_title_lower) || csv_title_lower.contains(&title_lower) {
                0.85
            } else {
                strsim::jaro_winkler(
                    &title_lower,
                    &csv_title_lower,
                )
            };
            score += title_similarity * 0.45;

            if !candidate
                .artist
                .is_empty()
                && !csv_artists_lower.is_empty()
            {
                let artist_lower = candidate
                    .artist
                    .to_lowercase();

                let artist_similarity: f64 = csv_artists_lower
                    .iter()
                    .map(
                        |csv_artist| {
                            let sim: f64 = if artist_lower.contains(csv_artist) || csv_artist.contains(&artist_lower) {
                                1.0
                            } else {
                                strsim::jaro_winkler(
                                    &artist_lower,
                                    csv_artist,
                                )
                            };
                            sim
                        },
                    )
                    .fold(
                        0.0_f64,
                        |max, sim| max.max(sim),
                    );

                score += artist_similarity * 0.35;
            }

            if let Some(ref csv_album) = csv_album_lower {
                if let Some(ref candidate_album) = candidate.album {
                    let album_lower = candidate_album.to_lowercase();

                    let album_similarity: f64 = if album_lower == *csv_album {
                        1.0
                    } else if album_lower.contains(csv_album) || csv_album.contains(&album_lower) {
                        0.9
                    } else {
                        strsim::jaro_winkler(
                            &album_lower,
                            csv_album,
                        )
                    };

                    score += album_similarity * 0.20;
                }
            } else {
                score += 0.10;
            }

            if score > best_score {
                best_score = score;
                best_match = Some(candidate.clone());
            }
        }

        best_match.map(
            |m| {
                (
                    m, best_score,
                )
            },
        )
    }
}

pub fn parse_csv_file(path: &Path) -> Result<Vec<CsvTrack>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);

    let mut csv_r = csv::Reader::from_reader(reader);
    let mut tracks = Vec::new();

    for result in csv_r.deserialize() {
        let track: CsvTrack = result?;
        tracks.push(track);
    }

    Ok(tracks)
}

pub fn print_csv_summary(results: &[CsvMatchResult]) {
    println!();
    println!(
        "{}",
        "═"
            .repeat(70)
            .bright_blue()
    );
    println!(
        "{}",
        "CSV Import Summary"
            .cyan()
            .bold()
    );
    println!(
        "{}",
        "═"
            .repeat(70)
            .bright_blue()
    );

    let matched = results
        .iter()
        .filter(
            |r| {
                r.best_match
                    .is_some()
            },
        )
        .count();
    let unmatched = results.len() - matched;

    println!(
        "{} {} / {} {}",
        "Matched:".green(),
        matched
            .to_string()
            .green()
            .bold(),
        "Unmatched:".red(),
        unmatched
            .to_string()
            .red()
    );
    println!();

    for result in results {
        match &result.best_match {
            Some(track) => {
                let confidence_color = if result.confidence > 0.8 {
                    "✓".green()
                } else if result.confidence > 0.6 {
                    "~".yellow()
                } else {
                    "?".red()
                };

                println!(
                    "  {} {} {} {}",
                    confidence_color,
                    format!(
                        "[{:.0}%]",
                        result.confidence * 100.0
                    )
                    .dimmed(),
                    result
                        .csv_track
                        .track_name
                        .white(),
                    format!(
                        "→ {}",
                        track.title
                    )
                    .cyan()
                );
                println!(
                    "      {} {}",
                    "Artist:".dimmed(),
                    track
                        .artist
                        .white()
                );
                if let Some(ref album) = track.album {
                    println!(
                        "      {} {}",
                        "Album:".dimmed(),
                        album.white()
                    );
                }
            }
            None => {
                println!(
                    "  {} {}",
                    "✗".red(),
                    result
                        .csv_track
                        .track_name
                        .white()
                );
                println!(
                    "    {} {}",
                    "Searched:".dimmed(),
                    result
                        .search_attempts
                        .join(", ")
                        .white()
                );
            }
        }
    }

    println!(
        "{}",
        "═"
            .repeat(70)
            .bright_blue()
    );
    println!();
}
