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

const API_BASE: &str = "https://triton.squid.wtf";
const FALLBACK_APIS: &[&str] = &[
    "https://aether.squid.wtf",
    "https://zeus.squid.wtf",
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
}

#[derive(Debug, Deserialize)]
struct TidalArtist {
    name: String,
}

#[derive(Debug, Deserialize)]
struct TidalAlbum {
    title: String,
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
}

#[derive(Clone)]
pub struct TidalService {
    client: Client,
    api_index: usize,
}

impl TidalService {
    pub fn new() -> Self {
        TidalService {
            client: Client::builder()
                .user_agent("SquidDownloader/0.1.0")
                .build()
                .unwrap(),
            api_index: 0,
        }
    }

    fn get_api_base(&self) -> &str {
        if self.api_index == 0 {
            API_BASE
        } else {
            FALLBACK_APIS[self.api_index - 1]
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

    fn map_track_to_info(track: TidalTrack) -> TrackInfo {
        TrackInfo {
            id: track.id,
            title: track.title,
            artist: track
                .artist
                .map(|a| a.name)
                .unwrap_or_default(),
            album: track
                .album
                .map(|a| a.title),
            duration: track.duration,
            quality: track
                .audio_quality
                .map(|q| Self::parse_quality(&q)),
            cover_url: None,
        }
    }

    fn parse_mpd_manifest(mpd_xml: &str) -> Result<Vec<String>> {
        let mut segment_urls = Vec::new();
        let mut base_url: Option<String> = None;

        let re = Regex::new(r"<BaseURL>([^<]+)</BaseURL>").unwrap();
        if let Some(caps) = re.captures(mpd_xml) {
            base_url = Some(caps[1].to_string());
        }

        let segment_re = Regex::new(r#"media="([^"]+)""#).unwrap();
        for caps in segment_re.captures_iter(mpd_xml) {
            let segment = caps[1].to_string();
            if let Some(ref base) = base_url {
                let full_url = if base.ends_with('/') {
                    format!(
                        "{}{}",
                        base, segment
                    )
                } else {
                    format!(
                        "{}/{}",
                        base, segment
                    )
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

        Ok(segment_urls)
    }

    async fn make_request<T: for<'de> Deserialize<'de>>(&self, endpoint: &str) -> Result<T> {
        let url = format!(
            "{}{}",
            self.get_api_base(),
            endpoint
        );

        let response = self
            .client
            .get(&url)
            .send()
            .await?;

        if response.status() == 429 {
            return Err(DownloadError::RateLimited);
        }

        let status = response.status();
        if !status.is_success() {
            return Err(
                DownloadError::ServiceUnavailable(
                    format!(
                        "API returned status {}",
                        status
                    ),
                ),
            );
        }

        let text = response
            .text()
            .await?;

        serde_json::from_str(&text).map_err(|e| DownloadError::JsonParseError(e))
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
                    .map(|a| a.title),
                duration: response
                    .data
                    .duration,
                quality: response
                    .data
                    .audio_quality
                    .map(|q| Self::parse_quality(&q)),
                cover_url: None,
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
                cover_url: None,
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
            "/track/?id={}&quality={}",
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

        if manifest_str.contains("application/dash+xml") || manifest_str.contains("<MPD") {
            let segment_urls = Self::parse_mpd_manifest(&manifest_str)?;
            Ok(
                DownloadManifest {
                    url: None,
                    segment_urls: Some(segment_urls),
                    quality,
                    bit_depth,
                    sample_rate,
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
                    quality,
                    bit_depth,
                    sample_rate,
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
