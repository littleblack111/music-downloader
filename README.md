# Full disclosure, completely vibe coded, uses squid.wtf's downloading service, but it works :+1
# CSV is downloaded from https://www.tunemymusic.com/transfer/spotify-to-file

# Squid Downloader

A CLI tool for downloading high-quality music from multiple streaming services.

## Features

- **Multiple Services**: Tidal, Amazon Music, SoundCloud, KHInsider
- **High-Quality Audio**: Support for Hi-Res FLAC (24-bit/192kHz), CD-quality FLAC, and lossy formats
- **Artist Filtering**: Optionally provide artist name to improve search accuracy
- **CSV Import**: Import tracks from CSV files with smart fuzzy matching
- **Pretty Output**: Beautiful CLI output with colors, tables, and progress bars
- **Error Handling**: Clear, formatted error messages

## Installation

```bash
cargo build --release
```

The binary will be at `./target/release/squid-downloader`.

## Usage

### Search for tracks

```bash
# Basic search
squid-downloader search "Waiting For Love"

# Search with artist filter for better accuracy
squid-downloader search "Waiting For Love" -a "Avicii"

# Limit results
squid-downloader search "Bohemian Rhapsody" -n 5
```

### Download a track

```bash
# Download by search query (will prompt for selection)
squid-downloader track "Levels"

# Download with artist filter
squid-downloader track "Levels" -a "Avicii"

# Download first result without prompting
squid-downloader track "Levels" -a "Avicii" -f

# Download by track ID
squid-downloader track 48717877
```

### Import from CSV

```bash
# Import and download tracks from CSV
squid-downloader csv tracks.csv

# Set minimum confidence threshold (default: 0.6)
squid-downloader csv tracks.csv -t 0.8

# Skip confirmation prompt
squid-downloader csv tracks.csv -y
```

#### CSV Format

The CSV file should have the following headers:

```csv
Track name,Artist name,Album,Playlist name,Type,ISRC,Spotify - id
"Bleed","Connor Kauffman","Bleed","Favorite Songs","Favorite","QZPY62100085","4k8D363TGNjILbQRciQfBD"
"Running Up That Hill","Kate Bush","Hounds Of Love","Favorite Songs","Favorite","GBCNR8500002","1PtQJZVZIdWIYdARpZRDFO"
```

#### Matching Algorithm

The CSV import uses a smart matching algorithm:

1. Searches for each track with the artist name
2. If multiple artists are detected (separated by `,`, `&`, `/`, `;`), searches for each separately
3. Also searches for the track name alone
4. Scores candidates using:
   - Title similarity (45% weight)
   - Artist similarity (35% weight) 
   - Album similarity (20% weight)
5. Uses Jaro-Winkler distance for fuzzy matching
6. Only downloads tracks above the confidence threshold

### Download an album

```bash
squid-downloader album "Stories"
squid-downloader album 12345678  # by album ID
```

### Download a playlist

```bash
squid-downloader playlist 12345678
```

### Get track info

```bash
squid-downloader info 48717877
```

### List available services and qualities

```bash
squid-downloader list
```

## Global Options

| Option | Description |
|--------|-------------|
| `-o, --output` | Output directory for downloads (default: `./downloads`) |
| `-q, --quality` | Audio quality: `hires`, `lossless`, `high`, `low`, `mp3` |
| `-s, --service` | Streaming service: `tidal`, `amazon`, `soundcloud`, `khinsider` |
| `--quiet` | Suppress non-essential output |

## Audio Qualities

| Quality | Format | Description |
|---------|--------|-------------|
| `hires` | FLAC | 24-bit/192kHz (Hi-Res) |
| `lossless` | FLAC | 16-bit/44.1kHz (CD Quality) |
| `high` | AAC | 320kbps |
| `low` | AAC | 96kbps |
| `mp3` | MP3 | 320kbps |

## Examples

```bash
# Download a Hi-Res FLAC from Tidal
squid-downloader track "Wake Me Up" -a "Avicii" -q hires -f

# Download to custom directory
squid-downloader track "Levels" -o ~/Music/downloads

# Import playlist from CSV with high confidence threshold
squid-downloader csv my_playlist.csv -t 0.8 -y -q lossless
```

## License

MIT
