use crate::{
    error::{DownloadError, Result},
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
    #[serde(rename = "Album", default)]
    pub album: Option<String>,
    #[serde(rename = "Playlist name", default)]
    pub playlist_name: Option<String>,
    #[serde(rename = "Type", default)]
    pub track_type: Option<String>,
    #[serde(rename = "ISRC", default)]
    pub isrc: Option<String>,
    #[serde(rename = "Spotify - id", default)]
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

            match self
                .tidal
                .search(
                    &csv_track.track_name,
                    Some(artist),
                )
                .await
            {
                Ok(results) => candidates.extend(results.tracks),
                Err(DownloadError::RateLimited) => {
                    return Err(DownloadError::RateLimited);
                }
                Err(_) => {}
            }
        }

        search_attempts.push(
            format!(
                "'{}' (track only)",
                csv_track.track_name
            ),
        );

        match self
            .tidal
            .search(
                &csv_track.track_name,
                None,
            )
            .await
        {
            Ok(results) => candidates.extend(results.tracks),
            Err(DownloadError::RateLimited) => {
                return Err(DownloadError::RateLimited);
            }
            Err(_) => {}
        }

        // If CSV provided an ISRC or Spotify id, try a targeted search with that
        // identifier Some proxy APIs index by ISRC/spotify id and will return
        // the exact track.
        let mut id_based_candidates: Vec<TrackInfo> = Vec::new();
        if let Some(ref isrc) = csv_track.isrc {
            if !isrc
                .trim()
                .is_empty()
            {
                match self
                    .tidal
                    .search(
                        isrc, None,
                    )
                    .await
                {
                    Ok(r) => id_based_candidates.extend(r.tracks),
                    Err(DownloadError::RateLimited) => return Err(DownloadError::RateLimited),
                    Err(_) => {}
                }
            }
        }
        if id_based_candidates.is_empty() {
            if let Some(ref spid) = csv_track.spotify_id {
                if !spid
                    .trim()
                    .is_empty()
                {
                    match self
                        .tidal
                        .search(
                            spid, None,
                        )
                        .await
                    {
                        Ok(r) => id_based_candidates.extend(r.tracks),
                        Err(DownloadError::RateLimited) => return Err(DownloadError::RateLimited),
                        Err(_) => {}
                    }
                }
            }
        }

        candidates.sort_by_key(|t| t.id);
        candidates.dedup_by_key(|t| t.id);

        // If id-based candidates were found, prefer them (they are likely exact
        // matches)
        if !id_based_candidates.is_empty() {
            id_based_candidates.sort_by_key(|t| t.id);
            id_based_candidates.dedup_by_key(|t| t.id);
            candidates = id_based_candidates;
        }

        // Filter out obvious non-music candidates (podcasts/interviews) unless CSV
        // explicitly references them
        let csv_title_lower = csv_track
            .track_name
            .to_lowercase();
        let filtered: Vec<TrackInfo> = candidates
            .into_iter()
            .filter(
                |t| {
                    let title_lower = t
                        .title
                        .to_lowercase();
                    let album_lower = t
                        .album
                        .as_deref()
                        .unwrap_or("")
                        .to_lowercase();
                    // If candidate looks like a podcast/episode/interview, exclude it unless CSV
                    // mentions podcast/episode
                    if (title_lower.contains("podcast") || title_lower.contains("episode") || title_lower.contains("interview") || album_lower.contains("podcast")) && !csv_title_lower.contains("podcast") && !csv_title_lower.contains("episode") && !csv_title_lower.contains("interview") {
                        return false;
                    }
                    true
                },
            )
            .collect();

        let best = Self::score_and_select_best(
            &filtered, csv_track,
        );

        // If we have a best match, attempt to fetch full track info (cover/album) from
        // the API to enrich the result
        let enriched_best = match best {
            Some((t, s)) => {
                // Try to get more info; ignore errors and keep original candidate if API call
                // fails
                match self
                    .tidal
                    .get_track_info(t.id)
                    .await
                {
                    Ok(full) => Some(
                        (
                            full, s,
                        ),
                    ),
                    Err(_) => Some(
                        (
                            t, s,
                        ),
                    ),
                }
            }
            None => None,
        };

        Ok(
            CsvMatchResult {
                csv_track: csv_track.clone(),
                best_match: enriched_best
                    .as_ref()
                    .map(|(track, _)| track.clone()),
                confidence: enriched_best
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
        // Delegate to score_candidates and pick the highest scoring candidate
        let mut scored = Self::score_candidates(
            candidates, csv_track,
        );
        if scored.is_empty() {
            return None;
        }
        // best entry is first after sorting
        let (best, best_score) = scored.remove(0);
        Some(
            (
                best, best_score,
            ),
        )
    }

    pub fn score_candidates(
        candidates: &[TrackInfo],
        csv_track: &CsvTrack,
    ) -> Vec<(
        TrackInfo,
        f64,
    )> {
        let mut scored: Vec<(
            TrackInfo,
            f64,
        )> = Vec::new();
        if candidates.is_empty() {
            return scored;
        }

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
            let csv_mentions_podcast = csv_title_lower.contains("podcast") || csv_title_lower.contains("episode") || csv_title_lower.contains("interview");

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

            let mut version_penalty = 0.0_f64;
            if title_lower.contains("podcast") || title_lower.contains("interview") || title_lower.contains("commentary") || title_lower.contains("speech") || title_lower.contains("talk") {
                version_penalty = if csv_mentions_podcast {
                    0.25
                } else {
                    1.5
                };
            }
            if title_lower.contains("live") || title_lower.contains("concert") || title_lower.contains("tour") || title_lower.contains("performance") {
                version_penalty = version_penalty.max(0.25);
            }
            if (title_lower.contains("remix") || title_lower.contains(" mix")) && !csv_title_lower.contains("remix") && !csv_title_lower.contains("mix") {
                version_penalty = version_penalty.max(0.20);
            }
            if title_lower.contains("radio edit") || title_lower.contains("radio version") {
                version_penalty = version_penalty.max(0.15);
            }
            if title_lower.contains("demo") || title_lower.contains("alternate") || title_lower.contains("alternative") || title_lower.contains("unreleased") {
                version_penalty = version_penalty.max(0.20);
            }
            if title_lower.contains("remaster") && !csv_title_lower.contains("remaster") {
                score += 0.05;
            }
            score -= version_penalty * title_similarity;
            score = score.max(0.0);

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
                            if artist_lower.contains(csv_artist) || csv_artist.contains(&artist_lower) {
                                1.0
                            } else {
                                strsim::jaro_winkler(
                                    &artist_lower,
                                    csv_artist,
                                )
                            }
                        },
                    )
                    .fold(
                        0.0_f64,
                        |max, sim| max.max(sim),
                    );
                score += artist_similarity * 0.35;
            }

            if let Some(ref candidate_album) = candidate.album {
                let album_lower = candidate_album.to_lowercase();
                let mut album_penalty = 0.0_f64;
                if album_lower.contains("greatest hits") || album_lower.contains("best of") || album_lower.contains("compilation") || album_lower.contains("anthology") || album_lower.contains("collection") || album_lower.contains("essential") {
                    album_penalty = 0.10;
                }
                if let Some(ref csv_album) = csv_album_lower {
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
                    score -= album_penalty;
                    if album_lower.contains(csv_album) || csv_album.contains(&album_lower) {
                        score += 0.25;
                    }
                    if (title_lower.contains("podcast") || title_lower.contains("episode"))
                        && !csv_title_lower.contains("podcast")
                        && csv_album_lower
                            .as_deref()
                            .is_none_or(
                                |a| !a.contains("podcast"),
                            )
                    {
                        score -= 0.8;
                    }
                } else {
                    score += 0.10;
                    score -= album_penalty;
                }
            }

            if csv_title_lower.contains("taylor") && csv_title_lower.contains("version") && title_lower.contains("taylor") {
                score += 0.15;
            }

            let has_identifier = csv_track
                .isrc
                .as_ref()
                .map(
                    |s| {
                        !s.trim()
                            .is_empty()
                    },
                )
                .unwrap_or(false)
                || csv_track
                    .spotify_id
                    .as_ref()
                    .map(
                        |s| {
                            !s.trim()
                                .is_empty()
                        },
                    )
                    .unwrap_or(false);
            if has_identifier {
                if title_lower == csv_title_lower {
                    score += 0.35;
                }
                if !candidate
                    .artist
                    .is_empty()
                    && csv_artists_lower
                        .iter()
                        .any(
                            |a| {
                                candidate
                                    .artist
                                    .to_lowercase()
                                    .contains(a)
                            },
                        )
                {
                    score += 0.35;
                }
            }

            // Optional debug logging when MUSIC_DL_DEBUG=1 or true
            if let Ok(val) = std::env::var("MUSIC_DL_DEBUG") {
                let v = val.to_lowercase();
                if v == "1" || v == "true" {
                    println!(
                        "DEBUG: candidate id={} title='{}' artist='{}' album='{}' score={:.3}",
                        candidate.id,
                        candidate.title,
                        candidate.artist,
                        candidate
                            .album
                            .as_deref()
                            .unwrap_or(""),
                        score
                    );
                }
            }

            scored.push(
                (
                    candidate.clone(),
                    score,
                ),
            );
        }

        // sort by score desc
        scored.sort_by(
            |a, b| {
                b.1.partial_cmp(&a.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
            },
        );
        scored
    }
}

pub fn parse_csv_file(path: &Path) -> Result<Vec<CsvTrack>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);

    let mut csv_r = csv::ReaderBuilder::new()
        .flexible(true)
        .from_reader(reader);
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
