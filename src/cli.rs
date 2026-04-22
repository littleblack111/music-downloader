use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "squid-downloader")]
#[command(author = "Squid Downloader")]
#[command(version = "0.1.0")]
#[command(about = "Download high-quality music from multiple streaming services", long_about = None)]
#[command(color = clap::ColorChoice::Always)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    #[arg(
        short,
        long,
        global = true,
        help = "Output directory for downloads"
    )]
    pub output: Option<PathBuf>,

    #[arg(short, long, global = true, help = "Audio quality", value_parser = ["hires", "lossless", "high", "low", "mp3"])]
    pub quality: Option<String>,

    #[arg(short, long, global = true, help = "Streaming service", value_parser = ["tidal", "amazon", "soundcloud", "khinsider"])]
    pub service: Option<String>,

    #[arg(
        long,
        global = true,
        help = "Suppress non-essential output"
    )]
    pub quiet: bool,

    #[arg(
        short = 'j',
        long,
        global = true,
        help = "Max concurrent downloads (default: CPU cores)"
    )]
    pub concurrent: Option<usize>,

    #[arg(short = 'd', long, global = true, help = "Directory organization mode", value_parser = ["playlist", "album", "artist", "flat"], default_value = "playlist")]
    pub dir_mode: Option<String>,

    #[arg(
        short,
        long,
        global = true,
        help = "Embed cover art in downloaded files",
        default_value_t = true
    )]
    pub embed_cover: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    #[command(about = "Search for tracks")]
    Search {
        #[arg(help = "Search query (song title)")]
        query: String,

        #[arg(
            short,
            long,
            help = "Artist name to improve accuracy"
        )]
        artist: Option<String>,

        #[arg(
            short = 'n',
            long,
            default_value = "10",
            help = "Number of results"
        )]
        limit: usize,
    },

    #[command(about = "Download a single track")]
    Track {
        #[arg(help = "Track ID or search query")]
        input: String,

        #[arg(
            short,
            long,
            help = "Artist name to improve accuracy"
        )]
        artist: Option<String>,

        #[arg(
            short = 'f',
            long,
            help = "Download first result without prompting"
        )]
        first: bool,
    },

    #[command(about = "Download an entire album")]
    Album {
        #[arg(help = "Album ID or search query")]
        input: String,
    },

    #[command(about = "Download tracks from an artist")]
    Artist {
        #[arg(help = "Artist ID or search query")]
        input: String,
    },

    #[command(about = "Download a playlist")]
    Playlist {
        #[arg(help = "Playlist ID or URL")]
        input: String,
    },

    #[command(about = "Import and download tracks from CSV file")]
    Csv {
        #[arg(help = "Path to CSV file")]
        file: Option<String>,

        #[arg(
            short = 't',
            long,
            default_value = "0.6",
            help = "Minimum confidence threshold (0.0-1.0)"
        )]
        threshold: f64,

        #[arg(
            short = 'y',
            long,
            help = "Skip confirmation prompt"
        )]
        yes: bool,
    },

    #[command(about = "Show information about a track")]
    Info {
        #[arg(help = "Track ID")]
        id: u64,
    },

    #[command(about = "Get recommendations for a track")]
    Recommend {
        #[arg(help = "Track ID")]
        id: u64,
    },

    #[command(about = "List available qualities and services")]
    List,

    #[command(about = "Fix metadata and cover art for existing FLAC files")]
    Fix {
        #[arg(help = "Directory containing FLAC files")]
        directory: PathBuf,
        
        #[arg(
            short,
            long,
            default_value = "false",
            help = "Also fix filenames based on metadata"
        )]
        fix_filenames: bool,
    },
}

impl Cli {
    pub fn parse() -> Self {
        Parser::parse()
    }
}
