use crate::{
    error::{DownloadError, Result},
    services::MusicService,
    types::*,
};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use regex::Regex;
use reqwest::Client;
use serde::Deserialize;

const API_BASE: &str = "https://hund.qqdl.site";
const FALLBACK_APIS: &[&str] = &[
    "https://katze.qqdl.site",
    "https://maus.qqdl.site",
    "https://vogel.qqdl.site",
    "https://wolf.qqdl.site",
];

#[derive(Debug, Deserialize)]
struct ApiResponse<T> {
    data: T,
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    items: Vec<TidalTrack>,
}

#[derive(Debug, Deserialize)]
struct TidalTrack {
    id: u64,
    title: String,
    #[serde(default)]
    artist: Option<TidalArtist>,
    #[serde(default)]
    album: Option<TidalAlbum>,
    #[serde(default)]
    duration: Option<u32>,
    #[serde(
        rename = "audioQuality",
        default
    )]
    audio_quality: Option<String>,
    #[serde(default)]
    cover: Option<String>,
    #[serde(default)]
    isrc: Option<String>,
    #[serde(
        rename = "spotifyId",
        default
    )]
    spotify_id: Option<String>,
    #[serde(
        rename = "albumId",
        default
    )]
    album_id: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct TidalArtist {
    name: String,
}

#[derive(Debug, Deserialize)]
struct TidalAlbum {
    title: String,
    #[serde(default)]
    cover: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TrackInfoData {
    id: u64,
    title: String,
    #[serde(default)]
    artist: Option<TidalArtist>,
    #[serde(default)]
    album: Option<TidalAlbum>,
    #[serde(default)]
    duration: Option<u32>,
    #[serde(
        rename = "audioQuality",
        default
    )]
    audio_quality: Option<String>,
    #[serde(default)]
    cover: Option<String>,
    #[serde(default)]
    isrc: Option<String>,
    #[serde(
        rename = "spotifyId",
        default
    )]
    spotify_id: Option<String>,
    #[serde(
        rename = "albumId",
        default
    )]
    album_id: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct TrackManifestData {
    #[serde(default)]
    manifest: Option<String>,
    #[serde(
        rename = "bitDepth",
        default
    )]
    bit_depth: Option<u8>,
    #[serde(
        rename = "sampleRate",
        default
    )]
    sample_rate: Option<u32>,
    #[serde(
        rename = "audioQuality",
        default
    )]
    audio_quality: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AlbumData {
    id: u64,
    title: String,
    #[serde(default)]
    artist: Option<TidalArtist>,
    #[serde(
        rename = "numberOfTracks",
        default
    )]
    number_of_tracks: Option<u32>,
    #[serde(default)]
    cover: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PlaylistData {
    id: u64,
    title: String,
    #[serde(default)]
    creator: Option<TidalCreator>,
    #[serde(
        rename = "numberOfTracks",
        default
    )]
    number_of_tracks: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct TidalCreator {
    name: String,
}

#[derive(Debug, Deserialize)]
struct ManifestJson {
    #[serde(default)]
    urls: Vec<String>,
    #[serde(rename = "mimeType", default)]
    mime_type: Option<String>,
}

use std::sync::atomic::{AtomicUsize, Ordering};

static CURRENT_API_INDEX: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone)]
pub struct TidalService {
    client: Client,
}

impl TidalService {
    pub fn new() -> Self {
        TidalService {
            client: Client::builder()
                .user_agent("SquidDownloader/0.1.0")
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap(),
        }
    }

    fn get_api_base() -> &'static str {
        let index = CURRENT_API_INDEX.load(Ordering::Relaxed);
        if index == 0 {
            API_BASE
        } else if index <= FALLBACK_APIS.len() {
            FALLBACK_APIS[index - 1]
        } else {
            API_BASE
        }
    }

    fn parse_quality(quality_str: &str) -> AudioQuality {
        match quality_str
            .to_uppercase()
            .as_str()
        {
            "HI_RES_LOSSLESS" | "HI_RES" => AudioQuality::HiResLossless,
            "LOSSLESS" => AudioQuality::Lossless,
            "HIGH" => AudioQuality::High,
            "LOW" => AudioQuality::Low,
            _ => AudioQuality::Auto,
        }
    }

    fn format_cover_url(uuid: Option<String>) -> Option<String> {
        uuid.map(|id| format!("https://resources.tidal.com/images/{}/1280x1280.jpg", id.replace('-', "/")))
    }

    fn map_track_to_info(track: TidalTrack) -> TrackInfo {
        let cover = track.cover.or_else(|| track.album.as_ref().and_then(|a| a.cover.clone()));
        TrackInfo {
            id: track.id,
            title: track.title,
            artist: track
                .artist
                .map(|a| a.name)
                .unwrap_or_default(),
            album: track
                .album
                .as_ref()
                .map(|a| a.title.clone()),
            duration: track.duration,
            quality: track
                .audio_quality
                .map(|q| Self::parse_quality(&q)),
            cover_url: Self::format_cover_url(cover),
            isrc: track.isrc,
            spotify_id: track.spotify_id,
            album_id: track.album_id,
        }
    }

    fn parse_mpd_manifest(mpd_xml: &str) -> Result<(Vec<String>, Option<String>)> {
        let mut segment_urls = Vec::new();
        let mut base_url: Option<String> = None;
        let mut mime_type: Option<String> = None;

        let mime_re = Regex::new(r#"mimeType="([^"]+)""#).unwrap();
        if let Some(caps) = mime_re.captures(mpd_xml) {
            mime_type = Some(caps[1].to_string());
        }

        let re = Regex::new(r"<BaseURL>([^<]+)</BaseURL>").unwrap();
        if let Some(caps) = re.captures(mpd_xml) {
            base_url = Some(caps[1].to_string());
        }

        // Try to parse SegmentTemplate
        let template_re = Regex::new(r#"<SegmentTemplate[^>]*initialization="([^"]+)"[^>]*media="([^"]+)"[^>]*startNumber="([^"]+)""#).unwrap();
        if let Some(caps) = template_re.captures(mpd_xml) {
            let init_url = caps[1].to_string();
            let media_url = caps[2].to_string();
            let start_number: u32 = caps[3].parse().unwrap_or(1);

            let init_full = if let Some(ref base) = base_url {
                if base.ends_with('/') { format!("{}{}", base, init_url) } else { format!("{}/{}", base, init_url) }
            } else { init_url };
            segment_urls.push(init_full);

            // Parse SegmentTimeline to get total segments
            let s_re = Regex::new(r#"<S\s+d="[^"]+"(?:\s+r="([^"]+)")?\s*/>"#).unwrap();
            let mut total_segments = 0;
            for caps in s_re.captures_iter(mpd_xml) {
                let r: u32 = caps.get(1).map_or(0, |m| m.as_str().parse().unwrap_or(0));
                total_segments += 1 + r;
            }

            for i in 0..total_segments {
                let num = start_number + i;
                let seg_url = media_url.replace("$Number$", &num.to_string());
                let seg_full = if let Some(ref base) = base_url {
                    if base.ends_with('/') { format!("{}{}", base, seg_url) } else { format!("{}/{}", base, seg_url) }
                } else { seg_url };
                segment_urls.push(seg_full);
            }
            
            if !segment_urls.is_empty() {
                return Ok((segment_urls, mime_type));
            }
        }

        // Fallback to old behavior
        let segment_re = Regex::new(r#"media="([^"]+)""#).unwrap();
        for caps in segment_re.captures_iter(mpd_xml) {
            let segment = caps[1].to_string();
            if segment.contains("$Number$") {
                continue; // Skip if we caught a template incorrectly above
            }
            if let Some(ref base) = base_url {
                let full_url = if base.ends_with('/') {
                    format!("{}{}", base, segment)
                } else {
                    format!("{}/{}", base, segment)
                };
                segment_urls.push(full_url);
            } else {
                segment_urls.push(segment);
            }
        }

        if segment_urls.is_empty() {
            let seg_pattern = r#"<SegmentURL[^>]*media="([^"]+)""#;
            let seg_re = Regex::new(seg_pattern).unwrap();
            for caps in seg_re.captures_iter(mpd_xml) {
                segment_urls.push(caps[1].to_string());
            }
        }

        if segment_urls.is_empty() {
            return Err(DownloadError::XmlParseError("Could not parse segment URLs from MPD manifest".to_string()));
        }

        Ok((segment_urls, mime_type))
    }

    async fn make_request<T: for<'de> Deserialize<'de>>(&self, endpoint: &str) -> Result<T> {
        let mut last_error = None;
        let total_apis = 1 + FALLBACK_APIS.len();
        
        for _ in 0..total_apis {
            let current_index = CURRENT_API_INDEX.load(Ordering::Relaxed);
            let api = Self::get_api_base();
            let url = format!("{}{}", api, endpoint);

            let response = match self.client.get(&url).send().await {
                Ok(r) => r,
                Err(e) => {
                    last_error = Some(DownloadError::NetworkError(e));
                    let _ = CURRENT_API_INDEX.compare_exchange(current_index, (current_index + 1) % total_apis, Ordering::SeqCst, Ordering::SeqCst);
                    continue;
                }
            };

            if response.status() == 429 {
                return Err(DownloadError::RateLimited);
            }

            let status = response.status();
            if !status.is_success() {
                last_error = Some(DownloadError::ServiceUnavailable(format!("API returned status {}", status)));
                let _ = CURRENT_API_INDEX.compare_exchange(current_index, (current_index + 1) % total_apis, Ordering::SeqCst, Ordering::SeqCst);
                continue;
            }

            let text = match response.text().await {
                Ok(t) => t,
                Err(e) => {
                    last_error = Some(DownloadError::NetworkError(e));
                    let _ = CURRENT_API_INDEX.compare_exchange(current_index, (current_index + 1) % total_apis, Ordering::SeqCst, Ordering::SeqCst);
                    continue;
                }
            };

            match serde_json::from_str(&text) {
                Ok(data) => return Ok(data),
                Err(e) => {
                    last_error = Some(DownloadError::JsonParseError(e));
                    let _ = CURRENT_API_INDEX.compare_exchange(current_index, (current_index + 1) % total_apis, Ordering::SeqCst, Ordering::SeqCst);
                    continue;
                }
            }
        }

        Err(last_error.unwrap_or(DownloadError::ServiceUnavailable("All APIs failed".to_string())))
    }
}

impl Default for TidalService {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MusicService for TidalService {
    async fn search(&self, query: &str, artist: Option<&str>) -> Result<SearchResults> {
        let search_query = match artist {
            Some(a) => format!(
                "{} {}",
                query, a
            ),
            None => query.to_string(),
        };

        let encoded = urlencoding::encode(&search_query);
        let endpoint = format!(
            "/search/?s={}",
            encoded
        );

        let response: ApiResponse<SearchResponse> = self
            .make_request(&endpoint)
            .await?;

        let tracks: Vec<TrackInfo> = response
            .data
            .items
            .into_iter()
            .map(Self::map_track_to_info)
            .collect();

        let filtered_tracks = if let Some(artist_name) = artist {
            tracks
                .into_iter()
                .filter(
                    |t| {
                        strsim::jaro_winkler(
                            t.artist
                                .to_lowercase()
                                .as_str(),
                            artist_name
                                .to_lowercase()
                                .as_str(),
                        ) > 0.7
                    },
                )
                .collect()
        } else {
            tracks
        };

        Ok(
            SearchResults {
                tracks: filtered_tracks,
            },
        )
    }

    async fn get_track_info(&self, track_id: u64) -> Result<TrackInfo> {
        let endpoint = format!(
            "/info/?id={}",
            track_id
        );
        let response: ApiResponse<TrackInfoData> = self
            .make_request(&endpoint)
            .await?;

        Ok(
            TrackInfo {
                id: response
                    .data
                    .id,
                title: response
                    .data
                    .title,
                artist: response
                    .data
                    .artist
                    .map(|a| a.name)
                    .unwrap_or_default(),
                album: response
                    .data
                    .album
                    .as_ref()
                    .map(|a| a.title.clone()),
                duration: response
                    .data
                    .duration,
                quality: response
                    .data
                    .audio_quality
                    .map(|q| Self::parse_quality(&q)),
                cover_url: Self::format_cover_url(response.data.cover.or_else(|| response.data.album.as_ref().and_then(|a| a.cover.clone()))),
                isrc: response
                    .data
                    .isrc,
                spotify_id: response
                    .data
                    .spotify_id,
                album_id: response
                    .data
                    .album_id,
            },
        )
    }

    async fn get_album_info(&self, album_id: u64) -> Result<AlbumInfo> {
        let endpoint = format!(
            "/album/?id={}",
            album_id
        );
        let response: ApiResponse<AlbumData> = self
            .make_request(&endpoint)
            .await?;

        Ok(
            AlbumInfo {
                id: response
                    .data
                    .id,
                title: response
                    .data
                    .title,
                artist: response
                    .data
                    .artist
                    .map(|a| a.name)
                    .unwrap_or_default(),
                track_count: response
                    .data
                    .number_of_tracks
                    .unwrap_or(0),
                year: None,
                cover_url: Self::format_cover_url(response.data.cover),
            },
        )
    }

    async fn get_playlist_info(&self, playlist_id: u64) -> Result<PlaylistInfo> {
        let endpoint = format!(
            "/playlist/?id={}",
            playlist_id
        );
        let response: ApiResponse<PlaylistData> = self
            .make_request(&endpoint)
            .await?;

        Ok(
            PlaylistInfo {
                id: response
                    .data
                    .id,
                title: response
                    .data
                    .title,
                creator: response
                    .data
                    .creator
                    .map(|c| c.name)
                    .unwrap_or_default(),
                track_count: response
                    .data
                    .number_of_tracks
                    .unwrap_or(0),
                cover_url: None,
            },
        )
    }

    async fn get_album_tracks(&self, album_id: u64) -> Result<Vec<TrackInfo>> {
        let endpoint = format!(
            "/album/tracks/?id={}",
            album_id
        );
        let response: ApiResponse<SearchResponse> = self
            .make_request(&endpoint)
            .await?;

        Ok(
            response
                .data
                .items
                .into_iter()
                .map(Self::map_track_to_info)
                .collect(),
        )
    }

    async fn get_playlist_tracks(&self, playlist_id: u64) -> Result<Vec<TrackInfo>> {
        let endpoint = format!(
            "/playlist/tracks/?id={}",
            playlist_id
        );
        let response: ApiResponse<SearchResponse> = self
            .make_request(&endpoint)
            .await?;

        Ok(
            response
                .data
                .items
                .into_iter()
                .map(Self::map_track_to_info)
                .collect(),
        )
    }

    async fn get_manifest(&self, track_id: u64, quality: AudioQuality) -> Result<DownloadManifest> {
        let endpoint = format!(
            "/track/?id={}&audioquality={}",
            track_id,
            quality.as_str()
        );
        let response: ApiResponse<TrackManifestData> = self
            .make_request(&endpoint)
            .await?;

        let manifest_b64 = response
            .data
            .manifest
            .ok_or(DownloadError::NoDownloadUrl)?;
        let manifest_decoded = BASE64.decode(&manifest_b64)?;
        let manifest_str = String::from_utf8_lossy(&manifest_decoded);

        let bit_depth = response
            .data
            .bit_depth;
        let sample_rate = response
            .data
            .sample_rate;

        let actual_quality = response.data.audio_quality.as_deref().map(Self::parse_quality).unwrap_or(quality);

        if manifest_str.contains("application/dash+xml") || manifest_str.contains("<MPD") {
            let (segment_urls, dash_mime) = Self::parse_mpd_manifest(&manifest_str)?;
            Ok(
                DownloadManifest {
                    url: None,
                    segment_urls: Some(segment_urls),
                    quality: actual_quality,
                    bit_depth,
                    sample_rate,
                    mime_type: dash_mime.or_else(|| Some("application/dash+xml".to_string())),
                },
            )
        } else {
            let manifest_data: ManifestJson = serde_json::from_str(&manifest_str)?;
            let url = manifest_data
                .urls
                .into_iter()
                .next()
                .ok_or(DownloadError::NoDownloadUrl)?;

            Ok(
                DownloadManifest {
                    url: Some(url),
                    segment_urls: None,
                    quality: actual_quality,
                    bit_depth,
                    sample_rate,
                    mime_type: manifest_data.mime_type,
                },
            )
        }
    }
}

mod urlencoding {
    pub fn encode(s: &str) -> String {
        url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
    }
}
