use crate::{error::Result, types::TrackInfo};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchedTrack {
    #[serde(rename = "trackId")]
    pub track_id: u64,
    #[serde(rename = "trackTitle")]
    pub track_title: String,
    #[serde(rename = "trackArtist")]
    pub track_artist: String,
    pub album: Option<String>,
    pub cover_url: Option<String>,
    pub confidence: f64,
    #[serde(rename = "csvIndex")]
    pub csv_index: usize,
    #[serde(rename = "csvTrackName")]
    pub csv_track_name: String,
    #[serde(rename = "csvArtistName")]
    pub csv_artist_name: String,
    #[serde(rename = "csvAlbum")]
    pub csv_album: Option<String>,
    #[serde(rename = "playlistName")]
    pub playlist_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailedMatch {
    #[serde(rename = "trackName")]
    pub track_name: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    #[serde(rename = "searchAttempts")]
    pub search_attempts: Vec<String>,
    pub attempts: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CsvTrackInfo {
    #[serde(rename = "trackName")]
    pub track_name: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub isrc: Option<String>,
    #[serde(rename = "spotifyId")]
    pub spotify_id: Option<String>,
    pub cover_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailedDownload {
    #[serde(rename = "trackId")]
    pub track_id: u64,
    #[serde(rename = "trackTitle")]
    pub track_title: String,
    #[serde(rename = "trackArtist")]
    pub track_artist: String,
    pub album: Option<String>,
    pub error: String,
    pub attempts: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadProgress {
    pub source: String,
    pub total: usize,
    pub successful: usize,
    pub failed: Vec<FailedDownload>,
    #[serde(rename = "startedAt")]
    pub started_at: String,
    #[serde(rename = "completedAt")]
    pub completed_at: Option<String>,
    #[serde(rename = "failedMatches")]
    pub failed_matches: Vec<FailedMatch>,
    #[serde(rename = "totalMatched")]
    pub total_matched: usize,
    #[serde(rename = "totalDownloaded")]
    pub total_downloaded: usize,
    #[serde(rename = "completedTrackIds")]
    pub completed_track_ids: Vec<u64>,
    #[serde(rename = "matchedCsvIndices")]
    pub matched_csv_indices: Vec<usize>,
    #[serde(rename = "matchedTracks")]
    pub matched_tracks: Vec<MatchedTrack>,
}

impl DownloadProgress {
    pub fn new(source: &str, total: usize) -> Self {
        Self {
            source: source.to_string(),
            total,
            successful: 0,
            failed: Vec::new(),
            started_at: Utc::now().to_rfc3339(),
            completed_at: None,
            failed_matches: Vec::new(),
            total_matched: 0,
            total_downloaded: 0,
            completed_track_ids: Vec::new(),
            matched_csv_indices: Vec::new(),
            matched_tracks: Vec::new(),
        }
    }

    pub fn mark_completed(&mut self, track_id: u64) {
        if !self
            .completed_track_ids
            .contains(&track_id)
        {
            self.completed_track_ids
                .push(track_id);
        }
    }

    pub fn is_completed(&self, track_id: u64) -> bool {
        self.completed_track_ids
            .contains(&track_id)
    }

    pub fn get_completed_count(&self) -> usize {
        self.completed_track_ids
            .len()
    }

    pub fn mark_csv_matched(&mut self, index: usize) {
        if !self
            .matched_csv_indices
            .contains(&index)
        {
            self.matched_csv_indices
                .push(index);
        }
    }

    pub fn is_csv_matched(&self, index: usize) -> bool {
        self.matched_csv_indices
            .contains(&index)
    }

    pub fn add_matched_track(&mut self, csv_index: usize, track: &TrackInfo, confidence: f64, csv_track_name: &str, csv_artist: &str, csv_album: Option<&str>, playlist: Option<&str>) {
        self.matched_tracks
            .push(
                MatchedTrack {
                    track_id: track.id,
                    track_title: track
                        .title
                        .clone(),
                    track_artist: track
                        .artist
                        .clone(),
                    album: track
                        .album
                        .clone(),
                    cover_url: track
                        .cover_url
                        .clone(),
                    confidence,
                    csv_index,
                    csv_track_name: csv_track_name.to_string(),
                    csv_artist_name: csv_artist.to_string(),
                    csv_album: csv_album.map(|s| s.to_string()),
                    playlist_name: playlist.map(|s| s.to_string()),
                },
            );
        self.mark_csv_matched(csv_index);
    }

    pub fn get_matched_track(&self, csv_index: usize) -> Option<&MatchedTrack> {
        self.matched_tracks
            .iter()
            .find(|m| m.csv_index == csv_index)
    }

    pub fn add_success(&mut self) {
        self.successful += 1;
    }

    pub fn add_failure(&mut self, track: &TrackInfo, album: Option<&str>, error: String) {
        self.failed
            .push(
                FailedDownload {
                    track_id: track.id,
                    track_title: track
                        .title
                        .clone(),
                    track_artist: track
                        .artist
                        .clone(),
                    album: album.map(|s| s.to_string()),
                    error,
                    attempts: 1,
                },
            );
    }

    pub fn add_failed_match(&mut self, track_name: &str, artist: Option<&str>, album: Option<&str>) {
        if let Some(existing) = self
            .failed_matches
            .iter_mut()
            .find(
                |m| {
                    m.track_name == track_name
                        && m.artist
                            .as_deref()
                            == artist
                        && m.album
                            .as_deref()
                            == album
                },
            )
        {
            existing.attempts += 1;
        } else {
            self.failed_matches
                .push(
                    FailedMatch {
                        track_name: track_name.to_string(),
                        artist: artist.map(|s| s.to_string()),
                        album: album.map(|s| s.to_string()),
                        search_attempts: Vec::new(),
                        attempts: 1,
                    },
                );
        }
    }

    pub fn get_failed_matches(&self) -> &[FailedMatch] {
        &self.failed_matches
    }

    pub fn total_failed(&self) -> usize {
        self.failed_matches
            .len()
            + self
                .failed
                .len()
    }

    pub fn save_to_file(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(
            path, json,
        )?;
        Ok(())
    }

    pub fn load_from_file(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let progress: Self = serde_json::from_str(&content)?;
        Ok(progress)
    }

    pub fn has_failures(&self) -> bool {
        !self
            .failed
            .is_empty()
    }

    pub fn retry_count(&self) -> usize {
        self.failed
            .iter()
            .map(|f| f.attempts)
            .sum()
    }
}
