pub mod tidal;

use crate::{error::Result, types::*};
use async_trait::async_trait;

#[async_trait]
pub trait MusicService: Send + Sync {
    async fn search(&self, query: &str, artist: Option<&str>) -> Result<SearchResults>;

    async fn get_track_info(&self, track_id: u64) -> Result<TrackInfo>;

    async fn get_album_info(&self, album_id: u64) -> Result<AlbumInfo>;

    async fn get_playlist_info(&self, playlist_id: u64) -> Result<PlaylistInfo>;

    async fn get_album_tracks(&self, album_id: u64) -> Result<Vec<TrackInfo>>;

    async fn get_playlist_tracks(&self, playlist_id: u64) -> Result<Vec<TrackInfo>>;

    async fn get_manifest(&self, track_id: u64, quality: AudioQuality) -> Result<DownloadManifest>;
}
