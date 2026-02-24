use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AudioQuality {
    #[serde(rename = "HI_RES_LOSSLESS")]
    HiResLossless,
    #[serde(rename = "LOSSLESS")]
    Lossless,
    #[serde(rename = "HIGH")]
    High,
    #[serde(rename = "LOW")]
    Low,
    #[serde(rename = "MP3_320")]
    Mp3_320,
    #[default]
    #[serde(rename = "AUTO")]
    Auto,
}

impl AudioQuality {
    pub fn as_str(&self) -> &'static str {
        match self {
            AudioQuality::HiResLossless => "HI_RES_LOSSLESS",
            AudioQuality::Lossless => "LOSSLESS",
            AudioQuality::High => "HIGH",
            AudioQuality::Low => "LOW",
            AudioQuality::Mp3_320 => "MP3_320",
            AudioQuality::Auto => "AUTO",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            AudioQuality::HiResLossless => "24-bit/192kHz FLAC (Hi-Res)",
            AudioQuality::Lossless => "16-bit/44.1kHz FLAC (CD Quality)",
            AudioQuality::High => "320kbps AAC",
            AudioQuality::Low => "96kbps AAC",
            AudioQuality::Mp3_320 => "320kbps MP3",
            AudioQuality::Auto => "Best available",
        }
    }

    pub fn file_extension(&self) -> &'static str {
        match self {
            AudioQuality::HiResLossless | AudioQuality::Lossless => "flac",
            AudioQuality::High | AudioQuality::Low => "m4a",
            AudioQuality::Mp3_320 => "mp3",
            AudioQuality::Auto => "flac",
        }
    }

    pub fn all() -> Vec<Self> {
        vec![
            AudioQuality::HiResLossless,
            AudioQuality::Lossless,
            AudioQuality::High,
            AudioQuality::Low,
        ]
    }
}

impl std::fmt::Display for AudioQuality {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            self.description()
        )
    }
}

impl std::str::FromStr for AudioQuality {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s
            .to_uppercase()
            .as_str()
        {
            "HI_RES_LOSSLESS" | "HIRES" | "HI-RES" | "MASTER" => Ok(AudioQuality::HiResLossless),
            "LOSSLESS" | "FLAC" | "CD" => Ok(AudioQuality::Lossless),
            "HIGH" | "AAC320" | "320" => Ok(AudioQuality::High),
            "LOW" | "AAC96" | "96" => Ok(AudioQuality::Low),
            "MP3_320" | "MP3" => Ok(AudioQuality::Mp3_320),
            "AUTO" => Ok(AudioQuality::Auto),
            _ => Err(
                format!(
                    "Unknown quality: {}",
                    s
                ),
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Service {
    Tidal,
    AmazonMusic,
    SoundCloud,
    KHInsider,
}

impl Service {
    pub fn as_str(&self) -> &'static str {
        match self {
            Service::Tidal => "tidal",
            Service::AmazonMusic => "amazon",
            Service::SoundCloud => "soundcloud",
            Service::KHInsider => "khinsider",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Service::Tidal => "Tidal",
            Service::AmazonMusic => "Amazon Music",
            Service::SoundCloud => "SoundCloud",
            Service::KHInsider => "KHInsider",
        }
    }

    pub fn all() -> Vec<Self> {
        vec![
            Service::Tidal,
            Service::AmazonMusic,
            Service::SoundCloud,
            Service::KHInsider,
        ]
    }
}

impl std::fmt::Display for Service {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            self.display_name()
        )
    }
}

impl std::str::FromStr for Service {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s
            .to_lowercase()
            .as_str()
        {
            "tidal" | "tdl" => Ok(Service::Tidal),
            "amazon" | "amazonmusic" | "amz" => Ok(Service::AmazonMusic),
            "soundcloud" | "sc" => Ok(Service::SoundCloud),
            "khinsider" | "khi" | "kh" => Ok(Service::KHInsider),
            _ => Err(
                format!(
                    "Unknown service: {}",
                    s
                ),
            ),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackInfo {
    pub id: u64,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub duration: Option<u32>,
    pub quality: Option<AudioQuality>,
    pub cover_url: Option<String>,
}

impl TrackInfo {
    pub fn format_duration(&self) -> String {
        self.duration
            .map(
                |d| {
                    let mins = d / 60;
                    let secs = d % 60;
                    format!(
                        "{:02}:{:02}",
                        mins, secs
                    )
                },
            )
            .unwrap_or_else(|| "--:--".to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlbumInfo {
    pub id: u64,
    pub title: String,
    pub artist: String,
    pub track_count: u32,
    pub year: Option<u32>,
    pub cover_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistInfo {
    pub id: u64,
    pub title: String,
    pub creator: String,
    pub track_count: u32,
    pub cover_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResults {
    pub tracks: Vec<TrackInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadManifest {
    pub url: Option<String>,
    pub segment_urls: Option<Vec<String>>,
    pub quality: AudioQuality,
    pub bit_depth: Option<u8>,
    pub sample_rate: Option<u32>,
}
